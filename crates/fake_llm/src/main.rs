//! Fake LLM Server — binary entry point.
//!
//! An independent HTTP test server that supports both OpenAI and Anthropic
//! protocol endpoints. Used as a black-box replacement for real LLM providers
//! during integration and E2E testing.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// Fake LLM Server for integration testing.
#[derive(Parser, Debug)]
#[command(
    name = "fake-llm-server",
    about = "Fake LLM Server for CloseClaw testing"
)]
struct Args {
    /// Address to bind to (e.g. "127.0.0.1:0").
    #[arg(long, default_value = "127.0.0.1:0")]
    addr: SocketAddr,

    /// Directory containing scenario JSON files.
    /// If not provided, the server uses default placeholder responses.
    #[arg(long)]
    scenarios_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    tracing::info!("Starting Fake LLM Server on {}", args.addr);

    closeclaw_fake_llm::server::start_server(&args.addr.to_string(), args.scenarios_dir.as_deref())
        .await
}
