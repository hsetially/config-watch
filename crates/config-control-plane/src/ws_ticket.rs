use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use constant_time_eq::constant_time_eq;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const TICKET_TTL_SECS: i64 = 30;

/// Generate a stateless WS ticket encoding `user_id` and an expiry timestamp,
/// signed with HMAC-SHA256 using the control-plane secret.
///
/// Format: `base64url(payload).base64url(signature)`
/// where payload = `{"u":"<user_id>","e":<unix_timestamp>}`
/// and signature = HMAC-SHA256(secret, payload_bytes)
pub fn generate_ticket(user_id: &str, secret: &str) -> String {
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(TICKET_TTL_SECS);
    let payload_json = serde_json::json!({
        "u": user_id,
        "e": expires_at.timestamp(),
    });
    let payload_str = serde_json::to_string(&payload_json).expect("ticket payload serializes");
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_str.as_bytes());

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig);

    format!("{}.{}", payload_b64, sig_b64)
}

/// Verify a stateless WS ticket and return the embedded `user_id`.
///
/// Returns `Ok(user_id)` if the signature is valid and the ticket has not expired.
/// Returns `Err(&str)` with a descriptive error on failure.
pub fn verify_ticket(ticket: &str, secret: &str) -> Result<String, &'static str> {
    let dot_pos = ticket.rfind('.').ok_or("invalid ticket format")?;
    let payload_b64 = &ticket[..dot_pos];
    let provided_sig_b64 = &ticket[dot_pos + 1..];

    // Verify HMAC signature
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload_b64.as_bytes());
    let expected_sig = mac.finalize().into_bytes();
    let expected_sig_b64 = URL_SAFE_NO_PAD.encode(expected_sig);

    if !constant_time_eq(provided_sig_b64.as_bytes(), expected_sig_b64.as_bytes()) {
        return Err("invalid ticket signature");
    }

    // Decode payload
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| "invalid ticket payload encoding")?;
    let payload: serde_json::Value =
        serde_json::from_slice(&payload_bytes).map_err(|_| "invalid ticket payload json")?;

    let user_id = payload["u"]
        .as_str()
        .ok_or("missing user_id in ticket payload")?;
    let expires_ts = payload["e"]
        .as_i64()
        .ok_or("missing expiry in ticket payload")?;

    // Check expiry
    let now_ts = chrono::Utc::now().timestamp();
    if expires_ts < now_ts {
        return Err("ticket expired");
    }

    Ok(user_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let secret = "test-secret-key-that-is-long-enough";
        let ticket = generate_ticket("user-123", secret);
        let user_id = verify_ticket(&ticket, secret).unwrap();
        assert_eq!(user_id, "user-123");
    }

    #[test]
    fn expired_ticket_rejected() {
        let secret = "test-secret-key-that-is-long-enough";
        // Generate a ticket with a manually crafted expired payload
        let payload_json = serde_json::json!({
            "u": "user-123",
            "e": chrono::Utc::now().timestamp() - 100, // expired 100s ago
        });
        let payload_str = serde_json::to_string(&payload_json).unwrap();
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload_str.as_bytes());
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload_b64.as_bytes());
        let sig = mac.finalize().into_bytes();
        let sig_b64 = URL_SAFE_NO_PAD.encode(sig);
        let ticket = format!("{}.{}", payload_b64, sig_b64);

        assert!(verify_ticket(&ticket, secret).is_err());
    }

    #[test]
    fn wrong_secret_rejected() {
        let secret = "secret-a";
        let ticket = generate_ticket("user-123", secret);
        assert!(verify_ticket(&ticket, "secret-b").is_err());
    }

    #[test]
    fn tampered_payload_rejected() {
        let secret = "test-secret-key-that-is-long-enough";
        let ticket = generate_ticket("user-123", secret);
        // Tamper with the payload part
        let mut tampered = ticket.clone();
        // Change one character in the payload (base64url encoded)
        let dot_pos = tampered.rfind('.').unwrap();
        let payload = &tampered[..dot_pos].to_string();
        let mut payload_chars: Vec<char> = payload.chars().collect();
        if let Some(c) = payload_chars.last_mut() {
            *c = if *c == 'A' { 'B' } else { 'A' };
        }
        let new_payload: String = payload_chars.iter().collect();
        tampered = format!("{}.{}", new_payload, &ticket[dot_pos + 1..]);
        assert!(verify_ticket(&tampered, secret).is_err());
    }
}