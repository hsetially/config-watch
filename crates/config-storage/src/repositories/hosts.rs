use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::HostRow;

pub struct HostsRepo;

impl HostsRepo {
    pub async fn register(
        pool: &PgPool,
        host_id: Uuid,
        hostname: &str,
        environment: &str,
        labels_json: serde_json::Value,
        agent_version: &str,
    ) -> anyhow::Result<HostRow> {
        let row = sqlx::query_as::<_, HostRow>(
            "INSERT INTO hosts (host_id, hostname, environment, labels_json, agent_version, status)
             VALUES ($1, $2, $3, $4, $5, 'healthy')
             ON CONFLICT (host_id) DO UPDATE SET
                 hostname = EXCLUDED.hostname,
                 environment = EXCLUDED.environment,
                 labels_json = EXCLUDED.labels_json,
                 agent_version = EXCLUDED.agent_version,
                 status = 'healthy',
                 last_heartbeat_at = NOW()
             RETURNING *",
        )
        .bind(host_id)
        .bind(hostname)
        .bind(environment)
        .bind(labels_json)
        .bind(agent_version)
        .fetch_one(pool)
        .await?;

        Ok(row)
    }

    pub async fn heartbeat(pool: &PgPool, host_id: Uuid) -> anyhow::Result<()> {
        sqlx::query("UPDATE hosts SET last_heartbeat_at = NOW() WHERE host_id = $1")
            .bind(host_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn get(pool: &PgPool, host_id: Uuid) -> anyhow::Result<Option<HostRow>> {
        let row = sqlx::query_as::<_, HostRow>("SELECT * FROM hosts WHERE host_id = $1")
            .bind(host_id)
            .fetch_optional(pool)
            .await?;
        Ok(row)
    }

    pub async fn list(
        pool: &PgPool,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<HostRow>> {
        let rows = match status {
            Some(s) => {
                sqlx::query_as::<_, HostRow>(
                    "SELECT * FROM hosts WHERE status = $1 ORDER BY registered_at DESC LIMIT $2 OFFSET $3"
                )
                .bind(s)
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, HostRow>(
                    "SELECT * FROM hosts ORDER BY registered_at DESC LIMIT $1 OFFSET $2"
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?
            }
        };
        Ok(rows)
    }

    pub async fn update_status(pool: &PgPool, host_id: Uuid, status: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE hosts SET status = $1 WHERE host_id = $2")
            .bind(status)
            .bind(host_id)
            .execute(pool)
            .await?;
        Ok(())
    }
}

pub fn derive_host_status(
    last_heartbeat: Option<DateTime<Utc>>,
    heartbeat_interval_secs: u64,
) -> &'static str {
    let Some(last_hb) = last_heartbeat else {
        return "registering";
    };
    let elapsed = Utc::now() - last_hb;
    let threshold_2x =
        chrono::Duration::from_std(std::time::Duration::from_secs(heartbeat_interval_secs * 2))
            .unwrap_or(chrono::Duration::seconds(60));
    let threshold_5x =
        chrono::Duration::from_std(std::time::Duration::from_secs(heartbeat_interval_secs * 5))
            .unwrap_or(chrono::Duration::seconds(150));

    if elapsed <= threshold_2x {
        "healthy"
    } else if elapsed <= threshold_5x {
        "degraded"
    } else {
        "offline"
    }
}
