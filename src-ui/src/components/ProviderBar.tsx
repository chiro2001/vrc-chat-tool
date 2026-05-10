import { t } from "../i18n";
import type { AppConfig } from "../types";

interface Props {
  config: AppConfig;
  updateConfig: (field: keyof AppConfig, value: string | number | boolean) => void;
  disabled?: boolean;
  onProviderChange?: (value: string) => void;
}

const PROVIDERS = [
  { value: "local_embedded", labelKey: "config.providerLocalEmbeddedShort" },
  { value: "local", labelKey: "config.providerLocalShort" },
  { value: "tencent", labelKey: "config.providerTencentShort" },
] as const;

export default function ProviderBar({ config, updateConfig, disabled, onProviderChange }: Props) {
  return (
    <div className="provider-bar">
      {PROVIDERS.map(({ value, labelKey }) => (
        <button
          key={value}
          className={`provider-btn ${config.asr_provider === value ? "active" : ""}`}
          onClick={() => {
            if (onProviderChange) {
              onProviderChange(value);
            } else {
              updateConfig("asr_provider", value);
            }
          }}
        >
          {t(labelKey)}
        </button>
      ))}
    </div>
  );
}
