pub mod alert;
pub mod buffer;
pub mod cache;
pub mod catchup;
pub mod config;
pub mod converter;
pub mod cursor;
pub mod extractor;
pub mod import;
pub mod slack;
pub mod vegapunk;

/// Vegapunk schema name for this ingester.
pub const SCHEMA_NAME: &str = "slack-ingester";
