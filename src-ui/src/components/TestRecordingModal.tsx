import { t } from "../i18n";
import type { TestRecording } from "../types";

interface Props {
  show: boolean;
  onClose: () => void;
  isTestRecording: boolean;
  toggleTestRecording: () => void;
  testRecordings: TestRecording[];
  loadRecordings: () => void;
  playRecording: (filepath: string) => void;
  deleteRecording: (filename: string) => void;
}

export default function TestRecordingModal({
  show,
  onClose,
  isTestRecording,
  toggleTestRecording,
  testRecordings,
  loadRecordings,
  playRecording,
  deleteRecording,
}: Props) {
  if (!show) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t("test.title")}</h2>
          <button className="modal-close" onClick={onClose}>X</button>
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
              {testRecordings.map((rec) => (
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
  );
}
