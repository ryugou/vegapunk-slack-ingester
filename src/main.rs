use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use vegapunk_slack_ingester::catchup;
use vegapunk_slack_ingester::config;
use vegapunk_slack_ingester::cursor;
use vegapunk_slack_ingester::import;
use vegapunk_slack_ingester::slack;
use vegapunk_slack_ingester::vegapunk;

#[derive(Parser)]
#[command(name = "vegapunk-slack-ingester")]
#[command(about = "Ingest Slack messages into Vegapunk")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ingester daemon (catchup + realtime sync)
    Serve,
    /// Import historical messages for a channel
    Import {
        /// Slack channel ID to import
        #[arg(long)]
        channel: String,
        /// Start date for import (YYYY-MM-DD)
        #[arg(long)]
        since: String,
    },
    /// Health check
    Health,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve => {
            let config = config::Config::from_env()?;

            let mut vegapunk_client = vegapunk::VegapunkClient::connect(
                &config.vegapunk_grpc_endpoint,
                &config.vegapunk_auth_token,
            )
            .await?;
            tracing::info!("connected to Vegapunk");

            if !vegapunk_client
                .check_schema_exists(vegapunk_slack_ingester::SCHEMA_NAME)
                .await?
            {
                anyhow::bail!("schema '{}' does not exist. Create it via admin UI before starting the ingester.", vegapunk_slack_ingester::SCHEMA_NAME);
            }
            tracing::info!(
                schema = vegapunk_slack_ingester::SCHEMA_NAME,
                "schema confirmed"
            );

            let mut slack_client = slack::SlackClient::new(&config.slack_user_token, 3600);

            let cursor_store = cursor::CursorStore::new(&config.cursor_file_path);

            let alert = vegapunk_slack_ingester::alert::AlertClient::new(
                &config.slack_bot_token,
                config.slack_alert_channel_id.clone(),
            );

            if let Err(e) = catchup::run_catchup(
                &config,
                &mut slack_client,
                &mut vegapunk_client,
                &cursor_store,
            )
            .await
            {
                tracing::error!(error = %e, "catchup failed");
                alert.send(&format!("Catchup failed: {e:#}")).await?;
                anyhow::bail!("catchup failed: {e:#}");
            }

            slack::socket::run_socket_mode(
                &config,
                &mut slack_client,
                &mut vegapunk_client,
                &cursor_store,
                &alert,
            )
            .await?;

            Ok(())
        }
        Commands::Import { channel, since } => {
            tracing::info!(channel = %channel, since = %since, "starting import mode");
            let config = config::Config::from_env()?;

            let mut vegapunk_client = vegapunk::VegapunkClient::connect(
                &config.vegapunk_grpc_endpoint,
                &config.vegapunk_auth_token,
            )
            .await?;

            if !vegapunk_client
                .check_schema_exists(vegapunk_slack_ingester::SCHEMA_NAME)
                .await?
            {
                anyhow::bail!(
                    "schema '{}' does not exist.",
                    vegapunk_slack_ingester::SCHEMA_NAME
                );
            }

            let mut slack_client = slack::SlackClient::new(&config.slack_user_token, 3600);

            import::run_import(
                &config,
                &mut slack_client,
                &mut vegapunk_client,
                &channel,
                &since,
            )
            .await?;

            Ok(())
        }
        Commands::Health => {
            let config = config::Config::from_env().unwrap_or_else(|e| {
                eprintln!("config error: {e:#}");
                std::process::exit(1);
            });
            let mut client = vegapunk::VegapunkClient::connect(
                &config.vegapunk_grpc_endpoint,
                &config.vegapunk_auth_token,
            )
            .await
            .unwrap_or_else(|e| {
                eprintln!("Vegapunk connection failed: {e:#}");
                std::process::exit(1);
            });
            let exists = client
                .check_schema_exists(vegapunk_slack_ingester::SCHEMA_NAME)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Vegapunk check failed: {e:#}");
                    std::process::exit(1);
                });
            if !exists {
                eprintln!(
                    "schema '{}' does not exist",
                    vegapunk_slack_ingester::SCHEMA_NAME
                );
                std::process::exit(1);
            }
            println!("OK");
            Ok(())
        }
    }
}
