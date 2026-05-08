/// Unified streaming ASR recognizer — compatibility layer between Tencent Cloud, Local WebSocket, and Local Embedded.
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub enum Recognizer {
    Tencent(crate::speech::streaming::StreamingRecognizer),
    Local(crate::speech::local::LocalRecognizer),
    LocalEmbedded(crate::speech::local_embedded::LocalEmbeddedRecognizer),
    LocalEmbeddedHybrid(crate::speech::local_embedded::LocalEmbeddedHybridRecognizer),
}

impl Recognizer {
    pub async fn recognize_pcm_stream(
        &self,
        pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        stop_signal: Arc<AtomicBool>,
        on_partial: impl Fn(&str) + Send + 'static,
        on_sentence: impl Fn(&str) + Send + 'static,
    ) -> anyhow::Result<String> {
        match self {
            Recognizer::Tencent(r) => {
                r.recognize_pcm_stream(pcm_rx, stop_signal, 16000, on_partial, on_sentence).await
            }
            Recognizer::Local(r) => {
                r.recognize_pcm_stream(pcm_rx, stop_signal, on_partial, on_sentence).await
            }
            Recognizer::LocalEmbedded(r) => {
                r.recognize_pcm_stream(pcm_rx, stop_signal, on_partial, on_sentence).await
            }
            Recognizer::LocalEmbeddedHybrid(r) => {
                r.recognize_pcm_stream(pcm_rx, stop_signal, on_partial, on_sentence).await
            }
        }
    }
}
