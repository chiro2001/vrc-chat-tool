//! Standalone STT server binary entry point.
//!
//! Usage:
//!   stt-server run                  Start the WebSocket server
//!   stt-server download             Download ASR and punctuation models
//!   stt-server download --force     Force re-download models

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "stt-server", about = "Streaming Speech-to-Text Server", version)]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "config.yaml")]
    config: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the WebSocket STT server (default)
    Run,
    /// Download ASR and punctuation models
    Download {
        /// Force re-download even if models exist
        #[arg(long)]
        force: bool,
        /// Skip punctuation model
        #[arg(long)]
        no_punct: bool,
    },
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = stt_server::Config::from_file(&cli.config)?;

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            let backend_name = if config.asr.backend == "hybrid" {
                format!("Hybrid ({})", config.asr.streaming_model)
            } else {
                "Sherpa-ONNX (transducer)".into()
            };

            tracing::info!("{}", "=".repeat(58));
            tracing::info!("  Streaming STT Server");
            tracing::info!("  Backend:            {}", backend_name);
            tracing::info!(
                "  Host:               {}",
                config.server.host
            );
            tracing::info!("  Port:               {}", config.server.port);
            tracing::info!(
                "  Max connections:    {}",
                config.server.max_connections
            );
            tracing::info!("  Model:              {}", config.asr.model_name);
            tracing::info!(
                "  Threads:            {}",
                config.asr.num_threads
            );
            tracing::info!(
                "  Sample rate:        {}",
                config.asr.sample_rate
            );
            tracing::info!(
                "  Provider:           {}",
                config.asr.provider.as_deref().unwrap_or("cpu")
            );
            tracing::info!(
                "  Punctuation:        {}",
                if config.punctuation.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            tracing::info!("{}", "=".repeat(58));

            // Validate model paths before starting
            config.validate_model_paths()?;

            let server = stt_server::SttServer::new(config)?;

            // Create tokio runtime for the actix-web server
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async { server.run().await })?;
        }
        Commands::Download { force, no_punct } => {
            if !stt_server::download::check_network_connectivity() {
                anyhow::bail!(
                    "No network connectivity detected. Set HTTP_PROXY / HTTPS_PROXY if behind a firewall."
                );
            }

            tracing::info!("{}", "=".repeat(60));
            tracing::info!("ASR Model: {}", config.asr.model_name);
            tracing::info!("{}", "=".repeat(60));

            stt_server::download_models(&config, force, no_punct)?;

            tracing::info!("{}", "=".repeat(60));
            tracing::info!("Done. All models downloaded and verified successfully.");
            tracing::info!(
                "Model directory: {}",
                config.asr.model_dir.display()
            );
            tracing::info!("{}", "=".repeat(60));
        }
    }

    Ok(())
}
