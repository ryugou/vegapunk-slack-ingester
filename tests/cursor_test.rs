use vegapunk_slack_ingester::cursor::CursorStore;

#[test]
fn test_load_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.json");

    let store = CursorStore::new(path.to_str().unwrap());
    let cursors = store.load().unwrap();

    assert!(cursors.is_empty());
}

#[test]
fn test_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.json");

    let store = CursorStore::new(path.to_str().unwrap());
    store.save_channel("C123", "1712345678.123456").unwrap();
    store.save_channel("C456", "1712345700.654321").unwrap();

    let cursors = store.load().unwrap();
    assert_eq!(cursors.get("C123"), Some(&"1712345678.123456".to_string()));
    assert_eq!(cursors.get("C456"), Some(&"1712345700.654321".to_string()));
}

#[test]
fn test_update_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cursor.json");

    let store = CursorStore::new(path.to_str().unwrap());
    store.save_channel("C123", "1712345678.000000").unwrap();
    store.save_channel("C123", "1712345700.000000").unwrap();

    let cursors = store.load().unwrap();
    assert_eq!(cursors.get("C123"), Some(&"1712345700.000000".to_string()));
}
