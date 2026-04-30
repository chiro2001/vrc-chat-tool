# VRC Chat Tool 项目需求文档

参考项目:
- PicoCCB (Kotlin/Android): `D:\Projects\vrc-chat-tool\PicoCCB` - 头显端语音识别原始项目
- Old Tauri Project: `D:\Projects\vrc-chat-tool-old` - 之前的 Tauri 实现

## 0. 软件流程梳理 (从简单到复杂)

### 0.1 数据流全景

```
麦克风 → [cpal 音频捕获] → PCM 16kHz/16bit/mono → [WebSocket客户端] → 腾讯云 ASR API
                                                          ↓
VRChat ← [UDP OSC] ← /chatbox/input ← 识别文本 ← [JSON响应解析]
```

### 0.2 模块依赖图 (从底层到上层)

```
Layer 0 - 基础设施
  ├── config.rs        # YAML配置加载/保存 (无依赖)
  └── (项目脚手架)      # Cargo.toml, package.json, tauri.conf.json

Layer 1 - 独立功能模块 (可并行)
  ├── osc/sender.rs    # OSC UDP发送器 (rosc库)
  ├── speech/tencent.rs # HMAC-SHA1签名生成 (纯算法)
  └── audio/capture.rs  # 音频捕获 + 重采样 (cpal库)

Layer 2 - 组合模块 (依赖Layer 1)
  └── speech/streaming.rs # 流式ASR客户端
       ├── 依赖 tencent.rs (签名URL)
       └── 使用音频PCM数据

Layer 3 - 应用层 (集成所有模块)
  ├── main.rs           # Tauri命令 + 事件系统 + 线程管理
  └── App.tsx           # React前端UI

Layer 4 - 测试
  ├── #[cfg(test)] 单元测试 (每个Rust模块)
  └── test_rust_api.py 集成测试

依赖顺序:
  Tencent Signing → Streaming ASR
  Config + OSC + Audio → main.rs
  Streaming ASR → main.rs
  main.rs → Integration Test
```

### 0.3 程序执行流程

```
1. 启动 → 加载 config.yaml (或使用默认配置)
2. 用户点击"开始录音"
3. Tauri 命令 start_recording:
   a. 生成后台线程
   b. 在后台线程中:
      i.   枚举音频设备, 选择用户指定的设备
      ii.  启动 cpal 输入流 (16kHz, mono, f32)
      iii. 音频回调: f32采样 → 线性重采样到16kHz → i16 PCM → 6400字节块
      iv.  构建Tencent Cloud签名URL (HMAC-SHA1)
      v.   连接 wss://asr.cloud.tencent.com/asr/v2/{appid}?{签名参数}
      vi.  循环: 发送PCM块 → 接收JSON → 解析部分/最终结果
      vii. 发送 {"type":"end"} → 接收最终结果
      viii.通过OSC发送最终结果到 /chatbox/input
   c. 通过 Tauri 事件发射状态更新到前端
4. 用户点击"停止录音" → 设置停止标志 → 线程清理
```

### 0.4 腾讯云 ASR WebSocket 协议流程

```
1. 构建签名: HMAC-SHA1(key, "asr.cloud.tencent.com/asr/v2/{appid}?param1=val1&param2=val2...")
2. 构建URL: wss://asr.cloud.tencent.com/asr/v2/{appid}?[排序参数]&signature=[base64签名]
   必填参数: secretid, timestamp, expired, nonce, engine_model_type=16k_zh, voice_format=1, needvad=1
3. 连接 WebSocket
4. 循环: 发送6400字节二进制PCM块 (200ms @ 16kHz)
5. 接收JSON响应:
   - slice_type=0: 识别开始
   - slice_type=1: 部分结果 (streaming)
   - slice_type=2: 最终结果 (一句话结束)
6. 发送 {"type":"end"} 文本帧
7. 接收最终确认
```

---

## 1. 项目概述

开发一个基于 Tauri v1 的桌面应用程序，用于捕获用户音频，通过腾讯云语音识别 API 进行实时转写，并将结果通过 OSC 协议发送至 VRChat 聊天框。

## 2. 核心功能需求

### 2.1 音频捕获

