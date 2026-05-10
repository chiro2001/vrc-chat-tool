/// Overlay window — transparent always-on-top display showing real-time recognition.
/// Two-line layout: current partial (top) + last confirmed sentence (bottom).
/// Uses JS mousedown/move/up for window dragging (Tauri v1 data-tauri-drag-region not reliable).
import { useState, useEffect, useRef } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { appWindow, PhysicalPosition } from "@tauri-apps/api/window";
import ReactDOM from "react-dom/client";
import "./overlay.css";

function Overlay() {
  const [status, setStatus] = useState<string>("idle");
  const [currentPartial, setCurrentPartial] = useState("");
  const [lastSentence, setLastSentence] = useState("");
  const [currentVolume, setCurrentVolume] = useState(0);
  const [lastError, setLastError] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);

  // --- Window dragging via JS events ---
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    let dragging = false;
    let startX = 0;
    let startY = 0;
    let winX = 0;
    let winY = 0;

    const onMouseDown = async (e: MouseEvent) => {
      dragging = true;
      startX = e.screenX;
      startY = e.screenY;
      try {
        const pos = await appWindow.outerPosition();
        winX = pos.x;
        winY = pos.y;
      } catch {}
    };

    const onMouseMove = async (e: MouseEvent) => {
      if (!dragging) return;
      const dx = e.screenX - startX;
      const dy = e.screenY - startY;
      try {
        await appWindow.setPosition(new PhysicalPosition(winX + dx, winY + dy));
      } catch {}
    };

    const onMouseUp = () => { dragging = false; };

    el.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);

    return () => {
      el.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

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

    return () => {
      unlisteners.forEach((fn) => fn());
    };
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
    <div className="overlay-container" ref={containerRef}>
      {status !== "idle" && (
        <div className={`overlay-status status-${status}`}>
          <span className="status-dot" />
          <span>{statusText()}</span>
          <span className="volume-bar">
            <div
              className="volume-fill"
              style={{ width: `${(currentVolume * 100).toFixed(0)}%` }}
            />
          </span>
        </div>
      )}

      {/* Line 1: current partial (real-time) */}
      <div className={`overlay-partial${currentPartial ? " visible" : ""}`}>
        {currentPartial || (status === "idle" && !lastSentence ? "就绪 — 等待语音输入" : "")}
      </div>

      {/* Line 2: last confirmed sentence */}
      {lastSentence && (
        <div className="overlay-sentence">{lastSentence}</div>
      )}

      {lastError && (
        <div className="overlay-error">{lastError}</div>
      )}
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("overlay-root")!).render(<Overlay />);
