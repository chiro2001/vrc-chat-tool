import { t } from "../i18n";
import type { LogEntry } from "../types";

interface Props {
  show: boolean;
  onToggle: () => void;
  logs: LogEntry[];
  logFilter: string;
  setLogFilter: (filter: string) => void;
  onClear: () => void;
}

export default function LogPanel({
  show,
  onToggle,
  logs,
  logFilter,
  setLogFilter,
  onClear,
}: Props) {
  return (
    <div className="log-panel-wrapper">
      <button
        className="log-toggle-button"
        onClick={onToggle}
      >
        {show ? t("log.hide") : t("log.show")} ({logs.length})
      </button>

      {show && (
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
              onClick={onClear}
            >
              {t("log.clear")}
            </button>
          </div>

          <div className="log-entries">
            {logs
              .filter((l) => logFilter === "all" || l.level === logFilter)
              .slice(-100)
              .map((log, i) => (
                <div key={`${log.timestamp}-${i}`} className="log-line">
                  <span className={`log-level log-level-${log.level}`}>{log.level}</span>
                  <span className="log-module">[{log.module}]</span>
                  <span className="log-message">{log.message}</span>
                </div>
              ))}
          </div>
        </section>
      )}
    </div>
  );
}
