import { useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { t } from "../i18n";
import type {
  AppConfig,
  SttModelStatus,
  AvailableModel,
  DownloadProgress,
} from "../types";

interface Props {
  config: AppConfig;
  updateConfig: (field: keyof AppConfig, value: string | number | boolean) => void;
  modelStatus?: SttModelStatus | null;
  availableModels?: AvailableModel[];
  currentModelName?: string;
  onOpenSettings?: () => void;
  triggerStatus?: string;
  /** For local_embedded model download */
  isDownloading?: boolean;
  downloadProgress?: DownloadProgress;
  downloadError?: string;
  setCurrentModelName?: (v: string) => void;
  setModelStatus?: (v: SttModelStatus | null) => void;
  setDownloadError?: (v: string) => void;
  setIsDownloading?: (v: boolean) => void;
  setDownloadProgress?: (v: DownloadProgress) => void;
}

export default function ProviderCard({
  config,
  updateConfig,
  modelStatus,
  availableModels,
  currentModelName,
  onOpenSettings,
  triggerStatus,
  isDownloading,
  downloadProgress,
  downloadError,
  setCurrentModelName,
  setModelStatus,
  setDownloadError,
  setIsDownloading,
  setDownloadProgress,
}: Props) {
  const provider = config.asr_provider;

  const hasTencentCreds =
    config.tencent_app_id.trim() !== "" &&
    config.tencent_secret_id.trim() !== "" &&
    config.tencent_secret_key.trim() !== "";

  const localConfigured = config.local_stt_url.trim() !== "";

  return (
    <div className="provider-card">
      {provider === "local_embedded" && (
        <div className="provider-card-content">
          {/* Only show card content when model is not ready */}
          {modelStatus && modelStatus.exists ? null : (
            <>
          {/* Model selector */}
          {availableModels && availableModels.length > 0 && (
            <div className="provider-card-row">
              <span className="provider-card-label">{t("config.model")}</span>
              <select
                className="provider-card-select"
                value={currentModelName || modelStatus?.model_name || ""}
                onChange={async (e) => {
                  const name = e.target.value;
                  if (setCurrentModelName) setCurrentModelName(name);
                  try {
                    await invoke("set_stt_model", {
                      sttConfigPath: config.stt_config_path,
                      modelName: name,
                    });
                    const status = await invoke<SttModelStatus>("check_stt_model", {
                      sttConfigPath: config.stt_config_path,
                    });
                    if (setModelStatus) setModelStatus(status);
                    if (setCurrentModelName) setCurrentModelName(status.model_name);
                  } catch (err) {
                    console.error("Failed to switch model:", err);
                  }
                }}
              >
                {availableModels
                  .filter((m) => config.asr_backend === "hybrid" || m.backend === config.asr_backend)
                  .map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.display_name}
                  </option>
                ))}
              </select>
              {modelStatus ? (
                modelStatus.exists ? (
                  <span className="model-status ok">{t("model.exists", currentModelName || modelStatus.model_name)}</span>
                ) : (
                  <span className="model-status error">{t("model.missing", modelStatus.missing_files.join(", "))}</span>
                )
              ) : (
                <span className="model-status checking">
                  {t("model.statusChecking")}
                </span>
              )}
            </div>
          )}

          {/* Download button (when model missing) */}
          {modelStatus && !modelStatus.exists && (
            <div className="provider-card-row">
              {downloadError && (
                <span className="model-status error" style={{ whiteSpace: "pre-wrap" }}>
                  {t("model.downloadError")}: {downloadError}
                </span>
              )}
              <button
                className="download-button"
                disabled={isDownloading}
                onClick={() => {
                  if (setDownloadError) setDownloadError("");
                  if (setIsDownloading) setIsDownloading(true);
                  if (setDownloadProgress)
                    setDownloadProgress({ phase: "connecting", current: 0, total: 0 });
                  invoke("download_stt_model", {
                    sttConfigPath: config.stt_config_path,
                    force: false,
                  }).catch((e) => {
                    if (setDownloadError) setDownloadError(String(e));
                    if (setIsDownloading) setIsDownloading(false);
                  });
                }}
              >
                {isDownloading ? t("model.downloading") : t("model.download")}
              </button>
            </div>
          )}

          {/* Download progress */}
          {isDownloading && downloadProgress && (
            <div className="download-progress provider-card-progress">
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

          {/* Open settings link */}
          {onOpenSettings && (
            <div className="provider-card-row provider-card-action">
              <button className="provider-card-link" onClick={onOpenSettings}>
                {t("config.title")} →
              </button>
            </div>
          )}
            </>
          )}
        </div>
      )}

      {provider === "local" && (
        <div className="provider-card-content">
          <div className="provider-card-row">
            <span className="provider-card-label">{t("config.localSttUrl")}</span>
            <span className="provider-card-value">{config.local_stt_url}</span>
          </div>
          <div className="provider-card-row">
            <span className={`provider-card-status ${localConfigured ? "ok" : "error"}`}>
              {localConfigured ? "✅ " + t("status.configured") : "❌ " + t("status.notConfigured")}
            </span>
          </div>
          {onOpenSettings && (
            <div className="provider-card-row provider-card-action">
              <button className="provider-card-link" onClick={onOpenSettings}>
                {t("config.title")} →
              </button>
            </div>
          )}
        </div>
      )}

      {provider === "tencent" && (
        <div className="provider-card-content">
          <div className="provider-card-row">
            <span className="provider-card-label">{t("config.tencent")}</span>
            <span className={`provider-card-status ${hasTencentCreds ? "ok" : "error"}`}>
              {hasTencentCreds
                ? "✅ " + t("config.tencentCredentialsSaved")
                : "❌ " + t("config.placeholder.credentials")}
            </span>
          </div>
          {hasTencentCreds && (
            <>
            <div className="provider-card-row">
              <span className="provider-card-label">VAD</span>
              <span className={`provider-card-status ${config.vad_enabled ? "ok" : ""}`}>
                {config.vad_enabled ? t("config.vadOn") : t("config.vadOff")}
              </span>
            </div>
            <div className="provider-card-row">
              <span className="provider-card-label">{t("config.tencentUsage")}</span>
              <span className="provider-card-value">
                {(() => {
                  const s = config.tencent_usage_seconds;
                  if (s < 60) return `${s} ${t("config.seconds")}`;
                  if (s < 3600) return `${Math.floor(s / 60)}${t("config.minutes")}${s % 60}${t("config.seconds")}`;
                  return `${(s / 3600).toFixed(2)} ${t("config.hours")}`;
                })()}
              </span>
            </div>
            </>
          )}
          {!hasTencentCreds && onOpenSettings && (
            <div className="provider-card-row provider-card-action">
              <button className="provider-card-link" onClick={onOpenSettings}>
                {t("config.tencentCredentials")} →
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
