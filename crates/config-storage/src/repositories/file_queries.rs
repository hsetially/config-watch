use sqlx::PgPool;
use uuid::Uuid;

use crate::models::FileQueryRow;

pub struct FileQueriesRepo;

impl FileQueriesRepo {
    pub async fn insert(
        pool: &PgPool,
        query_id: Uuid,
        requester_id: &str,
        host_id: Uuid,
        canonical_path: &str,
        query_kind: &str,
        result_status: &str,
    ) -> anyhow::Result<FileQueryRow> {
        let row = sqlx::query_as::<_, FileQueryRow>(
            "INSERT INTO file_queries (query_id, requester_id, host_id, canonical_path, query_kind, result_status, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())
             RETURNING *"
        )
        .bind(query_id)
        .bind(requester_id)
        .bind(host_id)
        .bind(canonical_path)
        .bind(query_kind)
        .bind(result_status)
        .fetch_one(pool)
        .await?;
        Ok(row)
    }
}