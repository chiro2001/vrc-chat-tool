/// Overlay window — transparent always-on-top display showing real-time recognition.
/// Two-line layout: current partial (top) + last confirmed sentence (bottom).
import { useState, useEffect } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import ReactDOM from "react-dom/client";
import "./overlay.css";

function Overlay() {
  const [status, setStatus] = useState<string>("idle");
  const [currentPartial, setCurrentPartial] = useState("");
  const [lastSentence, setLastSentence] = useState("");
  const [currentVolume, setCurrentVolume] = useState(0);
  const [lastError, setLastError] = useState("");

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];

    listen<string>("recording-started", () => {
      setStatus("recording");
      setCurrentPartial("");
      setLastError("");
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-partial", (event) => {
      setStatus("recognizing");
      setCurrentPartial(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-sentence", (event) => {
      setLastSentence(event.payload);
      setCurrentPartial("");
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-complete", () => {
      setStatus("idle");
      setCurrentPartial("");
    }).then((fn) => unlisteners.push(fn));

    listen<string>("recording-error", (event) => {
      setStatus("error");
      setLastError(event.payload);
    }).then((fn) => unlisteners.push(fn));

    listen<number>("volume-update", (event) => {
      setCurrentVolume(event.payload);
    }).then((fn) => unlisteners.push(fn));

    return () => { unlisteners.forEach((fn) => fn()); };
  }, []);

  const statusText = () => {
    switch (status) {
      case "recording": return "录音中...";
      case "recognizing": return "识别中...";
      case "error": return "错误";
      default: return "";
    }
  };

  return (
    <div className="overlay-container" data-tauri-drag-region>
      {status !== "idle" && (
        <div className={`overlay-status status-${status}`}>
          <span className="status-dot" />
          <span>{statusText()}</span>
          <span className="volume-bar">
            <div className="volume-fill" style={{ width: `${(currentVolume * 100).toFixed(0)}%` }} />
          </span>
        </div>
      )}

      <div className={`overlay-partial${currentPartial ? " visible" : ""}`}>
        {currentPartial || (status === "idle" && !lastSentence ? "就绪 — 等待语音输入" : "")}
      </div>

      {lastSentence && <div className="overlay-sentence">{lastSentence}</div>}
      {lastError && <div className="overlay-error">{lastError}</div>}
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("overlay-root")!).render(<Overlay />);
