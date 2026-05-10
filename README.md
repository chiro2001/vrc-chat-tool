# VRC 语音 OSC

VRChat 桌面语音转文字工具。麦克风音频 → 本地 ASR 识别 → OSC 消息 → VRChat 聊天框。

## 特性

- **本地混合推理**：Zipformer CTC 流式识别 + SenseVoice 离线校正，无需网络
- **腾讯云 ASR**：支持腾讯云语音识别 API，带 VAD 过滤和用量统计
- **远程 STT**：连接远程 STT 服务器 (WebSocket)
- **OSC 控制**：自动发送到 VRChat 聊天框，支持多行缓冲和打字状态
- **语音触发**：说"开始语音识别"自动开始，说"停止语音识别"自动结束
- **全局热键**：F10 开始/停止录音
- **VAD 端点检测**：自动识别句子边界，逐句输出
- **测试录音**：录制带 VAD 的音频用于调试模型效果

## 技术栈

- **桌面框架**：Tauri v1 (Rust + WebView2)
- **前端**：React 19 + TypeScript + Vite 8
- **ASR 引擎**：Sherpa-ONNX (k2-fsa)
- **OSC**：rosc (UDP)
- **数据库**：SQLite (rusqlite, 识别历史)

## 系统要求

- Windows 10/11
- VB-Cable（虚拟音频设备，用于回声测试）

## 安装

从 [Releases](https://github.com/YOUR_REPO/vrc-chat-tool/releases) 下载 `vrc-chat-tool.exe`。

### 首次运行

1. 将 `vrc-chat-tool.exe` 放到任意目录
2. 双击运行：软件会自动在 `exe 所在目录` 创建 `stt-config.yaml`
3. 点击"下载模型"下载 CTC + SenseVoice 模型（约 176 MB，首次需要下载）
4. 下载完成后即可使用

### 配置

| 文件 | 用途 |
|------|------|
| `config.yaml` | 语音后端、OSC、触发词等设置 |
| `stt-config.yaml` | Sherpa-ONNX 模型路径和 VAD 配置 |
| `.env` | 腾讯云 API 凭证（可选） |

## 开发

```bash
# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 构建
npm run tauri build
```

## 语音后端

| 后端 | 说明 |
|------|------|
| 本地嵌入式 | Zipformer CTC 流式 + SenseVoice 离线校正（默认） |
| 远程 STT | 连接 WebSocket STT 服务器 |
| 腾讯云 | 腾讯云语音识别 API |

## 许可证

MIT
