use config_auth::tokens::AgentCredential;
use config_auth::enrollment::EnrollmentVerifier;
use config_auth::policy;

#[test]
fn auth_issue_and_verify_roundtrip() {
    let secret = "test-secret-key";
    let host_id = "host-123";

    let credential = AgentCredential::issue(secret, host_id, chrono::Duration::hours(24));

    // Token should have the format: host_id|expires_at|hmac_hex
    assert!(credential.token.contains('|'));

    let verified = AgentCredential::verify(secret, &credential.token);
    assert!(verified.is_ok());
    assert_eq!(verified.unwrap().host_id, host_id);
}

#[test]
fn auth_expired_token_rejected() {
    let secret = "test-secret-key";
    let host_id = "host-456";

    // Issue a credential that's already expired (negative TTL)
    let credential = AgentCredential::issue(secret, host_id, chrono::Duration::seconds(-1));

    let result = AgentCredential::verify(secret, &credential.token);
    assert!(result.is_err());
}

#[test]
fn auth_wrong_secret_rejected() {
    let secret = "correct-secret";
    let wrong_secret = "wrong-secret";
    let host_id = "host-789";

    let credential = AgentCredential::issue(secret, host_id, chrono::Duration::hours(24));
    let result = AgentCredential::verify(wrong_secret, &credential.token);
    assert!(result.is_err());
}

#[test]
fn auth_malformed_token_rejected() {
    let secret = "test-secret";
    let result = AgentCredential::verify(secret, "not-a-valid-token");
    assert!(result.is_err());
}

#[test]
fn enrollment_valid_token_accepted() {
    let valid_tokens = vec!["enroll-token-1".to_string(), "enroll-token-2".to_string()];
    let verifier = EnrollmentVerifier::new(valid_tokens);

    assert!(verifier.verify("enroll-token-1"));
    assert!(verifier.verify("enroll-token-2"));
}

#[test]
fn enrollment_invalid_token_rejected() {
    let valid_tokens = vec!["enroll-token-1".to_string()];
    let verifier = EnrollmentVerifier::new(valid_tokens);

    assert!(!verifier.verify("invalid-token"));
    assert!(!verifier.verify(""));
}

#[test]
fn policy_ssl_paths_denied() {
    assert!(policy::is_path_denied("/etc/ssl/certs/ca-cert.pem"));
    assert!(policy::is_path_denied("/etc/ssh/sshd_config"));
}

#[test]
fn policy_private_segment_denied() {
    assert!(policy::is_path_denied("/home/user/private/config.yaml"));
}

#[test]
fn policy_normal_paths_ok() {
    assert!(!policy::is_path_denied("/etc/myapp/config.yaml"));
}

#[test]
fn policy_allowed_within_watch_root() {
    let roots = vec!["/etc/myapp"];
    assert!(policy::is_path_allowed("/etc/myapp/config.yaml", &roots));
}

#[test]
fn policy_denied_overrides_watch_root() {
    let roots = vec!["/etc/ssl"];
    // Path is in watch root but denied by security policy
    assert!(!policy::is_path_allowed("/etc/ssl/certs/cert.pem", &roots));
}