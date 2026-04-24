use uuid::Uuid;

use config_control_plane::realtime::SubscriptionFilter;
use config_transport::websocket::RealtimeMessage;

fn make_message(env: &str, host_id: Uuid, path: &str, severity: &str) -> RealtimeMessage {
    RealtimeMessage {
        event_id: Uuid::new_v4(),
        host_id,
        environment: env.to_string(),
        path: path.to_string(),
        event_kind: "modified".to_string(),
        event_time: chrono::Utc::now().to_rfc3339(),
        severity: severity.to_string(),
        author_display: None,
        summary: None,
        diff_render: None,
        pr_url: None,
        pr_number: None,
    }
}

#[test]
fn filter_allows_when_no_filters_set() {
    let filter = SubscriptionFilter::default();
    let msg = make_message("prod", Uuid::new_v4(), "/etc/app.yaml", "info");
    assert!(filter.matches(&msg));
}

#[test]
fn filter_matches_environment() {
    let filter = SubscriptionFilter {
        environment: Some("prod".to_string()),
        ..Default::default()
    };

    let matching_msg = make_message("prod", Uuid::new_v4(), "/etc/app.yaml", "info");
    let non_matching_msg = make_message("dev", Uuid::new_v4(), "/etc/app.yaml", "info");

    assert!(filter.matches(&matching_msg));
    assert!(!filter.matches(&non_matching_msg));
}

#[test]
fn filter_matches_host_id() {
    let host_id = Uuid::new_v4();
    let filter = SubscriptionFilter {
        host_id: Some(host_id),
        ..Default::default()
    };

    let matching_msg = make_message("prod", host_id, "/etc/app.yaml", "info");
    let non_matching_msg = make_message("prod", Uuid::new_v4(), "/etc/app.yaml", "info");

    assert!(filter.matches(&matching_msg));
    assert!(!filter.matches(&non_matching_msg));
}

#[test]
fn filter_matches_path_prefix() {
    let filter = SubscriptionFilter {
        path_prefix: Some("/etc/myapp".to_string()),
        ..Default::default()
    };

    let matching_msg = make_message("prod", Uuid::new_v4(), "/etc/myapp/config.yaml", "info");
    let non_matching_msg = make_message("prod", Uuid::new_v4(), "/opt/other/config.yaml", "info");

    assert!(filter.matches(&matching_msg));
    assert!(!filter.matches(&non_matching_msg));
}

#[test]
fn filter_matches_severity() {
    let filter = SubscriptionFilter {
        severity: Some("warning".to_string()),
        ..Default::default()
    };

    let matching_msg = make_message("prod", Uuid::new_v4(), "/etc/app.yaml", "warning");
    let non_matching_msg = make_message("prod", Uuid::new_v4(), "/etc/app.yaml", "info");

    assert!(filter.matches(&matching_msg));
    assert!(!filter.matches(&non_matching_msg));
}

#[test]
fn filter_combines_all_fields_with_and_logic() {
    let host_id = Uuid::new_v4();
    let filter = SubscriptionFilter {
        environment: Some("prod".to_string()),
        host_id: Some(host_id),
        path_prefix: Some("/etc/myapp".to_string()),
        severity: Some("warning".to_string()),
    };

    let matching_msg = make_message("prod", host_id, "/etc/myapp/config.yaml", "warning");
    assert!(filter.matches(&matching_msg));

    // Wrong environment
    let wrong_env = make_message("staging", host_id, "/etc/myapp/config.yaml", "warning");
    assert!(!filter.matches(&wrong_env));

    // Wrong host
    let wrong_host = make_message("prod", Uuid::new_v4(), "/etc/myapp/config.yaml", "warning");
    assert!(!filter.matches(&wrong_host));

    // Wrong path
    let wrong_path = make_message("prod", host_id, "/opt/other/config.yaml", "warning");
    assert!(!filter.matches(&wrong_path));

    // Wrong severity
    let wrong_sev = make_message("prod", host_id, "/etc/myapp/config.yaml", "info");
    assert!(!filter.matches(&wrong_sev));
}

#[test]
fn filter_partial_match_still_rejects() {
    let filter = SubscriptionFilter {
        environment: Some("prod".to_string()),
        severity: Some("warning".to_string()),
        ..Default::default()
    };

    // Matches environment but not severity
    let msg = make_message("prod", Uuid::new_v4(), "/etc/app.yaml", "info");
    assert!(!filter.matches(&msg));
}
