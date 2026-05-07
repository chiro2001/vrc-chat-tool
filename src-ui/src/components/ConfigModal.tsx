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
  // Tencent credentials
  credsExist: boolean;
  credAppId: string;
  credSecretId: string;
  credSecretKey: string;
  credSaveMsg: string;
  setCredAppId: (v: string) => void;
  setCredSecretId: (v: string) => void;
  setCredSecretKey: (v: string) => void;
  saveCredentials: () => void;
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

export default function ConfigModal({
  show,
  onClose,
  config,
  updateConfig,
  credsExist,
  credAppId,
  credSecretId,
  credSecretKey,
  credSaveMsg,
  setCredAppId,
  setCredSecretId,
  setCredSecretKey,
  saveCredentials,
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
  if (!show) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t("config.title")}</h2>
          <button className="modal-close" onClick={onClose}>X</button>
        </div>
        <div className="modal-body">
          {/* ASR Provider */}
          <div className="config-field">
            <label>{t("config.provider")}</label>
            <select value={config.asr_provider} onChange={(e) => updateConfig("asr_provider", e.target.value)}>
              <option value="tencent">{t("config.providerTencent")}</option>
              <option value="local">{t("config.providerLocal")}</option>
              <option value="local_embedded">{t("config.providerLocalEmbedded")}</option>
            </select>
          </div>

          {/* Provider-specific settings */}
          {config.asr_provider === "local" ? (
            <div className="config-field">
              <label>{t("config.localSttUrl")}</label>
              <input type="text" value={config.local_stt_url} onChange={(e) => updateConfig("local_stt_url", e.target.value)} />
            </div>
          ) : config.asr_provider === "local_embedded" ? (
            <>
              <div className="config-field">
                <label>模型选择</label>
                <select
                  value={currentModelName}
                  onChange={async (e) => {
                    const name = e.target.value;
                    setCurrentModelName(name);
                    try {
                      await invoke("set_stt_model", { sttConfigPath: config.stt_config_path, modelName: name });
                      const status = await invoke<SttModelStatus>("check_stt_model", { sttConfigPath: config.stt_config_path });
                      setModelStatus(status);
                    } catch (err) {
                      console.error("Failed to switch model:", err);
                    }
                  }}
                >
                  {availableModels.map((m) => (
                    <option key={m.name} value={m.name}>{m.display_name}</option>
                  ))}
                </select>
              </div>
              {/* Model status & download */}
              <div className="config-field model-status-section">
                {modelStatus ? (
                  modelStatus.exists ? (
                    <span className="model-status ok">{t("model.exists", modelStatus.model_name)}</span>
                  ) : (
                    <div className="model-missing">
                      <span className="model-status error">{t("model.missing", modelStatus.missing_files.join(", "))}</span>
                      {downloadError && (
                        <span className="model-status error" style={{ whiteSpace: "pre-wrap" }}>{t("model.downloadError")}: {downloadError}</span>
                      )}
                      <button
                        className="download-button"
                        disabled={isDownloading}
                        onClick={() => {
                          setDownloadError("");
                          setIsDownloading(true);
                          setDownloadProgress({ phase: "connecting", current: 0, total: 0 });
                          invoke("download_stt_model", { sttConfigPath: config.stt_config_path, force: false })
                            .catch((e) => { setDownloadError(String(e)); setIsDownloading(false); });
                        }}
                      >
                        {isDownloading ? t("model.downloading") : t("model.download")}
                      </button>
                    </div>
                  )
                ) : (
                  <span className="model-status checking">{modelCheckError ? t("model.checkError", modelCheckError) : t("model.statusChecking")}</span>
                )}
                {isDownloading && (
                  <div className="download-progress">
                    <div className="progress-bar">
                      <div
                        className="progress-fill"
                        style={{
                          width: downloadProgress.total > 0
                            ? `${((downloadProgress.current / downloadProgress.total) * 100).toFixed(1)}%`
                            : "0%"
                        }}
                      />
                    </div>
                    <span className="progress-text">
                      {downloadProgress.phase}{downloadProgress.total > 0 ? `: ${(downloadProgress.current / 1048576).toFixed(1)}MB / ${(downloadProgress.total / 1048576).toFixed(1)}MB` : ""}
                    </span>
                  </div>
                )}
              </div>
            </>
          ) : (
            <div className="config-field">
              <label>{t("config.credentialsFile")}</label>
              <input type="text" value={config.tencent_credentials_file} onChange={(e) => updateConfig("tencent_credentials_file", e.target.value)} />
            </div>
          )}

          {/* Tencent Credentials */}
          <h3>{t("config.tencentCredentials")}</h3>
          {credSaveMsg === "loadError" && <p className="cred-status-msg error" style={{ margin: "0 0 0.5rem 0" }}>{t("config.tencentCredentialsLoadFailed")}</p>}
          <div className="config-field">
            <label>{t("config.tencentAppId")}</label>
            <input type="password" placeholder={credsExist ? "••••••••" : t("config.placeholder.credentials")} value={credAppId} onChange={(e) => setCredAppId(e.target.value)} />
          </div>
          <div className="config-field">
            <label>{t("config.tencentSecretId")}</label>
            <input type="password" placeholder={credsExist ? "••••••••" : t("config.placeholder.credentials")} value={credSecretId} onChange={(e) => setCredSecretId(e.target.value)} />
          </div>
          <div className="config-field">
            <label>{t("config.tencentSecretKey")}</label>
            <input type="password" placeholder={credsExist ? "••••••••" : t("config.placeholder.credentials")} value={credSecretKey} onChange={(e) => setCredSecretKey(e.target.value)} />
          </div>
          <div className="config-field" style={{ flexDirection: "row", alignItems: "center", gap: "0.75rem" }}>
            <button className="save-button" onClick={saveCredentials}>{t("config.save")}</button>
            {credSaveMsg === "saved" && <span className="cred-status-msg saved">{t("config.tencentCredentialsSaved")}</span>}
            {credSaveMsg === "error" && <span className="cred-status-msg error">{t("config.tencentCredentialsSaveFailed")}</span>}
          </div>

          {/* OSC Config */}
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
          <div className="config-field">
            <label>
              <input type="checkbox" checked={config.osc_enabled} onChange={(e) => updateConfig("osc_enabled", e.target.checked)} />
              {t("config.oscEnabled")}
            </label>
          </div>

          {/* Trigger Words */}
          <h3>{t("config.trigger")}</h3>
          <div className="config-field">
            <label>{t("config.triggerStart")}</label>
            <input type="text" value={config.trigger_start} onChange={(e) => updateConfig("trigger_start", e.target.value)} />
          </div>
          <div className="config-field">
            <label>{t("config.triggerStop")}</label>
            <input type="text" value={config.trigger_stop} onChange={(e) => updateConfig("trigger_stop", e.target.value)} />
          </div>

          {/* Hotkey */}
          <h3>{t("config.hotkey")}</h3>
          <div className="config-field">
            <label>
              <input type="checkbox" checked={config.global_hotkey_enabled}
                onChange={(e) => updateConfig("global_hotkey_enabled", e.target.checked)} />
              {t("config.hotkeyEnabled")}
            </label>
          </div>

          {/* Trigger Listener */}
          <h3>{t("config.triggerListener")}</h3>
          <div className="config-field">
            <label>
              <input type="checkbox" checked={config.trigger_listener_enabled}
                onChange={(e) => updateConfig("trigger_listener_enabled", e.target.checked)} />
              {t("config.triggerListenerEnabled")}
            </label>
          </div>
          {config.trigger_listener_enabled && (
            <div className="config-field">
              <label>{t("config.triggerSttProvider")}</label>
              <select value={config.trigger_stt_provider} onChange={(e) => updateConfig("trigger_stt_provider", e.target.value)}>
                <option value="local">{t("config.triggerSttProviderLocal")}</option>
                <option value="local_embedded">{t("config.triggerSttProviderLocalEmbedded")}</option>
              </select>
            </div>
          )}

          {/* Reset */}
          <div className="config-reset-section">
            {showResetConfirm ? (
              <div className="confirm-reset">
                <span>{t("config.confirmReset")}</span>
                <div className="confirm-reset-buttons">
                  <button className="danger-button" onClick={() => { resetConfig(); setShowResetConfirm(false); }}>{t("config.resetYes")}</button>
                  <button className="cancel-button" onClick={() => setShowResetConfirm(false)}>{t("config.resetNo")}</button>
                </div>
              </div>
            ) : (
              <button className="reset-button" onClick={() => setShowResetConfirm(true)}>
                {t("config.reset")}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
