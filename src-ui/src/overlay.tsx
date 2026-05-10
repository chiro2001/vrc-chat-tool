/// Overlay window — transparent always-on-top display showing real-time recognition.
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
  const dragPos = useRef({ winX: 0, winY: 0, dragging: false, queried: false });

  useEffect(() => {
    let frame: number;
    const onMouseDown = async (e: MouseEvent) => {
      const d = dragPos.current;
      d.dragging = true;
      if (!d.queried) {
        try {
          const pos = await appWindow.outerPosition();
          d.winX = pos.x;
          d.winY = pos.y;
          d.queried = true;
        } catch {}
      }
    };
    const onMouseMove = (e: MouseEvent) => {
      const d = dragPos.current;
      if (!d.dragging || !d.queried) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        d.winX += e.movementX;
        d.winY += e.movementY;
        appWindow.setPosition(new PhysicalPosition(d.winX, d.winY)).catch(() => {});
      });
    };
    const onMouseUp = () => {
      dragPos.current.dragging = false;
      dragPos.current.queried = false; // re-query on next drag
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    listen<string>("recording-started", () => { setStatus("recording"); setCurrentPartial(""); setLastError(""); }).then(fn => unlisteners.push(fn));
    listen<string>("recording-partial", (e) => { setStatus("recognizing"); setCurrentPartial(e.payload); }).then(fn => unlisteners.push(fn));
    listen<string>("recording-sentence", (e) => { setLastSentence(e.payload); setCurrentPartial(""); }).then(fn => unlisteners.push(fn));
    listen<string>("recording-complete", () => { setStatus("idle"); setCurrentPartial(""); }).then(fn => unlisteners.push(fn));
    listen<string>("recording-error", (e) => { setStatus("error"); setLastError(e.payload); }).then(fn => unlisteners.push(fn));
    listen<number>("volume-update", (e) => { setCurrentVolume(e.payload); }).then(fn => unlisteners.push(fn));
    return () => { unlisteners.forEach(fn => fn()); };
  }, []);

  const st = () => { switch(status) { case "recording": return "录音中..."; case "recognizing": return "识别中..."; case "error": return "错误"; default: return ""; } };

  return (
    <div className="overlay-container">
      {status !== "idle" && (
        <div className={`overlay-status status-${status}`}>
          <span className="status-dot" />
          <span>{st()}</span>
          <span className="volume-bar"><div className="volume-fill" style={{ width: `${(currentVolume*100).toFixed(0)}%` }} /></span>
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
