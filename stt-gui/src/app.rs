//! Main GUI application — layout, state management, queue polling.

use std::path::PathBuf;
use std::sync::mpsc;

use egui::{Color32, Frame, Margin, RichText, ScrollArea, Ui};

use crate::audio;
use crate::worker::{StreamWorker, WorkerEvent};

/// Persistent configuration saved between sessions.
#[derive(serde::Serialize, serde::Deserialize)]
struct GuiConfig {
    server_url: String,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            server_url: "ws://192.168.101.7:8765".to_string(),
        }
    }
}

/// Recording state.
#[derive(PartialEq)]
enum RecState {
    Idle,
    Recording,
}

pub struct SttGuiApp {
    // Config
    server_url: String,

    // State
    state: RecState,
    source_mode: AudioSource,
    file_path: PathBuf,
    selected_device: String,
    devices: Vec<String>,

    // Results
    partial_text: String,
    segments: Vec<(u32, String)>,

    // Worker
    worker: Option<StreamWorker>,
    event_rx: Option<mpsc::Receiver<WorkerEvent>>,
    log_entries: Vec<LogEntry>,
    show_logs: bool,

    // Volume
    volume: f32,

    // Config persistence
    config_path: PathBuf,
}

#[derive(Clone, Copy, PartialEq)]
enum AudioSource {
    Mic,
    File,
}

struct LogEntry {
    timestamp: String,
    level: LogLevel,
    message: String,
}

enum LogLevel {
    Info,
    Warn,
    Error,
}

impl SttGuiApp {
    fn start(&mut self) {
        self.stop(); // ensure cleanup

        // Save config
        if let Ok(json) = serde_json::to_string_pretty(&GuiConfig {
            server_url: self.server_url.clone(),
        }) {
            let _ = std::fs::write(&self.config_path, json);
        }

        self.partial_text.clear();
        self.segments.clear();
        self.log_entries.clear();

        let (event_tx, event_rx) = mpsc::channel();
        let mut worker = StreamWorker::new(self.server_url.clone(), event_tx);

        match self.source_mode {
            AudioSource::Mic => {
                worker.start_mic();
                self.add_log(LogLevel::Info, "Started microphone recording...");
            }
            AudioSource::File => {
                if self.file_path.as_os_str().is_empty() {
                    self.add_log(LogLevel::Warn, "Please select a WAV file first");
                    return;
                }
                worker.start_file(&self.file_path);
                self.add_log(
                    LogLevel::Info,
                    &format!(
                        "Sending file: {}",
                        self.file_path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                );
            }
        }

        self.worker = Some(worker);
        self.event_rx = Some(event_rx);
        self.state = RecState::Recording;
    }

    fn stop(&mut self) {
        if let Some(ref mut worker) = self.worker {
            worker.stop();
        }
        self.worker = None;
        self.event_rx = None;
        self.state = RecState::Idle;
        self.volume = 0.0;
        self.add_log(LogLevel::Info, "Stopped");
    }

    fn pick_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV files", &["wav"])
            .pick_file()
        {
            self.file_path = path;
        }
    }

    fn add_log(&mut self, level: LogLevel, msg: &str) {
        use chrono::Local;
        let ts = Local::now().format("%H:%M:%S").to_string();
        self.log_entries.push(LogEntry {
            timestamp: ts,
            level,
            message: msg.to_string(),
        });
        if self.log_entries.len() > 500 {
            self.log_entries = self.log_entries.split_off(200);
        }
    }
}

impl Default for SttGuiApp {
    fn default() -> Self {
        let config_path = std::env::current_exe()
            .unwrap_or_default()
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("stt_gui_config.json");

        let cfg: GuiConfig = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let devices = audio::list_input_devices();

        Self {
            server_url: cfg.server_url,
            state: RecState::Idle,
            source_mode: AudioSource::Mic,
            file_path: PathBuf::new(),
            selected_device: devices.first().cloned().unwrap_or_default(),
            devices,
            partial_text: String::new(),
            segments: Vec::new(),
            worker: None,
            event_rx: None,
            log_entries: Vec::new(),
            show_logs: false,
            volume: 0.0,
            config_path,
        }
    }
}

