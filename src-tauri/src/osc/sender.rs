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
            buffer: Mutex::new(Vec::new()),
            last_sent: Mutex::new(String::new()),
        }
    }

    pub fn with_config(host: String, port: u16, line_count: usize, retention_secs: u64) -> Self {
        Self {
            host,
            port,
            line_count: line_count.max(1),
            retention_secs,
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

    /// Send a final (complete) sentence — adds to buffer and sends combined text
    pub fn send_chatbox(&self, text: &str) -> anyhow::Result<()> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }

        let now = Self::now_ms();
        let combined = {
            let mut buf = self.buffer.lock().unwrap();
            buf.push(OutputMessage {
                text: text.clone(),
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
}
