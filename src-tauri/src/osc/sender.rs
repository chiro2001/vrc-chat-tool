use std::net::UdpSocket;
use rosc::{OscMessage, OscPacket, OscType};

pub struct OscSender {
    host: String,
    port: u16,
}

impl OscSender {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub fn send_chatbox(&self, text: &str) -> anyhow::Result<()> {
        let msg = OscMessage {
            addr: "/chatbox/input".to_string(),
            args: vec![
                OscType::String(text.to_string()),
                OscType::Bool(true),  // send (press Enter)
                OscType::Bool(false), // sound notification
            ],
        };

        let packet = OscPacket::Message(msg);
        let encoded_bytes = rosc::encoder::encode(&packet)?;

        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(format!("{}:{}", self.host, self.port))?;
        socket.send(&encoded_bytes)?;

        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_chatbox_encoding() {
        let text = "Hello VRChat!";
        let msg = OscMessage {
            addr: "/chatbox/input".to_string(),
            args: vec![
                OscType::String(text.to_string()),
                OscType::Bool(true),
                OscType::Bool(false),
            ],
        };

        let packet = OscPacket::Message(msg);
        let encoded_bytes = rosc::encoder::encode(&packet).unwrap();

        let (_, decoded) = rosc::decoder::decode_udp(&encoded_bytes).unwrap();

        match decoded {
            OscPacket::Message(decoded_msg) => {
                assert_eq!(decoded_msg.addr, "/chatbox/input");
                assert_eq!(decoded_msg.args.len(), 3);

                match &decoded_msg.args[0] {
                    OscType::String(s) => assert_eq!(s, "Hello VRChat!"),
                    _ => panic!("Expected String type"),
                }
                match &decoded_msg.args[1] {
                    OscType::Bool(b) => assert!(b),
                    _ => panic!("Expected Bool type"),
                }
                match &decoded_msg.args[2] {
                    OscType::Bool(b) => assert!(!b),
                    _ => panic!("Expected Bool type"),
                }
            }
            _ => panic!("Expected OscPacket::Message"),
        }
    }

    #[test]
    fn test_send_typing_encoding() {
        let msg = OscMessage {
            addr: "/chatbox/typing".to_string(),
            args: vec![OscType::Bool(true)],
        };

        let packet = OscPacket::Message(msg);
        let encoded_bytes = rosc::encoder::encode(&packet).unwrap();

        let (_, decoded) = rosc::decoder::decode_udp(&encoded_bytes).unwrap();

        match decoded {
            OscPacket::Message(decoded_msg) => {
                assert_eq!(decoded_msg.addr, "/chatbox/typing");
                assert_eq!(decoded_msg.args.len(), 1);
                match &decoded_msg.args[0] {
                    OscType::Bool(b) => assert!(b),
                    _ => panic!("Expected Bool type"),
                }
            }
            _ => panic!("Expected OscPacket::Message"),
        }
    }
}
