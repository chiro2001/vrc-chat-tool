import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { t, useI18n } from "./i18n";
import "./App.css";

// Type definitions
interface AppConfig {
  tencent_app_id: string;
  tencent_secret_id: string;
  tencent_secret_key: string;
  osc_host: string;
  osc_port: number;
  osc_line_count: number;
  osc_retention_secs: number;
  osc_remove_period: boolean;
  asr_provider: string;
  local_stt_url: string;
}

interface AudioDevice {
  name: string;
  index: number;
}

type ApiState = "idle" | "recording" | "recognizing" | "done" | "error";

function App() {
  const { t, lang, toggleLang } = useI18n();
  // Recording state
  const [apiState, setApiState] = useState<ApiState>("idle");
  const [lastResult, setLastResult] = useState("");
  const [currentPartial, setCurrentPartial] = useState("");
  const [lastError, setLastError] = useState("");
  const [currentVolume, setCurrentVolume] = useState(0);

  // Test recording state
  const [testRecordings, setTestRecordings] = useState<any[]>([]);
  const [isTestRecording, setIsTestRecording] = useState(false);
  const [showTestModal, setShowTestModal] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);

  // Log panel state
  const [showLogs, setShowLogs] = useState(false);
  const [logs, setLogs] = useState<any[]>([]);
  const [logFilter, setLogFilter] = useState<string>("all");

  // Recognition history
  const [recognitionHistory, setRecognitionHistory] = useState<any[]>([]);

  // Audio devices
  const [audioDevices, setAudioDevices] = useState<AudioDevice[]>([]);
  const [selectedDeviceIndex, setSelectedDeviceIndex] = useState(0);

  // Config
  const [config, setConfig] = useState<AppConfig>({
    tencent_app_id: "",
    tencent_secret_id: "",
    tencent_secret_key: "",
    osc_host: "127.0.0.1",
    osc_port: 9000,
    osc_line_count: 2,
    osc_retention_secs: 5,
    osc_remove_period: true,
    asr_provider: "tencent",
    local_stt_url: "ws://192.168.101.7:8765",
  });

  // Load config on mount
  useEffect(() => {
    invoke<AppConfig>("get_config")
      .then((cfg) => {
        setConfig(cfg);
      })
      .catch((e) => {
        console.error("Failed to load config:", e);
      });

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

  // Set up event listeners
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];

    listen<string>("recording-started", () => {
      setApiState("recording");
      setCurrentPartial("");
      setLastResult("");
      setLastError("");
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-partial", (event) => {
      setApiState("recognizing");
      setCurrentPartial(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-complete", (event) => {
      setApiState("done");
      setLastResult(event.payload);
      setCurrentPartial("");
      // Refresh recordings list when a test recording completes
      invoke<any[]>("list_test_recordings").then(setTestRecordings).catch(console.error);
      loadHistory();
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-error", (event) => {
      setApiState("error");
      setLastError(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<number>("volume-update", (event) => {
      setCurrentVolume(event.payload);
    }).then((fn) => unlisteners.push(fn));

    return () => {
      unlisteners.forEach((fn) => fn());
    };
    }, []);

  // Load initial logs and subscribe to log-entry events
  useEffect(() => {
    invoke<any[]>("get_recent_logs").then(setLogs).catch(console.error);

    const logUnlisteners: UnlistenFn[] = [];
    const fn = listen<any>("log-entry", (event) => {
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

  // Test recording functions
  const loadRecordings = useCallback(() => {
    invoke<any[]>("list_test_recordings").then(setTestRecordings).catch(console.error);
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
    invoke<any[]>("get_recognition_history").then(setRecognitionHistory).catch(console.error);
  }, []);

  const clearHistory = useCallback(() => {
    invoke("clear_recognition_history").then(() => setRecognitionHistory([])).catch(console.error);
  }, []);

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

        <button className="test-modal-trigger" onClick={() => setShowTestModal(true)}>
          {t("test.title")}
        </button>
        <button className="test-modal-trigger" onClick={() => setShowConfigModal(true)}>
          {t("config.title")}
        </button>

        <button
          className={`record-button ${isRecording ? "recording" : ""}`}
          onClick={toggleRecording}
        >
          {isRecording ? t("control.stop") : t("control.startRecording")}
        </button>
      </section>

      {/* Results Display */}
      <section className="results-panel">
        {currentPartial && (
          <div className="partial-result">
            <span className="label">{t("results.listening")}</span>
            <span className="text">{currentPartial}</span>
          </div>
        )}
        {lastResult && (
          <div className="final-result">
            <span className="label">{t("results.result")}</span>
            <span className="text">{lastResult}</span>
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
          <div className="history-list">
            {recognitionHistory.map((entry: any) => (
              <div key={entry.id} className="history-entry">
                <span className="history-time">{entry.timestamp}</span>
                <span className="history-text">{entry.text}</span>
                <span className="history-source">{entry.source}</span>
              </div>
            ))}
          </div>
        )}
        <button className="history-clear" onClick={clearHistory}>{t("history.clear")}</button>
      </section>

      {/* Test Recording Modal */}
      {showTestModal && (
        <div className="modal-overlay" onClick={() => setShowTestModal(false)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t("test.title")}</h2>
              <button className="modal-close" onClick={() => setShowTestModal(false)}>X</button>
            </div>
            <div className="modal-body">
              <div className="test-recording-controls">
                <button
                  className={`record-button ${isTestRecording ? "recording" : ""}`}
                  onClick={toggleTestRecording}
                >
                  {isTestRecording ? t("test.stop") : t("test.start")}
                </button>
                <button className="refresh-button" onClick={loadRecordings}>
                  {t("test.refresh")}
                </button>
              </div>

              {testRecordings.length > 0 && (
                <div className="recordings-list">
                  {testRecordings.map((rec: any) => (
                    <div key={rec.filename} className="recording-item">
                      <span className="rec-filename" title={rec.path}>{rec.filename}</span>
                      <span className="rec-size">({(rec.size_bytes / 1024).toFixed(1)} KB)</span>
                      <button
                        className="play-button"
                        onClick={() => playRecording(rec.path)}
                      >
                        {t("test.play")}
                      </button>
                      <button
                        className="delete-button"
                        onClick={() => deleteRecording(rec.filename)}
                      >
                        {t("test.delete")}
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Config Modal */}
      {showConfigModal && (
        <div className="modal-overlay" onClick={() => setShowConfigModal(false)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t("config.title")}</h2>
              <button className="modal-close" onClick={() => setShowConfigModal(false)}>X</button>
            </div>
            <div className="modal-body">
              <div className="config-field">
                <label>{t("config.provider")}</label>
                <select value={config.asr_provider} onChange={(e) => updateConfig("asr_provider", e.target.value)}>
                  <option value="tencent">{t("config.providerTencent")}</option>
                  <option value="local">{t("config.providerLocal")}</option>
                </select>
              </div>

              {config.asr_provider === "local" ? (
                <div className="config-field">
                  <label>{t("config.localSttUrl")}</label>
                  <input type="text" value={config.local_stt_url} onChange={(e) => updateConfig("local_stt_url", e.target.value)} />
                </div>
              ) : (
                <>
                  <h3>{t("config.tencent")}</h3>
              <div className="config-field">
                <label>{t("config.appId")}</label>
                <input type="text" value={config.tencent_app_id} onChange={(e) => updateConfig("tencent_app_id", e.target.value)} placeholder="REDACTED_APPID" />
              </div>
              <div className="config-field">
                <label>{t("config.secretId")}</label>
                <input type="text" value={config.tencent_secret_id} onChange={(e) => updateConfig("tencent_secret_id", e.target.value)} />
              </div>
              <div className="config-field">
                <label>{t("config.secretKey")}</label>
                <input type="password" value={config.tencent_secret_key} onChange={(e) => updateConfig("tencent_secret_key", e.target.value)} />
              </div>
                </>
              )}
              <h3>{t("config.osc")}</h3>
              <div className="config-field">
                <label>{t("config.host")}</label>
                <input type="text" value={config.osc_host} onChange={(e) => updateConfig("osc_host", e.target.value)} />
              </div>
              <div className="config-field">
                <label>{t("config.port")}</label>
                <input type="number" value={config.osc_port} onChange={(e) => updateConfig("osc_port", Number(e.target.value))} />
              </div>
              <div className="config-field">
                <label>{t("config.lineCount")}</label>
                <input type="number" value={config.osc_line_count} min="1" max="10" onChange={(e) => updateConfig("osc_line_count", Number(e.target.value))} />
              </div>
              <div className="config-field">
                <label>{t("config.retentionSecs")}</label>
                <input type="number" value={config.osc_retention_secs} min="1" max="60" onChange={(e) => updateConfig("osc_retention_secs", Number(e.target.value))} />
              </div>
              <div className="config-field">
                <label>
                  <input type="checkbox" checked={config.osc_remove_period} onChange={(e) => updateConfig("osc_remove_period", e.target.checked)} />
                  {t("config.removePeriod")}
                </label>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Log Panel */}
      <div className="log-panel-wrapper">
        <button
          className="log-toggle-button"
          onClick={() => setShowLogs(!showLogs)}
        >
          {showLogs ? t("log.hide") : t("log.show")} ({logs.length})
        </button>

        {showLogs && (
          <section className="log-panel">
            <div className="log-controls">
              <select
                value={logFilter}
                onChange={(e) => setLogFilter(e.target.value)}
              >
                <option value="all">{t("log.allLevels")}</option>
                <option value="debug">{t("log.debug")}</option>
                <option value="info">{t("log.info")}</option>
                <option value="warn">{t("log.warn")}</option>
                <option value="error">{t("log.error")}</option>
              </select>
              <button
                className="clear-button"
                onClick={() => { invoke("clear_logs"); setLogs([]); }}
              >
                {t("log.clear")}
              </button>
            </div>

            <div className="log-entries">
              {logs
                .filter((l: any) => logFilter === "all" || l.level === logFilter)
                .slice(-100)
                .map((log: any, i: number) => (
                  <div key={i} className={`log-entry log-${log.level}`}>
                    <span className="log-time">
                      {new Date(log.timestamp).toLocaleTimeString()}
                    </span>
                    <span className={`log-level log-level-${log.level}`}>
                      [{log.level.toUpperCase()}]
                    </span>
                    <span className="log-module">[{log.module}]</span>
                    <span className="log-message">{log.message}</span>
                  </div>
                ))}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}

export default App;