- 使用 `cpal` 库捕获系统音频输入
- 支持用户选择音频设备 (通过 `list_audio_devices` Tauri 命令)
- 输出格式：16kHz, 16-bit, Mono PCM
- 实时流式传输音频块 (6400字节块 = 200ms)
- 音量RMS计算, 通过 Tauri 事件 `volume-update` 发送到前端

### 2.2 语音识别 (ASR)

- 集成腾讯云流式语音识别 API (WebSocket)
- 端点: `wss://asr.cloud.tencent.com/asr/v2/{appid}`
- 签名算法: V1 HMAC-SHA1 (base64编码)
- 支持实时识别结果 (`slice_type=1`) 和最终结果 (`slice_type=2`)
- 引擎型号: `16k_zh` (中文普通话)

### 2.3 OSC 发送

- 识别完成后通过 OSC 协议发送结果
- 目标地址: `/chatbox/input` (参数: text: string, send: true)
- 打字指示: `/chatbox/typing` (参数: bool)
- OSC 主机和端口: 可配置 (默认 127.0.0.1:9000)

### 2.4 配置管理

- YAML 格式配置文件 (`config.yaml`, 用户要求更易编辑的格式)
- 存储项:
  - `tencent_app_id`, `tencent_secret_id`, `tencent_secret_key`
  - `osc_host`, `osc_port`
- 默认凭据 (从旧项目提取):
  - AppID: `REDACTED_APPID`
  - SecretId: `REDACTED_SECRET_ID`
  - SecretKey: `REDACTED_SECRET_KEY`
  - OSC Host: `127.0.0.1`, Port: `9000`
- 运行时加载 (项目根目录 `config.yaml`)
- 支持序列化保存

## 3. 技术架构

### 3.1 后端 (Rust + Tauri v1)

- **框架**: Tauri v1.x (与旧项目一致, 避免v2迁移问题)
- **音频**: `cpal` 0.15 库
- **网络**: `tokio` + `tokio-tungstenite` 0.21 (WebSocket)
- **OSC**: `rosc` 0.7 库
- **配置**: `serde` + `serde_yaml`
- **签名**: `hmac` 0.12 + `sha1` 0.10 + `base64` 0.21
- **重采样**: 线性插值 (轻量, 无需 `rubato`)
- **错误处理**: `anyhow`

### 3.2 前端 (React 19 + Vite 8)

- **框架**: React 19
- **构建工具**: Vite 8
- **Tauri API**: `@tauri-apps/api` ^1.6
- **TypeScript**: ^6
- **UI**: 单文件组件 (`App.tsx`), 暗色主题 CSS
- **功能**:
  - 音频设备列表下拉选择
  - 开始/停止录音按钮 (带脉冲动画)
  - 实时部分识别结果展示
  - 最终结果展示区域
  - 音量指示条
  - 配置表单 (AppID, SecretId, SecretKey, OSC主机/端口)
  - 状态机: idle → recording → recognizing → done → error

### 3.3 项目结构

```
D:\Projects\vrc-chat-tool\
├── config.yaml              # 运行时配置 (YAML, 用户可编辑)
├── package.json             # Node依赖
├── vite.config.ts           # Vite配置
├── tsconfig.json            # TypeScript配置
├── index.html               # 入口HTML
├── .gitignore               # 忽略 PicoCCB/, node_modules/, target/, .env, *.log, tmp/
├── scripts/
│   └── gen_test_wav.py      # 生成测试用WAV文件
├── tmp/                     # 临时文件 (测试WAV等, 不提交git)
├── src-ui/
│   └── src/
│       ├── main.tsx          # React入口
│       ├── App.tsx           # 主UI组件
│       ├── App.css           # 组件样式
│       └── vite-env.d.ts    # Vite类型声明
├── src-tauri/
│   ├── Cargo.toml           # Rust依赖
│   ├── build.rs             # Tauri构建脚本
│   ├── tauri.conf.json      # Tauri v1配置
│   └── src/
│       ├── main.rs          # Tauri入口和命令定义
│       ├── config.rs        # YAML配置管理
│       ├── audio/
│       │   ├── mod.rs
│       │   └── capture.rs   # 音频捕获和重采样
│       ├── speech/
│       │   ├── mod.rs
│       │   ├── tencent.rs   # HMAC-SHA1签名
│       │   └── streaming.rs # 流式WebSocket ASR客户端
│       └── osc/
│           ├── mod.rs
│           └── sender.rs    # OSC发送器
└── test_rust_api.py         # Python集成测试
```

