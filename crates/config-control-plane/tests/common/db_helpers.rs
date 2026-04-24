use sqlx::PgPool;
use uuid::Uuid;

#[allow(dead_code)]
pub async fn seed_host(
    pool: &PgPool,
    host_id: Uuid,
    hostname: &str,
    environment: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO hosts (host_id, hostname, environment, labels_json, agent_version, status)
         VALUES ($1, $2, $3, '{}', '0.1.0', 'healthy')
         ON CONFLICT (host_id) DO UPDATE SET
             hostname = EXCLUDED.hostname,
             environment = EXCLUDED.environment,
             status = 'healthy',
             last_heartbeat_at = NOW()",
    )
    .bind(host_id)
    .bind(hostname)
    .bind(environment)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn seed_change_event(
    pool: &PgPool,
    event_id: Uuid,
    idempotency_key: &str,
    host_id: Uuid,
    event_kind: &str,
    severity: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO change_events (
            event_id, idempotency_key, host_id, event_time, event_kind,
            author_confidence, severity
         ) VALUES ($1, $2, $3, NOW(), $4, 'unknown', $5)",
    )
    .bind(event_id)
    .bind(idempotency_key)
    .bind(host_id)
    .bind(event_kind)
    .bind(severity)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn set_host_status(pool: &PgPool, host_id: Uuid, status: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE hosts SET status = $1 WHERE host_id = $2")
        .bind(status)
        .bind(host_id)
        .execute(pool)
        .await?;
    Ok(())
}
