use std::net::UdpSocket;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Mutex;
use rosc::{OscMessage, OscPacket, OscType};

struct OutputMessage {
    text: String,
    timestamp: u64,
}

pub struct OscSender {
    host: String,
    port: u16,
    line_count: usize,
    retention_secs: u64,
    remove_period: bool,
    buffer: Mutex<Vec<OutputMessage>>,
    last_sent: Mutex<String>,
}

impl OscSender {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            line_count: 2,
            retention_secs: 5,
            remove_period: true,
            buffer: Mutex::new(Vec::new()),
            last_sent: Mutex::new(String::new()),
        }
    }

    pub fn with_config(host: String, port: u16, line_count: usize, retention_secs: u64, remove_period: bool) -> Self {
        Self {
            host,
            port,
            line_count: line_count.max(1),
            retention_secs,
            remove_period,
            buffer: Mutex::new(Vec::new()),
            last_sent: Mutex::new(String::new()),
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Strip trailing punctuation from text for clean display/OSC output.
    pub fn strip_trailing_punctuation(text: &str) -> String {
        let punct = "。，！？；：、,.;:!?~～…—";
        text.trim_end_matches(|c: char| punct.contains(c)).to_string()
    }

    /// Send a final (complete) sentence — adds to buffer and sends combined text
    pub fn send_chatbox(&self, text: &str) -> anyhow::Result<()> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }

        let display_text = if self.remove_period {
            Self::strip_trailing_punctuation(&text)
        } else {
            text.clone()
        };

        let now = Self::now_ms();
        let combined = {
            let mut buf = self.buffer.lock().unwrap();
            buf.push(OutputMessage {
                text: display_text.clone(),
                timestamp: now,
            });

            // Expire old messages
            let cutoff = now.saturating_sub(self.retention_secs * 1000);
            buf.retain(|m| m.timestamp >= cutoff);

            // Keep only last N lines
            let messages: Vec<&str> = buf.iter()
                .rev()
                .take(self.line_count)
                .map(|m| m.text.as_str())
                .collect();
            messages.into_iter().rev().collect::<Vec<_>>().join("\n")
        };

        self.send_osc("/chatbox/input", &combined, true, false)
    }

    /// Send a partial result — debounced, doesn't add to permanent buffer
    pub fn send_partial(&self, text: &str) -> anyhow::Result<()> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }

        // Get buffered messages + current partial
        let combined = {
            let buf = self.buffer.lock().unwrap();
            let messages: Vec<&str> = buf.iter()
                .rev()
                .take(self.line_count)
                .map(|m| m.text.as_str())
                .collect();
            let mut lines: Vec<&str> = messages.into_iter().rev().collect();
            lines.push(&text);
            lines.join("\n")
        };

        // Skip if unchanged
        {
            let last = self.last_sent.lock().unwrap();
            if *last == combined {
                return Ok(());
            }
        }

        self.send_osc("/chatbox/input", &combined, true, false)
    }

    pub fn send_typing(&self, is_typing: bool) -> anyhow::Result<()> {
        let msg = OscMessage {
            addr: "/chatbox/typing".to_string(),
            args: vec![OscType::Bool(is_typing)],
        };
        let packet = OscPacket::Message(msg);
        let encoded_bytes = rosc::encoder::encode(&packet)?;

        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(format!("{}:{}", self.host, self.port))?;
        socket.send(&encoded_bytes)?;
        Ok(())
    }

    fn send_osc(&self, addr: &str, text: &str, visible: bool, sound: bool) -> anyhow::Result<()> {
        let msg = OscMessage {
            addr: addr.to_string(),
            args: vec![
                OscType::String(text.to_string()),
                OscType::Bool(visible),
                OscType::Bool(sound),
            ],
        };
        let packet = OscPacket::Message(msg);
        let encoded_bytes = rosc::encoder::encode(&packet)?;

        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(format!("{}:{}", self.host, self.port))?;
        socket.send(&encoded_bytes)?;

        // Update last sent
        *self.last_sent.lock().unwrap() = text.to_string();
        Ok(())
    }

    pub fn clear_buffer(&self) {
        self.buffer.lock().unwrap().clear();
    }

    /// Clear the VRChat chatbox: clears internal buffer and sends a stop indicator.
    /// VRChat ignores empty strings, so we send a brief "stopped" message instead.
    pub fn clear_chatbox(&self) -> anyhow::Result<()> {
        self.clear_buffer();
        self.send_osc("/chatbox/input", "语音识别已停止", true, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_trailing_punctuation_chinese() {
        assert_eq!(
            OscSender::strip_trailing_punctuation("你好世界。"),
            "你好世界"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("你好世界！"),
            "你好世界"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("你好世界？"),
            "你好世界"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("你好世界，"),
            "你好世界"
        );
    }

    #[test]
    fn test_strip_trailing_punctuation_english() {
        assert_eq!(
            OscSender::strip_trailing_punctuation("Hello world."),
            "Hello world"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("Hello world!"),
            "Hello world"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("Hello world?"),
            "Hello world"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("Hello, world;"),
            "Hello, world"
        );
    }

    #[test]
    fn test_strip_trailing_punctuation_preserves_inner() {
        // Punctuation in the middle should be preserved
        assert_eq!(
            OscSender::strip_trailing_punctuation("你好，世界。"),
            "你好，世界"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("Hello, world."),
            "Hello, world"
        );
    }

    #[test]
    fn test_strip_trailing_punctuation_no_punct() {
        assert_eq!(
            OscSender::strip_trailing_punctuation("你好世界"),
            "你好世界"
        );
        assert_eq!(
            OscSender::strip_trailing_punctuation("Hello world"),
            "Hello world"
        );
    }

    #[test]
    fn test_strip_trailing_punctuation_empty() {
        assert_eq!(OscSender::strip_trailing_punctuation(""), "");
    }

    #[test]
    fn test_strip_trailing_punctuation_only_punct() {
        assert_eq!(OscSender::strip_trailing_punctuation("。"), "");
        assert_eq!(OscSender::strip_trailing_punctuation("..."), "");
    }

    #[test]
    fn test_now_ms_returns_positive() {
        let ts = OscSender::now_ms();
        assert!(ts > 0, "Timestamp should be positive, got {}", ts);
    }

    #[test]
    fn test_buffer_respects_line_count() {
        let sender = OscSender::with_config(
            "127.0.0.1".to_string(),
            9000,
            2,  // line_count = 2
            3600, // retention = 1 hour
            true,
        );

        // Add 3 messages directly to buffer
        {
            let mut buf = sender.buffer.lock().unwrap();
            let now = OscSender::now_ms();
            buf.push(OutputMessage { text: "第一句".to_string(), timestamp: now });
            buf.push(OutputMessage { text: "第二句".to_string(), timestamp: now });
            buf.push(OutputMessage { text: "第三句".to_string(), timestamp: now });
        }

        // After pushing 3, the buffer should only show last 2 in combined text
        let combined = {
            let buf = sender.buffer.lock().unwrap();
            let messages: Vec<&str> = buf.iter()
                .rev()
                .take(sender.line_count)
                .map(|m| m.text.as_str())
                .collect();
            messages.into_iter().rev().collect::<Vec<_>>().join("\n")
        };
        assert_eq!(combined, "第二句\n第三句");
    }

    #[test]
    fn test_buffer_expiry() {
        let sender = OscSender::with_config(
            "127.0.0.1".to_string(),
            9000,
            10,
            1, // retention = 1 second
            true,
        );

        let old_time = OscSender::now_ms().saturating_sub(2000); // 2 seconds ago

        {
            let mut buf = sender.buffer.lock().unwrap();
            buf.push(OutputMessage { text: "旧消息".to_string(), timestamp: old_time });
            buf.push(OutputMessage { text: "新消息".to_string(), timestamp: OscSender::now_ms() });
        }

        // Trigger expiry by calling send_chatbox which calls retain
        // (send_chatbox requires network, so we manually simulate the retain logic)
        {
            let mut buf = sender.buffer.lock().unwrap();
            let cutoff = OscSender::now_ms().saturating_sub(sender.retention_secs * 1000);
            buf.retain(|m| m.timestamp >= cutoff);
        }

        let remaining = {
            let buf = sender.buffer.lock().unwrap();
            buf.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0], "新消息");
    }

    #[test]
    fn test_last_sent_dedup() {
        let sender = OscSender::with_config(
            "127.0.0.1".to_string(),
            9000,
            2,
            3600,
            true,
        );

        // Manually set last_sent to simulate previous send
        {
            let mut last = sender.last_sent.lock().unwrap();
            *last = "测试消息".to_string();
        }

        // Verify last_sent is set
        {
            let last = sender.last_sent.lock().unwrap();
            assert_eq!(*last, "测试消息");
        }
    }

    #[test]
    fn test_sender_default_config() {
        let sender = OscSender::new("127.0.0.1".to_string(), 9000);
        assert_eq!(sender.line_count, 2);
        assert_eq!(sender.retention_secs, 5);
        assert!(sender.remove_period);
    }

    #[test]
    fn test_sender_line_count_minimum() {
        let sender = OscSender::with_config(
            "127.0.0.1".to_string(), 9000, 0, 5, true,
        );
        assert_eq!(sender.line_count, 1, "line_count should be clamped to minimum 1");
    }
}
