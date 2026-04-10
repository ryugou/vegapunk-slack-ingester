use anyhow::Result;
use tracing::info;

use crate::buffer::SortBuffer;
use crate::config::Config;
use crate::cursor::CursorStore;
use crate::slack::SlackClient;
use crate::vegapunk::VegapunkClient;

/// Run Socket Mode event loop.
/// Blocks until the process receives SIGINT.
///
/// # Panics
///
/// Does not panic — ctrl_c() errors propagate as `Result`.
pub async fn run_socket_mode(
    _config: &Config,
    _slack: &mut SlackClient,
    _vegapunk: &mut VegapunkClient,
    _cursor_store: &CursorStore,
) -> Result<()> {
    // Keep the buffer type visible for future integration
    let _buffer: SortBuffer = SortBuffer::new();

    info!("socket mode ready — waiting for SIGINT");
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");
    Ok(())
}
