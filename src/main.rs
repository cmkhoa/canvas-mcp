use anyhow::Context;
use canvas_mcp::{CanvasClient, CanvasServer, Config, canvas::models::Profile};
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

/// MCP server exposing your Canvas LMS data over stdio.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Check CANVAS_API_URL and CANVAS_API_TOKEN against Canvas, then exit.
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdout carries MCP JSON-RPC, so all logging goes to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_env("CANVAS_MCP_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let config = Config::from_env().context("loading configuration")?;
    let client = CanvasClient::new(&config)?;

    if args.check {
        let me: Profile = client
            .get("users/self/profile", &[])
            .await
            .context("contacting Canvas")?;
        eprintln!(
            "OK: signed in to {} as {} (id {})",
            config.api_url, me.name, me.id
        );
        return Ok(());
    }

    tracing::info!(url = %config.api_url, "starting canvas-mcp on stdio");
    let service = CanvasServer::new(client)
        .with_timezone(config.timezone)
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
