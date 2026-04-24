use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::ChangeEventRow;

pub struct ChangeEventsRepo;

#[derive(Debug, Clone)]
pub struct ChangeEventFilters {
    pub host_id: Option<Uuid>,
    pub path_prefix: Option<String>,
    pub filename: Option<String>,
    pub author: Option<String>,
    pub severity: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

impl ChangeEventsRepo {
    pub async fn insert(pool: &PgPool, event: &ChangeEventRow) -> anyhow::Result<ChangeEventRow> {
        let row = sqlx::query_as::<_, ChangeEventRow>(
            "INSERT INTO change_events (
                event_id, idempotency_key, host_id, file_id, event_time, event_kind,
                previous_snapshot_id, current_snapshot_id, diff_artifact_uri, diff_summary_json,
                author_name, author_source, author_confidence, process_hint, severity,
                diff_render, canonical_path, pr_url, pr_number
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
            RETURNING *"
        )
        .bind(event.event_id)
        .bind(&event.idempotency_key)
        .bind(event.host_id)
        .bind(event.file_id)
        .bind(event.event_time)
        .bind(&event.event_kind)
        .bind(event.previous_snapshot_id)
        .bind(event.current_snapshot_id)
        .bind(&event.diff_artifact_uri)
        .bind(&event.diff_summary_json)
        .bind(&event.author_name)
        .bind(&event.author_source)
        .bind(&event.author_confidence)
        .bind(&event.process_hint)
        .bind(&event.severity)
        .bind(&event.diff_render)
        .bind(&event.canonical_path)
        .bind(&event.pr_url)
        .bind(event.pr_number)
        .fetch_one(pool)
        .await?;

        Ok(row)
    }

    pub async fn get(pool: &PgPool, event_id: Uuid) -> anyhow::Result<Option<ChangeEventRow>> {
        let row =
            sqlx::query_as::<_, ChangeEventRow>("SELECT * FROM change_events WHERE event_id = $1")
                .bind(event_id)
                .fetch_optional(pool)
                .await?;
        Ok(row)
    }

    pub async fn exists_by_idempotency_key(pool: &PgPool, key: &str) -> anyhow::Result<bool> {
        let row: Option<(Uuid,)> =
            sqlx::query_as("SELECT event_id FROM change_events WHERE idempotency_key = $1")
                .bind(key)
                .fetch_optional(pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn list(
        pool: &PgPool,
        filters: &ChangeEventFilters,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<ChangeEventRow>> {
        let mut query = String::from("SELECT * FROM change_events WHERE 1=1");
        let mut param_idx = 0u32;

        if filters.host_id.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND host_id = ${}", param_idx));
        }
        if filters.path_prefix.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND canonical_path LIKE ${}", param_idx));
        }
        if filters.filename.is_some() {
            param_idx += 1;
            query.push_str(&format!(
                " AND SPLIT_PART(canonical_path, '/', -1) LIKE ${}",
                param_idx
            ));
        }
        if filters.author.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND author_name = ${}", param_idx));
        }
        if filters.severity.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND severity = ${}", param_idx));
        }
        if filters.since.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND event_time >= ${}", param_idx));
        }
        if filters.until.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND event_time <= ${}", param_idx));
        }

        param_idx += 1;
        let limit_idx = param_idx;
        param_idx += 1;
        let offset_idx = param_idx;
        query.push_str(&format!(
            " ORDER BY event_time DESC LIMIT ${} OFFSET ${}",
            limit_idx, offset_idx
        ));

        let mut q = sqlx::query_as::<_, ChangeEventRow>(&query);

        if let Some(v) = filters.host_id {
            q = q.bind(v);
        }
        if let Some(ref v) = filters.path_prefix {
            q = q.bind(format!("{}%", v));
        }
        if let Some(ref v) = filters.filename {
            q = q.bind(format!("%{}%", v));
        }
        if let Some(ref v) = filters.author {
            q = q.bind(v);
        }
        if let Some(ref v) = filters.severity {
            q = q.bind(v);
        }
        if let Some(v) = filters.since {
            q = q.bind(v);
        }
        if let Some(v) = filters.until {
            q = q.bind(v);
        }
        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(pool).await?;
        Ok(rows)
    }

    pub async fn count(pool: &PgPool, filters: &ChangeEventFilters) -> anyhow::Result<i64> {
        let mut query = String::from("SELECT COUNT(*) FROM change_events WHERE 1=1");
        let mut param_idx = 0u32;

        if filters.host_id.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND host_id = ${}", param_idx));
        }
        if filters.path_prefix.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND canonical_path LIKE ${}", param_idx));
        }
        if filters.filename.is_some() {
            param_idx += 1;
            query.push_str(&format!(
                " AND SPLIT_PART(canonical_path, '/', -1) LIKE ${}",
                param_idx
            ));
        }
        if filters.author.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND author_name = ${}", param_idx));
        }
        if filters.severity.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND severity = ${}", param_idx));
        }
        if filters.since.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND event_time >= ${}", param_idx));
        }
        if filters.until.is_some() {
            param_idx += 1;
            query.push_str(&format!(" AND event_time <= ${}", param_idx));
        }

        let mut q = sqlx::query_as::<_, (i64,)>(&query);

        if let Some(v) = filters.host_id {
            q = q.bind(v);
        }
        if let Some(ref v) = filters.path_prefix {
            q = q.bind(format!("{}%", v));
        }
        if let Some(ref v) = filters.filename {
            q = q.bind(format!("%{}%", v));
        }
        if let Some(ref v) = filters.author {
            q = q.bind(v);
        }
        if let Some(ref v) = filters.severity {
            q = q.bind(v);
        }
        if let Some(v) = filters.since {
            q = q.bind(v);
        }
        if let Some(v) = filters.until {
            q = q.bind(v);
        }

        let row = q.fetch_one(pool).await?;
        Ok(row.0)
    }

    pub async fn update_pr_url_batch(
        pool: &PgPool,
        event_ids: &[Uuid],
        pr_url: &str,
        pr_number: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE change_events SET pr_url = $1, pr_number = $2 WHERE event_id = ANY($3)",
        )
        .bind(pr_url)
        .bind(pr_number)
        .bind(event_ids)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Fetch (canonical_path, content_hash, previous_content_hash) tuples for the given event IDs.
    /// Joins with snapshots to resolve current_snapshot_id → content_hash and
    /// previous_snapshot_id → previous content_hash.
    pub async fn get_content_hashes_by_event_ids(
        pool: &PgPool,
        event_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(String, Option<String>, Option<String>)>> {
        let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT ce.canonical_path, curr.content_hash, prev.content_hash
             FROM change_events ce
             LEFT JOIN snapshots curr ON ce.current_snapshot_id = curr.snapshot_id
             LEFT JOIN snapshots prev ON ce.previous_snapshot_id = prev.snapshot_id
             WHERE ce.event_id = ANY($1)",
        )
        .bind(event_ids)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}
