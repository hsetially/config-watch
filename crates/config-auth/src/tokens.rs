use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct AgentCredential {
    pub host_id: String,
    pub expires_at: DateTime<Utc>,
    pub token: String,
}

impl AgentCredential {
    pub fn issue(secret: &str, host_id: &str, ttl: Duration) -> Self {
        let expires_at = Utc::now() + ttl;
        let expires_str = expires_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let message = format!("{}|{}", host_id, expires_str);

        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        let result = mac.finalize();
        let sig = hex::encode(result.into_bytes());

        let token = format!("{}|{}|{}", host_id, expires_str, sig);

        Self {
            host_id: host_id.to_string(),
            expires_at,
            token,
        }
    }

    pub fn verify(secret: &str, token: &str) -> Result<Self, &'static str> {
        let parts: Vec<&str> = token.splitn(3, '|').collect();
        if parts.len() != 3 {
            return Err("invalid token format");
        }

        let host_id = parts[0];
        let expires_str = parts[1];
        let provided_sig = parts[2];

        let expires_at: DateTime<Utc> = DateTime::parse_from_rfc3339(expires_str)
            .map_err(|_| "invalid expiration format")?
            .into();

        if expires_at < Utc::now() {
            return Err("token expired");
        }

        let message = format!("{}|{}", host_id, expires_str);
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        let result = mac.finalize();
        let expected_sig = hex::encode(result.into_bytes());

        if !constant_time_eq::constant_time_eq(provided_sig.as_bytes(), expected_sig.as_bytes()) {
            return Err("invalid signature");
        }

        Ok(Self {
            host_id: host_id.to_string(),
            expires_at,
            token: token.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify() {
        let secret = "test-secret-key";
        let credential = AgentCredential::issue(secret, "host-123", Duration::hours(24));
        let verified = AgentCredential::verify(secret, &credential.token).unwrap();
        assert_eq!(verified.host_id, "host-123");
    }

    #[test]
    fn expired_token_rejected() {
        let secret = "test-secret-key";
        let credential = AgentCredential::issue(secret, "host-123", Duration::hours(-1));
        assert!(AgentCredential::verify(secret, &credential.token).is_err());
    }

    #[test]
    fn wrong_secret_rejected() {
        let credential = AgentCredential::issue("secret-a", "host-123", Duration::hours(24));
        assert!(AgentCredential::verify("secret-b", &credential.token).is_err());
    }
}
