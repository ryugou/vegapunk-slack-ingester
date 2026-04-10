use vegapunk_slack_ingester::config::Config;

#[test]
fn test_config_from_env_all_required() {
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("VEGAPUNK_AUTH_TOKEN", "test-token");
    std::env::remove_var("VEGAPUNK_GRPC_ENDPOINT");
    std::env::remove_var("INGEST_BATCH_SIZE");
    std::env::remove_var("SORT_BUFFER_WINDOW_SECS");
    std::env::remove_var("SLACK_WATCH_CHANNEL_IDS");

    let config = Config::from_env().unwrap();

    assert_eq!(config.slack_bot_token, "xoxb-test");
    assert_eq!(config.slack_app_token, "xapp-test");
    assert_eq!(config.vegapunk_auth_token, "test-token");
    assert_eq!(config.vegapunk_grpc_endpoint, "host.docker.internal:6840");
    assert_eq!(config.ingest_batch_size, 20);
    assert_eq!(config.sort_buffer_window_secs, 5);

    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");
}

#[test]
fn test_config_missing_required() {
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");

    let result = Config::from_env();
    assert!(result.is_err());
}

#[test]
fn test_config_custom_values() {
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
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

    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("VEGAPUNK_AUTH_TOKEN");
    std::env::remove_var("VEGAPUNK_GRPC_ENDPOINT");
    std::env::remove_var("INGEST_BATCH_SIZE");
    std::env::remove_var("SORT_BUFFER_WINDOW_SECS");
    std::env::remove_var("SLACK_WATCH_CHANNEL_IDS");
}
