import { useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { t } from "../i18n";
import type {
  AppConfig,
  SttModelStatus,
  DownloadProgress,
  AvailableModel,
} from "../types";

interface Props {
  show: boolean;
  onClose: () => void;
  config: AppConfig;
  updateConfig: (field: keyof AppConfig, value: string | number | boolean) => void;
  // STT model
  modelStatus: SttModelStatus | null;
  isDownloading: boolean;
  downloadProgress: DownloadProgress;
  downloadError: string;
  modelCheckError: string;
  availableModels: AvailableModel[];
  currentModelName: string;
  setCurrentModelName: (v: string) => void;
  setModelStatus: (v: SttModelStatus | null) => void;
  setDownloadError: (v: string) => void;
  setIsDownloading: (v: boolean) => void;
  setDownloadProgress: (v: DownloadProgress) => void;
  // Reset
  showResetConfirm: boolean;
  setShowResetConfirm: (v: boolean) => void;
  resetConfig: () => void;
}

const TABS = [
  { key: "tabSpeech", labelKey: "config.tabSpeech" },
  { key: "tabVad", labelKey: "config.tabVad" },
  { key: "tabMaintenance", labelKey: "config.tabMaintenance" },
] as const;

export default function SettingsModal({
  show,
  onClose,
  config,
  updateConfig,

  modelStatus,
  isDownloading,
  downloadProgress,
  downloadError,
  modelCheckError,
  availableModels,
  currentModelName,
  setCurrentModelName,
  setModelStatus,
  setDownloadError,
  setIsDownloading,
  setDownloadProgress,
  showResetConfirm,
  setShowResetConfirm,
  resetConfig,
}: Props) {
  const [activeTab, setActiveTab] = useState(0);
  const [showDeleteModelsConfirm, setShowDeleteModelsConfirm] = useState(false);
  const [showDeleteAllConfirm, setShowDeleteAllConfirm] = useState(false);

  if (!show) return null;

  const renderTabSpeech = () => (
    <>
      {/* Provider-specific settings (no provider selector — now on main page) */}

      {/* Backend selector — shown when local_embedded is selected */}
      {config.asr_provider === "local_embedded" && (
        <div className="config-field">
          <label>{t("config.backend")}</label>
          <select
            value={config.asr_backend || "sherpa-onnx"}
            onChange={async (e) => {
              const backend = e.target.value;
              updateConfig("asr_backend", backend);
              try {
                await invoke("set_stt_backend", {
                  sttConfigPath: config.stt_config_path,
                  backend,
                });
                const status = await invoke<SttModelStatus>("check_stt_model", {
                  sttConfigPath: config.stt_config_path,
                });
                setModelStatus(status);
              } catch (err) {
                console.error("Failed to update STT backend:", err);
              }
            }}
          >
            <option value="sherpa-onnx">{t("config.backendStandard")}</option>
            <option value="hybrid">{t("config.backendHybrid")}</option>
          </select>
        </div>
      )}

      {/* ONNX Provider */}
      <div className="config-field">
        <label>{t("config.onnxProvider")}</label>
        <select
          value={config.onnx_provider || "cpu"}
          onChange={(e) => updateConfig("onnx_provider", e.target.value)}
        >
          <option value="cpu">CPU</option>
          <option value="cuda">CUDA</option>
        </select>
      </div>

      {/* Provider-specific settings */}
      {config.asr_provider === "local" ? (
        <div className="config-field">
          <label>{t("config.localSttUrl")}</label>
          <input
            type="text"
            value={config.local_stt_url}
            onChange={(e) => updateConfig("local_stt_url", e.target.value)}
          />
        </div>
      ) : config.asr_provider === "local_embedded" ? (
        <>
          <div className="config-field">
            <label>{t("config.backend")}</label>
            <select
              value={currentModelName}
              onChange={async (e) => {
                const name = e.target.value;
                setCurrentModelName(name);
                try {
                  await invoke("set_stt_model", {
                    sttConfigPath: config.stt_config_path,
                    modelName: name,
                  });
                  const status = await invoke<SttModelStatus>("check_stt_model", {
                    sttConfigPath: config.stt_config_path,
                  });
                  setModelStatus(status);
                } catch (err) {
                  console.error("Failed to switch model:", err);
                }
              }}
            >
              {availableModels.map((m) => (
                <option key={m.name} value={m.name}>
                  {m.display_name}
                </option>
              ))}
            </select>
          </div>
          {/* Model status & download */}
          <div className="config-field model-status-section">
            {modelStatus ? (
              modelStatus.exists ? (
                <span className="model-status ok">
                  {t("model.exists", modelStatus.model_name)}
                </span>
              ) : (
                <div className="model-missing">
                  <span className="model-status error">
                    {t("model.missing", modelStatus.missing_files.join(", "))}
                  </span>
                  {downloadError && (
                    <span className="model-status error" style={{ whiteSpace: "pre-wrap" }}>
                      {t("model.downloadError")}: {downloadError}
                    </span>
                  )}
                  <button
                    className="download-button"
                    disabled={isDownloading}
                    onClick={() => {
                      setDownloadError("");
                      setIsDownloading(true);
                      setDownloadProgress({ phase: "connecting", current: 0, total: 0 });
                      invoke("download_stt_model", {
                        sttConfigPath: config.stt_config_path,
                        force: false,
                      }).catch((e) => {
                        setDownloadError(String(e));
                        setIsDownloading(false);
                      });
                    }}
                  >
                    {isDownloading ? t("model.downloading") : t("model.download")}
                  </button>
                </div>
              )
            ) : (
              <span className="model-status checking">
                {modelCheckError
                  ? t("model.checkError", modelCheckError)
                  : t("model.statusChecking")}
              </span>
            )}
            {isDownloading && (
              <div className="download-progress">
                <div className="progress-bar">
                  <div
                    className="progress-fill"
                    style={{
                      width:
                        downloadProgress.total > 0
                          ? `${((downloadProgress.current / downloadProgress.total) * 100).toFixed(1)}%`
                          : "0%",
                    }}
                  />
                </div>
                <span className="progress-text">
                  {downloadProgress.phase}
                  {downloadProgress.total > 0
                    ? `: ${(downloadProgress.current / 1048576).toFixed(1)}MB / ${(downloadProgress.total / 1048576).toFixed(1)}MB`
                    : ""}
                </span>
              </div>
            )}
          </div>
        </>
      ) : (
        <>
          <h3 style={{ margin: "1rem 0 0.5rem", color: "#ccc" }}>{t("config.tencentCredentials")}</h3>
          <div className="config-field" style={{ marginBottom: "0.75rem" }}>
            <a
              href="https://console.cloud.tencent.com/asr"
              target="_blank"
              rel="noreferrer"
              style={{ color: "#4fc3f7", fontSize: "0.85rem" }}
            >
              {t("config.tencentConsoleLink")}
            </a>
          </div>
          <div className="config-field">
            <label>{t("config.tencentAppId")}</label>
            <input
              type="text"
              value={config.tencent_app_id}
              onChange={(e) => updateConfig("tencent_app_id", e.target.value)}
            />
          </div>
          <div className="config-field">
            <label>{t("config.tencentSecretId")}</label>
            <input
              type="text"
              value={config.tencent_secret_id}
              onChange={(e) => updateConfig("tencent_secret_id", e.target.value)}
            />
          </div>
          <div className="config-field">
            <label>{t("config.tencentSecretKey")}</label>
            <input
              type="password"
              value={config.tencent_secret_key}
              onChange={(e) => updateConfig("tencent_secret_key", e.target.value)}
            />
          </div>
        </>
      )}

      {/* OSC Config */}
      <h3>{t("config.osc")}</h3>
      <div className="config-field">
        <label>{t("config.host")}</label>
        <input
          type="text"
          value={config.osc_host}
          onChange={(e) => updateConfig("osc_host", e.target.value)}
        />
      </div>
      <div className="config-field">
        <label>{t("config.port")}</label>
        <input
          type="number"
          value={config.osc_port}
          onChange={(e) => updateConfig("osc_port", Number(e.target.value))}
        />
      </div>
      <div className="config-field">
        <label>{t("config.lineCount")}</label>
        <input
          type="number"
          value={config.osc_line_count}
          min="1"
          max="10"
          onChange={(e) => updateConfig("osc_line_count", Number(e.target.value))}
        />
      </div>
      <div className="config-field">
        <label>{t("config.retentionSecs")}</label>
        <input
          type="number"
          value={config.osc_retention_secs}
          min="1"
          max="60"
          onChange={(e) => updateConfig("osc_retention_secs", Number(e.target.value))}
        />
      </div>
      <div className="config-field">
        <label>
          <input
            type="checkbox"
            checked={config.osc_remove_period}
            onChange={(e) => updateConfig("osc_remove_period", e.target.checked)}
          />
          {t("config.removePeriod")}
        </label>
      </div>
      <div className="config-field">
        <label>
          <input
            type="checkbox"
            checked={config.osc_enabled}
            onChange={(e) => updateConfig("osc_enabled", e.target.checked)}
          />
          {t("config.oscEnabled")}
        </label>
      </div>

      {/* Trigger Words */}
      <h3>{t("config.trigger")}</h3>
      <div className="config-field">
        <label>{t("config.triggerStart")}</label>
        <input
          type="text"
          value={config.trigger_start}
          onChange={(e) => updateConfig("trigger_start", e.target.value)}
        />
      </div>
      <div className="config-field">
        <label>{t("config.triggerStop")}</label>
        <input
          type="text"
          value={config.trigger_stop}
          onChange={(e) => updateConfig("trigger_stop", e.target.value)}
        />
      </div>

      {/* Hotkey */}
      <h3>{t("config.hotkey")}</h3>
      <div className="config-field">
        <label>
          <input
            type="checkbox"
            checked={config.global_hotkey_enabled}
            onChange={(e) => updateConfig("global_hotkey_enabled", e.target.checked)}
          />
          {t("config.hotkeyEnabled")}
        </label>
      </div>

      {/* Trigger Listener */}
      <h3>{t("config.triggerListener")}</h3>
      <div className="config-field">
        <label>
          <input
            type="checkbox"
            checked={config.trigger_listener_enabled}
            onChange={(e) => updateConfig("trigger_listener_enabled", e.target.checked)}
          />
          {t("config.triggerListenerEnabled")}
        </label>
      </div>
      {config.trigger_listener_enabled && (
        <div className="config-field">
          <label>{t("config.triggerSttProvider")}</label>
          <select
            value={config.trigger_stt_provider}
            onChange={(e) => updateConfig("trigger_stt_provider", e.target.value)}
          >
            <option value="local">{t("config.triggerSttProviderLocal")}</option>
            <option value="local_embedded">
              {t("config.triggerSttProviderLocalEmbedded")}
            </option>
          </select>
        </div>
      )}
    </>
  );

  const renderTabVad = () => (
    <>
      <div className="config-field">
        <label>
          <input
            type="checkbox"
            checked={config.vad_enabled ?? false}
            onChange={(e) => updateConfig("vad_enabled", e.target.checked)}
          />
          {t("config.vadEnable")}
        </label>
      </div>
      <div className="config-field">
        <label>{t("config.vadRule1")}</label>
        <input
          type="number"
          value={config.vad_sentence_silence ?? 1.2}
          step="0.1"
          min="0.1"
          onChange={(e) =>
            updateConfig("vad_sentence_silence", Number(e.target.value))
          }
        />
      </div>
      <div className="config-field">
        <label>{t("config.vadRule2")}</label>
        <input
          type="number"
          value={config.vad_sub_phrase_silence ?? 0.6}
          step="0.1"
          min="0.1"
          onChange={(e) =>
            updateConfig("vad_sub_phrase_silence", Number(e.target.value))
          }
        />
      </div>
      <div className="config-field">
        <label>{t("config.vadRule3")}</label>
        <input
          type="number"
          value={config.vad_min_utterance ?? 200.0}
          step="10"
          min="10"
          onChange={(e) =>
            updateConfig("vad_min_utterance", Number(e.target.value))
          }
        />
      </div>
    </>
  );

  const renderTabMaintenance = () => (
    <div className="maintenance-section">
      {/* Reset defaults */}
      <div className="maintenance-item">
        {showResetConfirm ? (
          <div className="confirm-reset">
            <span>{t("config.confirmReset")}</span>
            <div className="confirm-reset-buttons">
              <button
                className="danger-button"
                onClick={() => {
                  resetConfig();
                  setShowResetConfirm(false);
                }}
              >
                {t("config.resetYes")}
              </button>
              <button
                className="cancel-button"
                onClick={() => setShowResetConfirm(false)}
              >
                {t("config.resetNo")}
              </button>
            </div>
          </div>
        ) : (
          <button
            className="reset-button"
            onClick={() => setShowResetConfirm(true)}
          >
            {t("config.resetDefaults")}
          </button>
        )}
      </div>

      {/* Delete models */}
      <div className="maintenance-item">
        {showDeleteModelsConfirm ? (
          <div className="confirm-reset">
            <span>{t("config.confirmReset")}</span>
            <div className="confirm-reset-buttons">
              <button
                className="danger-button"
                onClick={async () => {
                  setShowDeleteModelsConfirm(false);
                  try {
                    await invoke("delete_stt_model", {
                      sttConfigPath: config.stt_config_path,
                    });
                  } catch (err) {
                    console.error("Failed to delete models:", err);
                  }
                }}
              >
                {t("config.resetYes")}
              </button>
              <button
                className="cancel-button"
                onClick={() => setShowDeleteModelsConfirm(false)}
              >
                {t("config.resetNo")}
              </button>
            </div>
          </div>
        ) : (
          <button
            className="reset-button"
            onClick={() => setShowDeleteModelsConfirm(true)}
          >
            {t("config.deleteModels")}
          </button>
        )}
      </div>

      {/* Delete all data */}
      <div className="maintenance-item">
        {showDeleteAllConfirm ? (
          <div className="confirm-reset">
            <span>{t("config.confirmReset")}</span>
            <div className="confirm-reset-buttons">
              <button
                className="danger-button"
                onClick={async () => {
                  setShowDeleteAllConfirm(false);
                  try {
                    await invoke("clear_all_data");
                  } catch (err) {
                    console.error("Failed to clear all data:", err);
                  }
                }}
              >
                {t("config.resetYes")}
              </button>
              <button
                className="cancel-button"
                onClick={() => setShowDeleteAllConfirm(false)}
              >
                {t("config.resetNo")}
              </button>
            </div>
          </div>
        ) : (
          <button
            className="reset-button"
            onClick={() => setShowDeleteAllConfirm(true)}
          >
            {t("config.deleteAll")}
          </button>
        )}
      </div>
    </div>
  );

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t("config.title")}</h2>
          <button className="modal-close" onClick={onClose}>
            X
          </button>
        </div>

        {/* Tab Bar */}
        <div className="settings-tabs">
          {TABS.map((tab, idx) => (
            <button
              key={tab.key}
              className={`settings-tab ${idx === activeTab ? "active" : ""}`}
              onClick={() => setActiveTab(idx)}
            >
              {t(tab.labelKey)}
            </button>
          ))}
        </div>

        <div className="modal-body">
          {activeTab === 0 && renderTabSpeech()}
          {activeTab === 1 && renderTabVad()}
          {activeTab === 2 && renderTabMaintenance()}
        </div>
      </div>
    </div>
  );
}
