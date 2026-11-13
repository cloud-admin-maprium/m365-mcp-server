#![recursion_limit = "256"]
//! Outlook / Microsoft 365 MCP server.
//!
//! Rust port of `@lnwebworks/outlook-mcp-server` (MIT, access-ctrl/outlook-mcp-server).
//! Tool names, descriptions and response shapes are kept identical so existing
//! client configs and skills keep working.
//!
//! stdout is the JSON-RPC channel. All logging goes to stderr — never add a
//! `println!` outside the `smoke` / `print-tools` subcommands.

mod auth;
mod config;
mod graph;
#[cfg(test)]
mod http_tests;
mod out;
mod token;
mod tools;
mod triage;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use serde_json::json;

use crate::config::Config;
use crate::graph::GraphClient;
use crate::tools::McpService;

#[derive(Parser)]
#[command(
    name = "m365-mcp-server",
    version,
    about = "Outlook / Microsoft 365 MCP server over Microsoft Graph"
)]
struct Cli {
    /// Path to config.toml (default: ~/.config/m365-mcp/config.toml, or $M365_CONFIG)
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the MCP server on stdio (default)
    Serve,
    /// Call get_todays_briefing and print the JSON — bypasses the MCP client
    Smoke {
        /// Target mailbox (defaults to the configured user email)
        #[arg(long)]
        mailbox: Option<String>,
    },
    /// Print this server's tools/list JSON, resolved through a real MCP client
    PrintTools,
    /// Sign in once with the device-code flow (delegated, single user)
    Login,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(cli.config.as_deref()).await,
        Command::Smoke { mailbox } => smoke(cli.config.as_deref(), mailbox.as_deref()).await,
        Command::PrintTools => print_tools(cli.config.as_deref()).await,
        Command::Login => login(cli.config.as_deref()).await,
    }
}

async fn serve(config_path: Option<&std::path::Path>) -> Result<()> {
    let config = Arc::new(Config::load(config_path)?);
    tracing::info!("{}", config.summary());
    if !config.is_configured() {
        tracing::warn!(
            "Graph credentials incomplete — tools will return configuration errors until this is fixed"
        );
    }
    if config.token_path.exists() {
        tracing::info!("delegated tokens found at {}", config.token_path.display());
    } else {
        tracing::warn!(
            "no delegated tokens at {} — run `m365-mcp-server login`",
            config.token_path.display()
        );
    }
    if config.source.is_none() {
        tracing::warn!(
            "no config file found; using defaults + environment (expected at ~/.config/m365-mcp/config.toml)"
        );
    }

    let read_only = config.read_only;
    if read_only {
        tracing::info!(
            "read-only mode: {} write tools are hidden and rejected",
            tools::WRITE_TOOLS.len()
        );
    }
    let service = McpService::new(Arc::new(GraphClient::new(config)), read_only)
        .serve(stdio())
        .await
        .context("starting the MCP server on stdio")?;
    service.waiting().await?;
    Ok(())
}

async fn smoke(config_path: Option<&std::path::Path>, mailbox: Option<&str>) -> Result<()> {
    let config = Arc::new(Config::load(config_path)?);
    tracing::info!("{}", config.summary());
    let client = GraphClient::new(config);

    match client.get_todays_briefing(mailbox).await {
        Ok(value) => {
            let failed = value.get("emailError").is_some() || value.get("eventError").is_some();
            println!("{}", out::pretty(&value));
            if failed {
                std::process::exit(1);
            }
            Ok(())
        }
        Err(err) => {
            println!("{}", out::pretty(&json!({ "error": err.to_string() })));
            std::process::exit(1);
        }
    }
}

async fn print_tools(config_path: Option<&std::path::Path>) -> Result<()> {
    let exe = std::env::current_exe().context("resolving the current executable")?;

    let mut command = tokio::process::Command::new(exe);
    if let Some(path) = config_path {
        command.arg("--config").arg(path);
    }

    let transport = rmcp::transport::TokioChildProcess::new(command)
        .context("spawning the server as a child process")?;
    let client = ().serve(transport).await.context("MCP handshake failed")?;
    let tools = client.list_all_tools().await.context("tools/list failed")?;
    println!("{}", out::pretty(&serde_json::to_value(&tools)?));
    let _ = client.cancel().await;
    Ok(())
}

async fn login(config_path: Option<&std::path::Path>) -> Result<()> {
    let config = Config::load(config_path)?;
    if config.tenant_id.is_empty() || config.client_id.is_empty() {
        anyhow::bail!(
            "set tenant_id and client_id in the config (or AZURE_TENANT_ID / AZURE_CLIENT_ID) before logging in"
        );
    }
    tracing::info!("{}", config.summary());
    auth::device_code_login(&config).await?;
    Ok(())
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("M365_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}
