use std::path::PathBuf;

use anyhow::Context;
use canvas_mcp::{CanvasClient, CanvasServer, Config, canvas::models::Profile};
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

/// MCP server exposing your Canvas LMS data over stdio.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Check the Canvas URL and login against Canvas, then exit.
    #[arg(long)]
    check: bool,

    /// Settings file to read. Default: ./.env, then ~/.config/canvas-mcp/.env.
    #[arg(long, env = "CANVAS_MCP_ENV_FILE", value_name = "PATH")]
    env_file: Option<PathBuf>,
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
    let config = Config::from_env(args.env_file.as_deref()).context("loading configuration")?;
    let client = CanvasClient::new(&config)?;

    if args.check {
        let me: Profile = client
            .get("users/self/profile", &[])
            .await
            .context("contacting Canvas")?;
        let how = match config.auth {
            canvas_mcp::Auth::Token(_) => "an API token",
            canvas_mcp::Auth::SessionCookie(_) => "a browser session cookie",
        };
        eprintln!(
            "OK: signed in to {} as {} (id {}) using {how}",
            config.api_url, me.name, me.id
        );
        match &config.settings_file {
            Some(path) => eprintln!("Settings read from {}", path.display()),
            None => eprintln!("Settings read from environment variables (no .env file found)"),
        }
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