impl eframe::App for SttGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll worker events
        self.poll_events();

        // Apply dark theme
        ctx.set_visuals(egui::Visuals::dark());

        egui::CentralPanel::default().show(ctx, |ui| {
            self.header(ui);
            ui.separator();
            self.server_panel(ui);
            self.source_panel(ui);
            self.control_panel(ui);
            self.results_panel(ui);
            self.log_panel(ui);
        });

        // Periodic repaint for queue polling
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }
}

impl SttGuiApp {
    fn poll_events(&mut self) {
        // Drain events into a Vec first to avoid borrow conflicts
        let events: Vec<WorkerEvent> = if let Some(ref rx) = self.event_rx {
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            events
        } else {
            return;
        };

        for event in events {
            match event {
                WorkerEvent::Connected => {
                    self.add_log(LogLevel::Info, "Connected to server");
                }
                WorkerEvent::Disconnected => {
                    if self.state == RecState::Recording {
                        self.stop();
                    }
                }
                WorkerEvent::Error(msg) => {
                    self.add_log(LogLevel::Error, &msg);
                    if self.state == RecState::Recording {
                        self.stop();
                    }
                }
                WorkerEvent::Partial(text) => {
                    self.partial_text = text;
                }
                WorkerEvent::Final { text, segment } => {
                    self.segments.push((segment, text.clone()));
                    self.add_log(
                        LogLevel::Info,
                        &format!("Segment {} (final): {}", segment, text),
                    );
                    self.partial_text.clear();
                }
                WorkerEvent::Volume(v) => {
                    self.volume = v;
                }
                WorkerEvent::Log { level, message } => {
                    let level = match level.as_str() {
                        "warn" => LogLevel::Warn,
                        "error" => LogLevel::Error,
                        _ => LogLevel::Info,
                    };
                    self.add_log(level, &message);
                }
            }
        }
    }