## 4. API 集成细节

### 4.1 腾讯云流式识别

- **端点**: `wss://asr.cloud.tencent.com/asr/v2/{appid}`
- **签名参数**:
  - `secretid`, `timestamp`, `expired` (当前时间戳+24h), `nonce` (随机数)
  - `engine_model_type`: "16k_zh"
  - `voice_format`: 1 (PCM)
  - `needvad`: 1
- **签名算法**: V1 (HMAC-SHA1)
  ```
  signStr = urlencode("asr.cloud.tencent.com/asr/v2/{appid}?param1=val1&param2=val2&...")
  signature = base64(HMAC-SHA1(secretKey, signStr))
  URL = "wss://asr.cloud.tencent.com/asr/v2/{appid}?params&signature=" + urlencode(signature)
  ```
- **通信流程**:
  1. 构建带签名的 WebSocket URL
  2. 连接 WebSocket
  3. 循环发送音频块 (二进制帧, 6400字节/块, 200ms间隔)
  4. 接收 JSON 响应 (slice_type=0/1/2)
  5. 发送 `{"type": "end"}` 文本帧结束
  6. 接收最终结果

### 4.2 VRChat OSC 接口

- `/chatbox/input`: `,sTF` → [text: string, send: true, sound: false]
- `/chatbox/typing`: `,T` → [is_typing: bool]
- 协议: UDP, 大端字节序, 4字节字符串对齐

## 5. 测试需求

### 5.1 单元测试 (Rust)

每个 Rust 模块包含 `#[cfg(test)]` 单元测试:
- **`config.rs`**: 默认值, YAML序列化/反序列化往返
- **`osc/sender.rs`**: OSC包编码正确性
- **`speech/tencent.rs`**: 签名生成 (已知输入→预期输出)
- **`speech/streaming.rs`**: URL构建验证
- **`audio/capture.rs`**: PCM转换 (f32→i16), 重采样精度, WAV头生成

测试WAV来源:
1. 合成生成 (`scripts/gen_test_wav.py` - 正弦波)
2. Mozilla DeepSpeech smoke test WAV (16kHz mono)

### 5.2 Python 集成测试

- 脚本: `test_rust_api.py`
- 功能:
  - 检查 Rust 二进制编译
  - 验证配置加载/保存
  - 测试音频设备枚举
  - 启动后端进程 (健康检查)
  - 监控事件输出

## 6. 实施计划

遵循 Waves 并行执行模型:

| Wave | 任务 | 并行数 | 依赖 |
|------|------|--------|------|
| 1 | 生成测试WAV + 项目脚手架 | 2 | 无 |
| 2 | Config + OSC + Tencent签名 + 音频捕获 | 4 | Wave 1 |
| 3 | Streaming ASR + 前端UI | 2 | Wave 2 |
| 4 | main.rs Tauri集成 | 1 | Wave 2+3 |
| 5 | 单元测试 + Python集成测试 | 2 | Wave 4 |
| 6 | 最终构建验证 | 1 | Wave 4+5 |

## 7. 部署与运行

### 7.1 开发模式

```bash
# Tauri 开发模式 (自动启动前端+后端)
npm run tauri dev
```

### 7.2 构建发布

```bash
npm run tauri build
```

### 7.3 配置

- 配置文件: 项目根目录 `config.yaml`
- 示例:
  ```yaml
  tencent_app_id: "REDACTED_APPID"
  tencent_secret_id: "REDACTED_SECRET_ID"
  tencent_secret_key: "REDACTED_SECRET_KEY"
  osc_host: "127.0.0.1"
  osc_port: 9000
  ```

## 8. 验收标准

1. [ ] Rust 后端 `cargo build` 编译无错误
2. [ ] `cargo test` 所有单元测试通过
3. [ ] 前端 `npm run build` 构建成功
4. [ ] `npm run tauri dev` 能启动应用窗口
5. [ ] 音频捕获产生正确的 16kHz 16-bit Mono PCM
6. [ ] 腾讯云 ASR 签名生成符合 API 要求
7. [ ] OSC 消息格式正确 (针对 VRChat)
8. [ ] Python 集成测试验证后端功能
9. [ ] YAML 配置加载/保存正确定义
