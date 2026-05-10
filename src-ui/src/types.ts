// Shared type definitions

export interface AppConfig {
  tencent_app_id: string;
  tencent_secret_id: string;
  tencent_secret_key: string;
  tencent_usage_seconds: number;
  osc_host: string;
  osc_port: number;
  osc_line_count: number;
  osc_retention_secs: number;
  osc_remove_period: boolean;
  osc_enabled: boolean;
  trigger_start: string;
  trigger_stop: string;
  asr_provider: string;
  local_stt_url: string;
  stt_config_path: string;
  global_hotkey_enabled: boolean;
  trigger_listener_enabled: boolean;
  trigger_stt_provider: string;
  asr_backend: string;
  onnx_provider: string;
  vad_enabled: boolean;
  vad_sentence_silence: number;
  vad_sub_phrase_silence: number;
  vad_min_utterance: number;
  keyboard_input_enabled: boolean;
  keyboard_input_mode: string;
  floating_window_enabled: boolean;
  vr_controller_enabled: boolean;
}

export interface AudioDevice {
  name: string;
  index: number;
}

export interface TestRecording {
  filename: string;
  path: string;
  size_bytes: number;
  created: string;
}

export interface LogEntry {
  timestamp: number;
  level: string;
  message: string;
  module: string;
}

export interface HistoryEntry {
  id: number;
  timestamp: string;
  text: string;
}

export interface SttModelStatus {
  exists: boolean;
  model_name: string;
  missing_files: string[];
  model_dir: string;
}

export interface DownloadProgress {
  phase: string;
  current: number;
  total: number;
}

export interface AvailableModel {
  name: string;
  display_name: string;
  size_bytes: number;
  backend: string;
  files: {
    encoder: string;
    decoder: string;
    joiner: string;
    tokens: string;
  };
}

export type ApiState = "idle" | "recording" | "recognizing" | "done" | "error";
