use std::sync::Mutex;
use vegapunk_slack_ingester::config::Config;

// Environment variables are process-global state. These tests must not run
// concurrently with each other, so we serialize them with a mutex.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_all_config_vars() {
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_USER_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");
    std::env::remove_var("VEGAPUNK_GRPC_ENDPOINT");
    std::env::remove_var("INGEST_BATCH_SIZE");
    std::env::remove_var("SORT_BUFFER_WINDOW_SECS");
    std::env::remove_var("SLACK_WATCH_CHANNEL_IDS");
    std::env::remove_var("SLACK_ALERT_CHANNEL_ID");
    std::env::remove_var("CURSOR_FILE_PATH");
}

#[test]
fn test_config_from_env_all_required() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_all_config_vars();

    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_USER_TOKEN", "xoxp-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("VEGAPUNK_AUTH_TOKEN", "test-token");

    let config = Config::from_env().unwrap();

    assert_eq!(config.slack_bot_token, "xoxb-test");
    assert_eq!(config.slack_user_token, "xoxp-test");
    assert_eq!(config.slack_app_token, "xapp-test");
    assert_eq!(config.vegapunk_auth_token, "test-token");
    assert_eq!(config.vegapunk_grpc_endpoint, "host.docker.internal:6840");
    assert_eq!(config.ingest_batch_size, 20);
    assert_eq!(config.sort_buffer_window_secs, 5);

    clear_all_config_vars();
}

#[test]
fn test_config_missing_required() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_all_config_vars();

    let result = Config::from_env();
    assert!(result.is_err());
}

#[test]
fn test_config_custom_values() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_all_config_vars();

    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_USER_TOKEN", "xoxp-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("VEGAPUNK_AUTH_TOKEN", "test-token");
    std::env::set_var("VEGAPUNK_GRPC_ENDPOINT", "localhost:6840");
    std::env::set_var("INGEST_BATCH_SIZE", "50");
    std::env::set_var("SORT_BUFFER_WINDOW_SECS", "10");
    std::env::set_var("SLACK_WATCH_CHANNEL_IDS", "C123,C456");

    let config = Config::from_env().unwrap();

    assert_eq!(config.vegapunk_grpc_endpoint, "localhost:6840");
    assert_eq!(config.ingest_batch_size, 50);
    assert_eq!(config.sort_buffer_window_secs, 10);
    assert_eq!(config.slack_watch_channel_ids, vec!["C123", "C456"]);

    clear_all_config_vars();
}
