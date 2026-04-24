use config_shared::ids::IdempotencyKey;

pub fn generate_idempotency_header(key: &IdempotencyKey) -> String {
    key.0.clone()
}

pub fn parse_idempotency_header(value: &str) -> Option<IdempotencyKey> {
    if value.is_empty() {
        None
    } else {
        Some(IdempotencyKey(value.to_string()))
    }
}