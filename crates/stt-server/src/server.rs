//! WebSocket server for streaming STT.
//!
//! Accepts float32 PCM audio chunks via WebSocket and returns recognized text
//! as JSON messages. Each client connection gets its own stream session.
//!
//! Supports two backends:
//! - `SherpaOnnx`: traditional transducer-based OnlineRecognizer
//! - `Hybrid`: Zipformer CTC streaming + SenseVoice offline refinement

use crate::config::Config;
use crate::engine::SttEngine;
use crate::hybrid::{HybridEngine, HybridStream};
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

/// Backend engine variant.
pub enum Backend {
    /// Traditional Sherpa-ONNX transducer engine.
    SherpaOnnx(Arc<SttEngine>),
    /// Hybrid Zipformer CTC + SenseVoice engine.
    Hybrid(Arc<HybridEngine>),
}

/// The STT WebSocket server.
pub struct SttServer {
    config: Config,
    backend: Backend,
}

impl SttServer {
    /// Create a new server from configuration.
    ///
    /// Validates model paths and initializes the engine based on
    /// `config.asr.backend` ("sherpa-onnx" or "hybrid").
    pub fn new(config: Config) -> Result<Self> {
        config.validate_model_paths()?;
        let backend = if config.asr.backend == "hybrid" {
            tracing::info!("Using hybrid backend (Zipformer + SenseVoice)");
            let engine = Arc::new(HybridEngine::new(&config)?);
            Backend::Hybrid(engine)
        } else {
            tracing::info!("Using sherpa-onnx backend (transducer)");
            let engine = Arc::new(SttEngine::new(&config)?);
            Backend::SherpaOnnx(engine)
        };
        Ok(Self { config, backend })
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
                    let permit = Arc::clone(&semaphore).acquire_owned().await?;

                    tracing::info!("Client {} connected from {}", cid, addr);

                    let backend = match &self.backend {
                        Backend::SherpaOnnx(e) => Backend::SherpaOnnx(Arc::clone(e)),
                        Backend::Hybrid(e) => Backend::Hybrid(Arc::clone(e)),
                    };

                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Err(e) = handle_client(stream, backend, cid).await {
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

/// Dispatch to the appropriate handler based on the backend variant.
async fn handle_client(
    stream: TcpStream,
    backend: Backend,
    client_id: u64,
) -> Result<()> {
    match backend {
        Backend::SherpaOnnx(engine) => {
            handle_client_sherpa(stream, engine, client_id).await
        }
        Backend::Hybrid(engine) => {
            handle_client_hybrid(stream, engine, client_id).await
        }
    }
}

// ---------------------------------------------------------------------------
// Sherpa-ONNX (traditional) handler
// ---------------------------------------------------------------------------

/// Handle a single WebSocket client connection with the traditional engine.
async fn handle_client_sherpa(
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
                        if data.len() < 4 || data.len() % 4 != 0 {
                            continue;
                        }

                        let num_samples = data.len() / 4;
                        let mut samples = Vec::with_capacity(num_samples);

                        for chunk in data.chunks_exact(4) {
                            let bytes: [u8; 4] = chunk.try_into().unwrap();
                            samples.push(f32::from_le_bytes(bytes));
                        }

                        if samples.is_empty() {
                            continue;
                        }

                        engine.decode(&recog_stream, &samples);

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
                        flush_and_close_sherpa(
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

/// Flush remaining audio with silence, send final result, and close (sherpa-onnx).
async fn flush_and_close_sherpa(
    engine: &SttEngine,
    stream: &sherpa_onnx::OnlineStream,
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >,
    client_id: u64,
    segment: &mut u32,
) {
    let silence_len = (engine.sample_rate() as f32 * 0.3) as usize;
    let silence = vec![0.0f32; silence_len];
    engine.decode(stream, &silence);
    engine.input_finished(stream);

    while engine.get_text(stream).is_some() {
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

// ---------------------------------------------------------------------------
// Hybrid engine handler
// ---------------------------------------------------------------------------

/// Handle a single WebSocket client connection with the hybrid engine.
async fn handle_client_hybrid(
    stream: TcpStream,
    engine: Arc<HybridEngine>,
    client_id: u64,
) -> Result<()> {
    let ws_stream = accept_async(stream).await?;
    let (mut write, mut read) = ws_stream.split();

    let mut hb_stream = engine.create_stream();
    let mut segment: u32 = 0;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if data.len() < 4 || data.len() % 4 != 0 {
                            continue;
                        }

                        let num_samples = data.len() / 4;
                        let mut samples = Vec::with_capacity(num_samples);

                        for chunk in data.chunks_exact(4) {
                            let bytes: [u8; 4] = chunk.try_into().unwrap();
                            samples.push(f32::from_le_bytes(bytes));
                        }

                        if samples.is_empty() {
                            continue;
                        }

                        // Feed to hybrid engine
                        engine.decode(&mut hb_stream, &samples);

                        // Check results
                        if engine.is_endpoint(&hb_stream) {
                            // If refinement is in progress, run SenseVoice
                            if hb_stream.refining {
                                engine.refine(&mut hb_stream);
                            }

                            let text = engine.get_text(&hb_stream);
                            let final_text = engine.add_punctuation(&text);

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
                            hb_stream.last_partial.clear();
                            engine.reset(&mut hb_stream);
                        } else {
                            let text = engine.get_text(&hb_stream);
                            let t = text.trim().to_string();
                            if !t.is_empty() && t != hb_stream.last_partial {
                                let resp = SttResponse {
                                    text: t.clone(),
                                    is_final: false,
                                    segment,
                                };
                                let json = serde_json::to_string(&resp)?;
                                write.send(Message::Text(json.into())).await?;
                                hb_stream.last_partial = t;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        flush_and_close_hybrid(
                            &engine, &mut hb_stream, &mut write,
                            client_id, &mut segment,
                        ).await;
                        return Ok(());
                    }
                    Some(Ok(Message::Text(_))) => {
                        // Ignore text messages
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

/// Flush remaining audio, refine if needed, send final result, and close (hybrid).
async fn flush_and_close_hybrid(
    engine: &HybridEngine,
    hb_stream: &mut HybridStream,
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
    engine.decode(hb_stream, &silence);

    // If refinement is in progress, run it now
    if hb_stream.refining {
        engine.refine(hb_stream);
    }

    let text = engine.get_text(hb_stream);
    if !text.trim().is_empty() {
        let final_text = engine.add_punctuation(&text);
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

    engine.reset(hb_stream);
    let _ = write.close().await;
}
