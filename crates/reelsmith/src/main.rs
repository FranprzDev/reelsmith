//! `reelsmith`: an MCP server (stdio) for config-driven short-form promo videos, on top of the `promo` template.
//!
//! The host (Claude Desktop, Codex, OpenCode or any MCP client) starts the binary and talks JSON-RPC over stdin /
//! stdout; logs go to stderr. The render re-runs this same binary as its per-chunk fframes worker
//! (`promo::render::PASSTHROUGH`), so there is a single executable to install.
mod brief;
mod clips;
mod jobs;
mod sample;
mod server;

use rmcp::ServiceExt;
use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::var_os(promo::render::PASSTHROUGH).is_some() {
        return match promo::render::passthrough() {
            Ok(code) => code,
            Err(e) => {
                eprintln!("error: {e:#}");
                ExitCode::FAILURE
            }
        };
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: starting the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> anyhow::Result<()> {
    if let Err(e) = clips::ensure_ffmpeg() {
        eprintln!("warning: {e:#}");
    }
    let service = server::Reelsmith::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
