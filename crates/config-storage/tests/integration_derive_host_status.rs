use chrono::{Duration, Utc};
use config_storage::repositories::hosts::derive_host_status;

#[test]
fn no_heartbeat_returns_registering() {
    let result = derive_host_status(None, 30);
    assert_eq!(result, "registering");
}

#[test]
fn recent_heartbeat_returns_healthy() {
    let recent = Utc::now() - Duration::seconds(10);
    let result = derive_host_status(Some(recent), 30);
    assert_eq!(result, "healthy");
}

#[test]
fn stale_heartbeat_returns_degraded() {
    // Between 2x and 5x interval
    let stale = Utc::now() - Duration::seconds(90);
    let result = derive_host_status(Some(stale), 30);
    assert_eq!(result, "degraded");
}

#[test]
fn very_stale_heartbeat_returns_offline() {
    // Beyond 5x interval
    let very_stale = Utc::now() - Duration::seconds(200);
    let result = derive_host_status(Some(very_stale), 30);
    assert_eq!(result, "offline");
}
