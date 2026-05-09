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
    "control.startRecording": "开始语音识别",
    "control.stop": "结束语音识别",
    "results.listening": "识别中:",
    "results.result": "结果:",
    "results.error": "错误:",
    "test.title": "录音测试",
    "test.start": "开始测试录音",
    "test.stop": "停止测试",
    "test.refresh": "刷新列表",
    "test.play": "播放",
    "test.delete": "删除",
    "test.savedAlert": "录音已保存至:",
    "config.title": "设置",
    "config.tencent": "腾讯云配置",
    "config.appId": "App ID:",
    "config.secretId": "Secret ID:",
    "config.secretKey": "Secret Key:",
    "config.credentialsFile": "凭据文件:",
    "config.osc": "OSC 配置",
    "config.host": "主机:",
    "config.port": "端口:",
    "config.lineCount": "显示行数:",
    "config.retentionSecs": "保留时长(秒):",
    "config.removePeriod": "移除末尾标点",
    "config.oscEnabled": "启用OSC发送",
    "config.trigger": "触发词",
    "config.triggerStart": "开始录音词:",
    "config.triggerStop": "停止录音词:",
    "config.provider": "识别服务:",
    "config.providerTencent": "腾讯云",
    "config.providerLocal": "远程部署的TTS服务",
    "config.providerLocalEmbedded": "本地嵌入式推理",
    "config.providerLocalEmbeddedHybrid": "本地混合推理 (Zipformer+SenseVoice)",
    "config.localSttUrl": "远程TTS地址:",
    "config.sttConfigPath": "模型配置路径:",
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
    "history.title": "历史记录",
    "history.empty": "暂无记录",
    "history.clear": "清除",
    "history.confirmClear": "确认清除所有历史记录?",
    "history.yes": "确认",
    "history.no": "取消",
    "config.reset": "恢复默认设置",
    "config.confirmReset": "确认恢复所有设置为默认值？",
    "config.resetYes": "确认恢复",
    "config.resetNo": "取消",
    "config.hotkey": "全局热键",
    "config.hotkeyEnabled": "启用热键 (F10 切换录音)",
    "config.tencentCredentials": "腾讯云凭证",
    "config.tencentAppId": "App ID",
    "config.tencentSecretId": "Secret ID",
    "config.tencentSecretKey": "Secret Key",
    "config.tencentCredentialsSaved": "凭证已保存",
    "config.tencentCredentialsSaveFailed": "凭证保存失败",
    "config.tencentCredentialsLoadFailed": "凭证加载失败",
    "config.save": "保存",
    "stt.connected": "STT 已连接",
    "stt.connecting": "STT 连接中...",
    "stt.disconnected": "STT 已断开",
    "stt.error": "STT 错误",
    "stt.disabled": "STT 监听已关闭",
    "config.triggerListener": "关键词监听",
    "config.triggerListenerEnabled": "程序启动后监听开始关键词",
    "config.triggerSttProvider": "监听推理服务:",
    "config.triggerSttProviderLocal": "远程 STT 服务",
    "config.triggerSttProviderLocalEmbedded": "本地嵌入式推理",
    "config.triggerSttProviderLocalEmbeddedHybrid": "本地混合推理 (Zipformer+SenseVoice)",
    "model.statusChecking": "检查模型状态...",
    "model.checkError": "模型检查失败: {}",
    "model.exists": "模型就绪: {}",
    "model.missing": "模型缺失: {}",
    "model.download": "下载模型",
    "model.downloading": "下载中...",
    "model.downloadComplete": "模型下载完成",
    "model.downloadError": "模型下载失败",
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
    "config.title": "Settings",
    "config.tencent": "Tencent Cloud Config",
    "config.appId": "App ID:",
    "config.secretId": "Secret ID:",
    "config.secretKey": "Secret Key:",
    "config.credentialsFile": "Credentials File:",
    "config.osc": "OSC Config",
    "config.host": "Host:",
    "config.port": "Port:",
    "config.lineCount": "Line Count:",
    "config.retentionSecs": "Retention (s):",
    "config.removePeriod": "Strip Punctuation",
    "config.oscEnabled": "Enable OSC Send",
    "config.trigger": "Trigger Words",
    "config.triggerStart": "Start Trigger:",
    "config.triggerStop": "Stop Trigger:",
    "config.provider": "ASR Provider:",
    "config.providerTencent": "Tencent Cloud",
    "config.providerLocal": "Remotely Deployed TTS",
    "config.providerLocalEmbedded": "Local Embedded Inference",
    "config.providerLocalEmbeddedHybrid": "Local Hybrid (Zipformer+SenseVoice)",
    "config.localSttUrl": "Remote TTS URL:",
    "config.sttConfigPath": "Model Config Path:",
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
    "history.title": "History",
    "history.empty": "No history",
    "history.clear": "Clear",
    "history.confirmClear": "Clear all history?",
    "history.yes": "Yes",
    "history.no": "No",
    "config.reset": "Reset to Defaults",
    "config.confirmReset": "Reset all settings to defaults?",
    "config.resetYes": "Reset",
    "config.resetNo": "Cancel",
    "config.hotkey": "Global Hotkey",
    "config.hotkeyEnabled": "Enable F10 Hotkey Toggle",
    "config.tencentCredentials": "Tencent Cloud Credentials",
    "config.tencentAppId": "App ID",
    "config.tencentSecretId": "Secret ID",
    "config.tencentSecretKey": "Secret Key",
    "config.tencentCredentialsSaved": "Credentials saved",
    "config.tencentCredentialsSaveFailed": "Failed to save credentials",
    "config.tencentCredentialsLoadFailed": "Failed to load credentials",
    "config.save": "Save",
    "stt.connected": "STT Connected",
    "stt.connecting": "STT Connecting...",
    "stt.disconnected": "STT Disconnected",
    "stt.error": "STT Error",
    "stt.disabled": "STT Listener Disabled",
    "config.triggerListener": "Keyword Listener",
    "config.triggerListenerEnabled": "Listen for start keyword after launch",
    "config.triggerSttProvider": "Listener STT Provider:",
    "config.triggerSttProviderLocal": "Remote STT Service",
    "config.triggerSttProviderLocalEmbedded": "Local Embedded Inference",
    "config.triggerSttProviderLocalEmbeddedHybrid": "Local Hybrid (Zipformer+SenseVoice)",
    "model.statusChecking": "Checking model...",
    "model.checkError": "Model check failed: {}",
    "model.exists": "Model ready: {}",
    "model.missing": "Model missing: {}",
    "model.download": "Download Model",
    "model.downloading": "Downloading...",
    "model.downloadComplete": "Download complete",
    "model.downloadError": "Download failed",
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
