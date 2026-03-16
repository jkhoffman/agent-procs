use agent_procs::daemon::process_manager::is_valid_dns_label;

#[test]
fn test_valid_dns_labels() {
    assert!(is_valid_dns_label("api"));
    assert!(is_valid_dns_label("my-service"));
    assert!(is_valid_dns_label("web123"));
    assert!(is_valid_dns_label("a"));
}

#[test]
fn test_invalid_dns_labels() {
    assert!(!is_valid_dns_label(""));
    assert!(!is_valid_dns_label("-api"));
    assert!(!is_valid_dns_label("api-"));
    assert!(!is_valid_dns_label("my service"));
    assert!(!is_valid_dns_label("API"));
    assert!(!is_valid_dns_label("my_service"));
    assert!(!is_valid_dns_label(&"a".repeat(64)));
}
