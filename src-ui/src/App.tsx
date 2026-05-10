import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { t, useI18n } from "./i18n";
import "./App.css";
import type {
  AppConfig,
  AudioDevice,
  TestRecording,
  LogEntry,
  HistoryEntry,
  SttModelStatus,
  DownloadProgress,
  AvailableModel,
  ApiState,
} from "./types";
import TestRecordingModal from "./components/TestRecordingModal";
import ConfigModal from "./components/ConfigModal";
import LogPanel from "./components/LogPanel";

function App() {
  const { t, lang, toggleLang } = useI18n();
  // Recording state
  const [apiState, setApiState] = useState<ApiState>("idle");
  const [lastResult, setLastResult] = useState("");
  const [currentPartial, setCurrentPartial] = useState("");
  const [lastError, setLastError] = useState("");
  const [currentVolume, setCurrentVolume] = useState(0);

  // Test recording state
  const [testRecordings, setTestRecordings] = useState<TestRecording[]>([]);
  const [isTestRecording, setIsTestRecording] = useState(false);
  const [showTestModal, setShowTestModal] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);

  // Log panel state
  const [showLogs, setShowLogs] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [logFilter, setLogFilter] = useState<string>("all");

  // Recognition history
  const [recognitionHistory, setRecognitionHistory] = useState<HistoryEntry[]>([]);
  const [historyPage, setHistoryPage] = useState(1);
  const [historyPageSize] = useState(10);
  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const historyListRef = useRef<HTMLDivElement>(null);

  // Audio devices
  const [audioDevices, setAudioDevices] = useState<AudioDevice[]>([]);
  const [selectedDeviceIndex, setSelectedDeviceIndex] = useState(0);

  // Trigger listener echo
  const [triggerHeardText, setTriggerHeardText] = useState("");

  // STT connection status
  const [sttStatus, setSttStatus] = useState<string>("starting");

  // Config
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [config, setConfig] = useState<AppConfig>({
    tencent_credentials_file: "tencent_credentials.yaml",
    osc_host: "127.0.0.1",
    osc_port: 9000,
    osc_line_count: 2,
    osc_retention_secs: 5,
    osc_remove_period: true,
    osc_enabled: true,
    trigger_start: "开始语音识别",
    trigger_stop: "结束语音识别",
    asr_provider: "tencent",
    local_stt_url: "ws://192.168.101.7:8765",
    stt_config_path: "stt-config.yaml",
    global_hotkey_enabled: true,
    trigger_listener_enabled: false,
    trigger_stt_provider: "local",
    asr_backend: "sherpa-onnx",
  });

  // Credential form state
  const [credsExist, setCredsExist] = useState(false);
  const [credAppId, setCredAppId] = useState("");
  const [credSecretId, setCredSecretId] = useState("");
  const [credSecretKey, setCredSecretKey] = useState("");
  const [credSaveMsg, setCredSaveMsg] = useState("");

  // STT model status
  const [modelStatus, setModelStatus] = useState<SttModelStatus | null>(null);
  const [isDownloading, setIsDownloading] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState<DownloadProgress>({ phase: "", current: 0, total: 0 });
  const [downloadError, setDownloadError] = useState("");
  const [modelCheckError, setModelCheckError] = useState("");
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([]);
  const [currentModelName, setCurrentModelName] = useState("");

  // Load config on mount
  useEffect(() => {
    invoke<AppConfig>("get_config")
      .then((cfg) => {
        setConfig(cfg);
      })
      .catch((e) => {
        console.error("Failed to load config:", e);
      });

    // Load available STT models
    invoke<AvailableModel[]>("get_available_models")
      .then((models) => setAvailableModels(models))
      .catch(console.error);

    // Load audio devices
    invoke<AudioDevice[]>("list_audio_devices")
      .then((devices) => {
        setAudioDevices(devices);
      })
      .catch(console.error);

    // Load saved device index from DB
    invoke<number>("get_saved_device_index")
      .then((idx) => setSelectedDeviceIndex(idx))
      .catch(() => {});

    // Load recognition history
    loadHistory();
  }, []);

  // Check STT model status when local_embedded is selected
  useEffect(() => {
      if (config.asr_provider === "local_embedded"
       || config.trigger_stt_provider === "local_embedded") {
      invoke<SttModelStatus>("check_stt_model", { sttConfigPath: config.stt_config_path })
        .then((status) => {
          setModelStatus(status);
          setCurrentModelName(status.model_name);
          setModelCheckError("");
        })
        .catch((e) => {
          console.error("check_stt_model failed:", e);
          setModelStatus(null);
          setModelCheckError(String(e));
        });
    } else {
      setModelStatus(null);
    }
  }, [config.asr_provider, config.trigger_stt_provider, config.stt_config_path]);

  // Set up event listeners (recording + download)
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];

    listen<string>("recording-started", () => {
      setApiState("recording");
      setCurrentPartial("");
      setLastResult("");
      setLastError("");
      setTriggerHeardText("");
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-partial", (event) => {
      setApiState("recognizing");
      setCurrentPartial(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-sentence", () => {
      loadHistory();
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-complete", (event) => {
      setApiState("done");
      setLastResult(event.payload);
      setCurrentPartial("");
      // Refresh recordings list when a test recording completes
      invoke<TestRecording[]>("list_test_recordings").then(setTestRecordings).catch(console.error);
      loadHistory();
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-error", (event) => {
      setApiState("error");
      setLastError(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<number>("volume-update", (event) => {
      setCurrentVolume(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<string>("hotkey-toggle", () => {
      toggleRecordingRef.current();
    }).then((fn) => unlisteners.push(fn));

    listen<string>("trigger-heard", (event) => {
      setTriggerHeardText(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<string>("trigger-stt-status", (event) => {
      setSttStatus(event.payload);
    }).then((fn) => unlisteners.push(fn));

    return () => {
      unlisteners.forEach((fn) => fn());
    };
    }, []);

  // Load initial logs and subscribe to log-entry events
  useEffect(() => {
    invoke<LogEntry[]>("get_recent_logs").then(setLogs).catch(console.error);

    const logUnlisteners: UnlistenFn[] = [];
    const fn = listen<LogEntry>("log-entry", (event) => {
      setLogs((prev) => {
        const next = [...prev, event.payload];
        if (next.length > 200) next.shift();
        return next;
      });
    });
    fn.then((u) => logUnlisteners.push(u));

    return () => {
      logUnlisteners.forEach((fn) => fn());
    };
  }, []);

  // Load credentials when config modal opens
  useEffect(() => {
    if (showConfigModal) {
      invoke("get_tencent_credentials")
        .then(() => {
          setCredsExist(true);
          setCredSaveMsg("");
        })
        .catch(() => {
          setCredsExist(false);
          setCredSaveMsg("loadError");
        });
      setCredAppId("");
      setCredSecretId("");
      setCredSecretKey("");
    }
  }, [showConfigModal]);

  // Listen for STT model download progress events
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];

    listen<DownloadProgress>("stt-model-download-progress", (event) => {
      setDownloadProgress(event.payload);
      setIsDownloading(true);
    }).then((fn) => unlisteners.push(fn));

    listen<string>("stt-model-download-complete", () => {
      setIsDownloading(false);
      setDownloadError("");
      setDownloadProgress({ phase: "complete", current: 0, total: 0 });
      // Re-check model status after download
      invoke<SttModelStatus>("check_stt_model", { sttConfigPath: config.stt_config_path })
        .then((status) => {
          setModelStatus(status);
          setCurrentModelName(status.model_name);
          setModelCheckError("");
        })
        .catch(() => { setModelStatus(null); setModelCheckError("模型检查失败"); });
    }).then((fn) => unlisteners.push(fn));

    listen<string>("stt-model-download-error", (event) => {
      setIsDownloading(false);
      setDownloadError(event.payload);
      setDownloadProgress({ phase: "error", current: 0, total: 0 });
    }).then((fn) => unlisteners.push(fn));

    return () => { unlisteners.forEach((fn) => fn()); };
  }, [config.stt_config_path]);

  // Toggle recording
  const toggleRecording = useCallback(() => {
    if (apiState === "recording" || apiState === "recognizing") {
      invoke("stop_recording").catch(console.error);
      setApiState("idle");
    } else {
      invoke("start_recording", { deviceIndex: selectedDeviceIndex }).catch(
        (e) => {
          setApiState("error");
          setLastError(String(e));
        }
      );
    }
  }, [apiState, selectedDeviceIndex]);

  // Ref for hotkey listener to always have latest toggleRecording
  const toggleRecordingRef = useRef(toggleRecording);
  useEffect(() => {
    toggleRecordingRef.current = toggleRecording;
  }, [toggleRecording]);

  // Test recording functions
  const loadRecordings = useCallback(() => {
    invoke<TestRecording[]>("list_test_recordings").then(setTestRecordings).catch(console.error);
  }, []);

  const toggleTestRecording = useCallback(() => {
    if (isTestRecording) {
      invoke("stop_recording").catch(console.error);
      setIsTestRecording(false);
      setApiState("idle");
    } else {
      setApiState("recording");
      setLastResult("");
      setCurrentPartial("");
      invoke("start_test_recording", { deviceIndex: selectedDeviceIndex })
        .catch((e) => {
          setApiState("error");
          setLastError(String(e));
        });
      setIsTestRecording(true);
    }
  }, [isTestRecording, selectedDeviceIndex]);

  const deleteRecording = useCallback((filename: string) => {
    invoke("delete_test_recording", { filename })
      .then(() => loadRecordings())
      .catch(console.error);
  }, [loadRecordings]);

  const playRecording = useCallback((filepath: string) => {
    alert(t("test.savedAlert") + " " + filepath);
  }, []);

  const resetConfig = useCallback(async () => {
    try {
      const defaultConfig = await invoke<AppConfig>("reset_config");
      setConfig(defaultConfig);
    } catch (e) {
      console.error("Failed to reset config:", e);
    }
  }, []);

  // Save config on change
  const updateConfig = useCallback(
    (field: keyof AppConfig, value: string | number | boolean) => {
      const newConfig = { ...config, [field]: value };
      setConfig(newConfig);
      invoke("save_config", { config: newConfig }).catch(console.error);
    },
    [config]
  );

  // Recognition history
  const loadHistory = useCallback(() => {
    invoke<HistoryEntry[]>("get_recognition_history").then(entries => { setRecognitionHistory(entries); setHistoryPage(1); }).catch(console.error);
  }, []);

  const clearHistory = useCallback(() => {
    invoke("clear_recognition_history").then(() => setRecognitionHistory([])).catch(console.error);
  }, []);

  // Save Tencent Cloud credentials
  const saveCredentials = useCallback(async () => {
    try {
      await invoke("save_tencent_credentials", {
        appId: credAppId,
        secretId: credSecretId,
        secretKey: credSecretKey,
      });
      setCredsExist(true);
      setCredSaveMsg("saved");
      setCredAppId("");
      setCredSecretId("");
      setCredSecretKey("");
    } catch (e) {
      setCredSaveMsg("error");
      console.error("Failed to save credentials:", e);
    }
  }, [credAppId, credSecretId, credSecretKey]);

  // STT status helpers
  const getSttStatusClass = (status: string): string => {
    if (status === "disabled") return "disabled";
    if (status.startsWith("error")) return "error";
    if (status === "disconnected") return "disconnected";
    if (status === "connected") return "connected";
    if (status === "connecting") return "connecting";
    return "unknown";
  };

  const getSttStatusText = (status: string): string => {
    if (status === "disabled") return t("stt.disabled");
    if (status.startsWith("error")) return t("stt.error");
    if (status === "disconnected") return t("stt.disconnected");
    if (status === "connected") return t("stt.connected");
    if (status === "connecting") return t("stt.connecting");
    return "";
  };

  // Auto-scroll history list when new entries arrive
  useEffect(() => {
    if (historyListRef.current) {
      historyListRef.current.scrollTop = 0;
    }
  }, [recognitionHistory]);

  const isRecording = apiState === "recording" || apiState === "recognizing";

  return (
    <div className="app-container">
      <header>
        <h1>{t("app.title")}</h1>
        <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
          <button 
            onClick={toggleLang}
            style={{ padding: "0.2rem 0.5rem", background: "#444", color: "#ccc", border: "1px solid #555", borderRadius: "4px", cursor: "pointer", fontSize: "0.8rem" }}
            title="Switch language / 切换语言"
          >
            {lang === "zh" ? "EN" : "中文"}
          </button>
          <span className={`status-badge status-${apiState}`}>
            {apiState === "idle" && t("status.ready")}
            {apiState === "recording" && t("status.recording")}
            {apiState === "recognizing" && t("status.recognizing")}
            {apiState === "done" && t("status.done")}
            {apiState === "error" && t("status.error")}
          </span>
        </div>
      </header>

      {/* Control Panel */}
      <section className="control-panel">
        <div className="device-selector">
          <label>{t("control.audioDevice")}</label>
          <select
            value={selectedDeviceIndex}
            onChange={(e) => {
              const idx = Number(e.target.value);
              setSelectedDeviceIndex(idx);
              invoke("save_device_index", { deviceIdx: idx }).catch(() => {});
            }}
          >
            {audioDevices.map((d) => (
              <option key={d.index} value={d.index}>
                {d.name}
              </option>
            ))}
          </select>
        </div>

        <div className="volume-meter">
          <div className="volume-bar">
            <div
              className="volume-fill"
              style={{ width: `${(currentVolume * 100).toFixed(1)}%` }}
            />
          </div>
          <span className="volume-label">
            {t("control.volume")} {(currentVolume * 100).toFixed(0)}%
          </span>
        </div>

        {triggerHeardText && !isRecording && (
          <div className="trigger-heard">
            <span className="trigger-heard-text">{triggerHeardText}</span>
          </div>
        )}

        <div className="action-buttons">
          <button className="test-modal-trigger" onClick={() => setShowTestModal(true)}>
            {t("test.title")}
          </button>
          <button className="test-modal-trigger" onClick={() => setShowConfigModal(true)}>
            {t("config.title")}
          </button>
        </div>

        {config.trigger_listener_enabled && (
          <div className="stt-status">
            <span className={`stt-dot stt-${getSttStatusClass(sttStatus)}`} />
            <span className="stt-text">{getSttStatusText(sttStatus)}</span>
          </div>
        )}

      </section>

      {/* Record Button (full width) */}
      <button
        className={`record-button ${isRecording ? "recording" : ""}`}
        onClick={toggleRecording}
        style={{ width: "100%", marginTop: "0.5rem" }}
      >
        {isRecording ? t("control.stop") : t("control.startRecording")}
      </button>

      {/* Results Display */}
      <section className="results-panel">
        {currentPartial && (
          <div className="partial-result">
            <span className="label">{t("results.listening")}</span>
            <span className="text">{currentPartial}</span>
          </div>
        )}
        {lastError && (
          <div className="error-result">
            <span className="label">{t("results.error")}</span>
            <span className="text">{lastError}</span>
          </div>
        )}
      </section>

      {/* Recognition History */}
      <section className="history-panel">
        <h2>{t("history.title")}</h2>
        {recognitionHistory.length === 0 ? (
          <p className="history-empty">{t("history.empty")}</p>
        ) : (
          <>
            <div className="history-list" ref={historyListRef}>
              {recognitionHistory
                .slice((historyPage - 1) * historyPageSize, historyPage * historyPageSize)
                .map((entry) => (
                  <div key={entry.id} className="history-entry">
                    <span className="history-time">{entry.timestamp}</span>
                    <span className="history-text">{entry.text}</span>
                </div>
                ))}
            </div>
            <div className="history-pagination">
              <button onClick={() => setHistoryPage(p => Math.max(1, p - 1))} disabled={historyPage <= 1}>‹</button>
              <span className="page-info">{historyPage} / {Math.ceil(recognitionHistory.length / historyPageSize)}</span>
              <button onClick={() => setHistoryPage(p => p + 1)} disabled={historyPage >= Math.ceil(recognitionHistory.length / historyPageSize)}>›</button>
              <input type="number" className="page-jump" min="1" max={Math.ceil(recognitionHistory.length / historyPageSize)}
                placeholder="页" onKeyDown={(e) => { if (e.key === 'Enter') { const v = parseInt((e.target as HTMLInputElement).value); if (v >= 1) setHistoryPage(v); } }} />
            </div>
          </>
        )}
        <div className="history-actions">
          {!showClearConfirm ? (
            <button className="history-clear" onClick={() => setShowClearConfirm(true)}>{t("history.clear")}</button>
          ) : (
            <span className="clear-confirm">
              <span>{t("history.confirmClear")}</span>
              <button className="confirm-yes" onClick={() => { clearHistory(); setShowClearConfirm(false); }}>{t("history.yes")}</button>
              <button className="confirm-no" onClick={() => setShowClearConfirm(false)}>{t("history.no")}</button>
            </span>
          )}
        </div>
      </section>

      {/* Test Recording Modal */}
      <TestRecordingModal
        show={showTestModal}
        onClose={() => setShowTestModal(false)}
        isTestRecording={isTestRecording}
        toggleTestRecording={toggleTestRecording}
        testRecordings={testRecordings}
        loadRecordings={loadRecordings}
        playRecording={playRecording}
        deleteRecording={deleteRecording}
      />

      {/* Config Modal */}
      <ConfigModal
        show={showConfigModal}
        onClose={() => setShowConfigModal(false)}
        config={config}
        updateConfig={updateConfig}
        credsExist={credsExist}
        credAppId={credAppId}
        credSecretId={credSecretId}
        credSecretKey={credSecretKey}
        credSaveMsg={credSaveMsg}
        setCredAppId={setCredAppId}
        setCredSecretId={setCredSecretId}
        setCredSecretKey={setCredSecretKey}
        saveCredentials={saveCredentials}
        modelStatus={modelStatus}
        isDownloading={isDownloading}
        downloadProgress={downloadProgress}
        downloadError={downloadError}
        modelCheckError={modelCheckError}
        availableModels={availableModels}
        currentModelName={currentModelName}
        setCurrentModelName={setCurrentModelName}
        setModelStatus={setModelStatus}
        setDownloadError={setDownloadError}
        setIsDownloading={setIsDownloading}
        setDownloadProgress={setDownloadProgress}
        showResetConfirm={showResetConfirm}
        setShowResetConfirm={setShowResetConfirm}
        resetConfig={resetConfig}
      />

      {/* Log Panel */}
      <LogPanel
        show={showLogs}
        onToggle={() => setShowLogs(!showLogs)}
        logs={logs}
        logFilter={logFilter}
        setLogFilter={setLogFilter}
        onClear={() => { invoke("clear_logs"); setLogs([]); }}
      />
    </div>
  );
}

export default App;
