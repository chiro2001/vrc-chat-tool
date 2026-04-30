// Language detection: use navigator.language, default to zh-CN
// If language starts with "zh", use Chinese; otherwise English
// Allow manual override via a simple toggle

type Lang = "zh" | "en";

const translations: Record<Lang, Record<string, string>> = {
  zh: {
    "app.title": "VRC 语音 OSC",
    "status.ready": "就绪",
    "status.recording": "录音中...",
    "status.recognizing": "识别中...",
    "status.done": "完成",
    "status.error": "错误",
    "control.audioDevice": "音频设备:",
    "control.volume": "音量:",
    "control.startRecording": "开始录音",
    "control.stop": "停止",
    "results.listening": "正在听:",
    "results.result": "结果:",
    "results.error": "错误:",
    "test.title": "录音测试",
    "test.start": "开始测试录音",
    "test.stop": "停止测试",
    "test.refresh": "刷新列表",
    "test.play": "播放",
    "test.delete": "删除",
    "test.savedAlert": "录音已保存至:",
    "config.tencent": "腾讯云配置",
    "config.appId": "App ID:",
    "config.secretId": "Secret ID:",
    "config.secretKey": "Secret Key:",
    "config.osc": "OSC 配置",
    "config.host": "主机:",
    "config.port": "端口:",
    "config.placeholder.credentials": "请配置腾讯云凭证",
    "config.placeholder.configNotLoaded": "配置未加载",
    "log.show": "显示日志",
    "log.hide": "隐藏日志",
    "log.allLevels": "所有级别",
    "log.debug": "调试",
    "log.info": "信息",
    "log.warn": "警告",
    "log.error": "错误",
    "log.clear": "清除",
    "recording.audioCaptured": "捕获到 {} 字节的音频数据",
    "recording.noAudio": "未捕获到音频数据",
    "recording.failed": "录音失败",
    "recording.configNeeded": "请先配置腾讯云凭证",
  },
  en: {
    "app.title": "VRC Voice OSC",
    "status.ready": "Ready",
    "status.recording": "Recording...",
    "status.recognizing": "Recognizing...",
    "status.done": "Done",
    "status.error": "Error",
    "control.audioDevice": "Audio Device:",
    "control.volume": "Volume:",
    "control.startRecording": "Start Recording",
    "control.stop": "STOP",
    "results.listening": "Listening:",
    "results.result": "Result:",
    "results.error": "Error:",
    "test.title": "Recording Test",
    "test.start": "Start Test Recording",
    "test.stop": "STOP Test",
    "test.refresh": "Refresh List",
    "test.play": "Play",
    "test.delete": "Delete",
    "test.savedAlert": "Recording saved at:",
    "config.tencent": "Tencent Cloud Config",
    "config.appId": "App ID:",
    "config.secretId": "Secret ID:",
    "config.secretKey": "Secret Key:",
    "config.osc": "OSC Config",
    "config.host": "Host:",
    "config.port": "Port:",
    "config.placeholder.credentials": "Please configure Tencent credentials",
    "config.placeholder.configNotLoaded": "Config not loaded",
    "log.show": "Show Logs",
    "log.hide": "Hide Logs",
    "log.allLevels": "All Levels",
    "log.debug": "Debug",
    "log.info": "Info",
    "log.warn": "Warning",
    "log.error": "Error",
    "log.clear": "Clear",
    "recording.audioCaptured": "Captured {} bytes of audio",
    "recording.noAudio": "No audio data captured",
    "recording.failed": "Recording failed",
    "recording.configNeeded": "Please configure Tencent Cloud credentials",
  },
};

// Detect system language
export function detectLang(): Lang {
  if (typeof navigator !== "undefined") {
    const lang = navigator.language || "";
    if (lang.startsWith("zh")) return "zh";
  }
  return "zh"; // default to Chinese
}

let currentLang: Lang = detectLang();

export function getLang(): Lang {
  return currentLang;
}

export function setLang(lang: Lang) {
  currentLang = lang;
}

export function t(key: string, ...args: string[]): string {
  let text = translations[currentLang]?.[key] ?? translations.zh[key] ?? key;
  // Replace {} placeholders with args
  args.forEach((arg) => {
    text = text.replace("{}", arg);
  });
  return text;
}

// React hook for i18n
import { useState, useCallback } from "react";

export function useI18n() {
  const [, forceUpdate] = useState(0);
  const toggleLang = useCallback(() => {
    currentLang = currentLang === "zh" ? "en" : "zh";
    forceUpdate((n) => n + 1);
  }, []);
  return { t, lang: currentLang, toggleLang };
}