    fn header(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("STT Service Test Tool")
                    .size(20.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, color) = match self.state {
                    RecState::Idle => ("Ready", Color32::GRAY),
                    RecState::Recording => ("Recording", Color32::RED),
                };
                ui.label(RichText::new(label).color(color).size(13.0));
            });
        });
    }

    fn server_panel(&mut self, ui: &mut Ui) {
        Frame::NONE
            .fill(Color32::from_gray(20))
            .inner_margin(Margin::same(12))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.label(RichText::new("Server URL").size(14.0).strong());
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_url)
                        .hint_text("ws://192.168.101.7:8765")
                        .desired_width(f32::INFINITY),
                );
            });
    }

    fn source_panel(&mut self, ui: &mut Ui) {
        Frame::NONE
            .fill(Color32::from_gray(20))
            .inner_margin(Margin::same(12))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.label(RichText::new("Audio Source").size(14.0).strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.source_mode, AudioSource::Mic, "Microphone");
                    ui.selectable_value(&mut self.source_mode, AudioSource::File, "WAV File");
                });

                match self.source_mode {
                    AudioSource::Mic => {
                        ui.horizontal(|ui| {
                            ui.label("Device:");
                            egui::ComboBox::from_id_salt("mic_device")
                                .selected_text(&self.selected_device)
                                .show_ui(ui, |ui| {
                                    for dev in &self.devices {
                                        ui.selectable_value(
                                            &mut self.selected_device,
                                            dev.clone(),
                                            dev,
                                        );
                                    }
                                });
                        });
                    }
                    AudioSource::File => {
                        ui.horizontal(|ui| {
                            if self.file_path.as_os_str().is_empty() {
                                ui.label("No file selected");
                            } else {
                                ui.label(
                                    self.file_path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy(),
                                );
                            }
                            if ui.button("Browse...").clicked() {
                                self.pick_file();
                            }
                        });
                    }
                }
            });
    }

    fn control_panel(&mut self, ui: &mut Ui) {
        Frame::NONE
            .fill(Color32::from_gray(20))
            .inner_margin(Margin::same(12))
            .corner_radius(8)
            .show(ui, |ui| {
                // Volume bar
                ui.horizontal(|ui| {
                    ui.add(
                        egui::ProgressBar::new(self.volume)
                            .desired_width(160.0)
                            .animate(true),
                    );
                    ui.label(format!("Volume {}%", (self.volume * 100.0) as i32));
                });

                ui.add_space(8.0);

                // Action button
                let (label, color) = match self.state {
                    RecState::Idle => {
                        let label = match self.source_mode {
                            AudioSource::Mic => "Start Recording",
                            AudioSource::File => "Send File",
                        };
                        (label, Color32::from_rgb(33, 150, 243)) // blue
                    }
                    RecState::Recording => ("Stop", Color32::from_rgb(244, 67, 54)), // red
                };

                if ui
                    .add_sized([180.0, 36.0], egui::Button::new(RichText::new(label).size(14.0).strong()).fill(color))
                    .clicked()
                {
                    match self.state {
                        RecState::Idle => self.start(),
                        RecState::Recording => self.stop(),
                    }
                }
            });
    }

    fn results_panel(&mut self, ui: &mut Ui) {
        Frame::NONE
            .fill(Color32::from_gray(20))
            .inner_margin(Margin::same(12))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.label(RichText::new("Recognition Results").size(14.0).strong());
                ui.add_space(4.0);

                // Partial result
                Frame::NONE
                    .fill(Color32::from_rgb(13, 29, 21))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(46, 125, 50)))
                    .inner_margin(Margin::symmetric(10, 6))
                    .corner_radius(6)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Live").color(Color32::from_rgb(102, 187, 106)).small());
                        ui.label(
                            RichText::new(if self.partial_text.is_empty() {
                                "(waiting...)"
                            } else {
                                &self.partial_text
                            })
                            .size(13.0)
                            .color(Color32::from_rgb(165, 214, 167)),
                        );
                    });

                ui.add_space(4.0);

                // Final results list
                ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (seg_id, text) in self.segments.iter().rev() {
                            Frame::NONE
                                .fill(Color32::from_rgb(13, 21, 29))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(21, 101, 192)))
                                .inner_margin(Margin::symmetric(10, 6))
                                .corner_radius(6)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("[{}]", seg_id))
                                            .color(Color32::from_rgb(100, 181, 246))
                                            .small(),
                                    );
                                    ui.label(
                                        RichText::new(text)
                                            .size(14.0)
                                            .color(Color32::from_rgb(187, 222, 251)),
                                    );
                                });
                            ui.add_space(2.0);
                        }
                    });
            });
    }

    fn log_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);

        let count = self.log_entries.len();
        let toggle_label = if self.show_logs {
            format!("Hide Log ({})", count)
        } else {
            format!("Show Log ({})", count)
        };

        if ui.button(toggle_label).clicked() {
            self.show_logs = !self.show_logs;
        }

        if self.show_logs {
            Frame::NONE
                .fill(Color32::from_gray(15))
                .stroke(egui::Stroke::new(1.0, Color32::from_gray(68)))
                .inner_margin(Margin::same(4))
                .corner_radius(4)
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .max_height(160.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for entry in self.log_entries.iter().rev().take(100) {
                                let color = match entry.level {
                                    LogLevel::Info => Color32::from_gray(200),
                                    LogLevel::Warn => Color32::from_rgb(255, 152, 0),
                                    LogLevel::Error => Color32::from_rgb(244, 67, 54),
                                };
                                ui.label(
                                    RichText::new(format!(
                                        "{} {:6} {}",
                                        entry.timestamp,
                                        match entry.level {
                                            LogLevel::Info => "[INFO]",
                                            LogLevel::Warn => "[WARN]",
                                            LogLevel::Error => "[ERR]",
                                        },
                                        entry.message
                                    ))
                                    .monospace()
                                    .size(11.0)
                                    .color(color),
                                );
                            }
                        });
                });
        }
    }
}
