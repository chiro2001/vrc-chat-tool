//! WebSocket server for streaming STT.
//!
//! Accepts float32 PCM audio chunks via WebSocket and returns recognized text
//! as JSON messages. Each client connection gets its own OnlineStream session.

use crate::config::Config;
use crate::engine::SttEngine;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// A recognition result sent back to the client.
///
/// Mirrors the Python server's JSON format:
/// `{"text": "...", "is_final": bool, "segment": N}`
#[derive(Debug, Clone, Serialize)]
pub struct SttResponse {
    pub text: String,
    pub is_final: bool,
    pub segment: u32,
}

/// The STT WebSocket server.
pub struct SttServer {
    config: Config,
    engine: Arc<SttEngine>,
}

impl SttServer {
    /// Create a new server from configuration.
    ///
    /// Initializes the ASR engine and validates model paths.
    pub fn new(config: Config) -> Result<Self> {
        config.validate_model_paths()?;
        let engine = Arc::new(SttEngine::new(&config)?);
        Ok(Self { config, engine })
    }

    /// Start the WebSocket server and block until shutdown.
    pub async fn run(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        let listener = TcpListener::bind(&addr).await?;
        tracing::info!("Listening on ws://{}", addr);
        tracing::info!("Max connections: {}", self.config.server.max_connections);

        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            self.config.server.max_connections,
        ));

        // Graceful shutdown via Ctrl+C
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutdown signal received, stopping server...");
            let _ = shutdown_tx.send(()).await;
        });

        let mut client_id: u64 = 0;

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let (stream, addr) = accept_result?;
                    client_id += 1;
                    let cid = client_id;
                    let engine = Arc::clone(&self.engine);
                    let permit = Arc::clone(&semaphore).acquire_owned().await?;

                    tracing::info!("Client {} connected from {}", cid, addr);

                    tokio::spawn(async move {
                        let _permit = permit; // hold permit until task finishes
                        if let Err(e) = handle_client(stream, engine, cid).await {
                            tracing::error!("Client {} error: {}", cid, e);
                        }
                        tracing::info!("Client {} disconnected", cid);
                    });
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("Server shutting down gracefully.");
                    break;
                }
            }
        }

        Ok(())
    }
}

/// Handle a single WebSocket client connection.
async fn handle_client(
    stream: TcpStream,
    engine: Arc<SttEngine>,
    client_id: u64,
) -> Result<()> {
    let ws_stream = accept_async(stream).await?;
    let (mut write, mut read) = ws_stream.split();

    let recog_stream = engine.create_stream();
    let mut segment: u32 = 0;
    let mut last_partial = String::new();

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        // Parse float32 PCM samples
                        if data.len() < 4 || data.len() % 4 != 0 {
                            continue; // malformed chunk
                        }

                        let num_samples = data.len() / 4;
                        let mut samples = Vec::with_capacity(num_samples);

                        // Safe reinterpret: LE f32 bytes → f32 values
                        for chunk in data.chunks_exact(4) {
                            let bytes: [u8; 4] = chunk.try_into().unwrap();
                            samples.push(f32::from_le_bytes(bytes));
                        }

                        if samples.is_empty() {
                            continue;
                        }

                        // Feed to engine
                        engine.decode(&recog_stream, &samples);

                        // Check results
                        let text = engine.get_text(&recog_stream).unwrap_or_default();

                        if engine.is_endpoint(&recog_stream) {
                            let final_text = engine
                                .add_punctuation(&text)
                                .unwrap_or_else(|| text.clone());

                            if !final_text.trim().is_empty() {
                                let resp = SttResponse {
                                    text: final_text,
                                    is_final: true,
                                    segment,
                                };
                                let json = serde_json::to_string(&resp)?;
                                write.send(Message::Text(json.into())).await?;
                                tracing::info!(
                                    "Client {} segment {} (final): {}",
                                    client_id, segment, resp.text,
                                );
                            }

                            segment += 1;
                            last_partial.clear();
                            engine.reset(&recog_stream);
                        } else {
                            let t = text.trim().to_string();
                            if !t.is_empty() && t != last_partial {
                                let resp = SttResponse {
                                    text: t.clone(),
                                    is_final: false,
                                    segment,
                                };
                                let json = serde_json::to_string(&resp)?;
                                write.send(Message::Text(json.into())).await?;
                                last_partial = t;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        // Connection closed — flush remaining audio
                        flush_and_close(
                            &engine, &recog_stream, &mut write,
                            client_id, &mut segment,
                        ).await;
                        return Ok(());
                    }
                    Some(Ok(Message::Text(_))) => {
                        // Ignore text messages (protocol uses binary only)
                    }
                    Some(Err(e)) => {
                        tracing::error!("Client {} WebSocket error: {}", client_id, e);
                        return Err(e.into());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Flush remaining audio with silence, send final result, and close.
async fn flush_and_close(
    engine: &SttEngine,
    stream: &sherpa_onnx::OnlineStream,
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >,
    client_id: u64,
    segment: &mut u32,
) {
    // Send 300ms of silence to flush the decoder
    let silence_len = (engine.sample_rate() as f32 * 0.3) as usize;
    let silence = vec![0.0f32; silence_len];
    engine.decode(stream, &silence);
    engine.input_finished(stream);

    // Run final decode
    while engine.get_text(stream).is_some() {
        // The sherpa-onnx API handles this internally; we just need to ensure
        // all frames are processed by calling decode one more time.
        // Break when text stabilizes.
        let before = engine.get_text(stream);
        if let Some(text) = before {
            if !text.trim().is_empty() {
                let final_text = engine
                    .add_punctuation(&text)
                    .unwrap_or_else(|| text.clone());

                if !final_text.trim().is_empty() {
                    let resp = SttResponse {
                        text: final_text,
                        is_final: true,
                        segment: *segment,
                    };
                    if let Ok(json) = serde_json::to_string(&resp) {
                        let _ = write.send(Message::Text(json.into())).await;
                        tracing::info!(
                            "Client {} segment {} (flush): {}",
                            client_id, *segment, resp.text,
                        );
                    }
                }
            }
        }
        break;
    }

    engine.reset(stream);
    let _ = write.close().await;
}
