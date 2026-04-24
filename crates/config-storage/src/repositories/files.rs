use sqlx::PgPool;
use uuid::Uuid;

use crate::models::FileRow;

pub struct FilesRepo;

impl FilesRepo {
    pub async fn upsert(
        pool: &PgPool,
        file_id: Uuid,
        host_id: Uuid,
        watch_root_id: Uuid,
        canonical_path: &str,
        last_hash: &str,
    ) -> anyhow::Result<FileRow> {
        let row = sqlx::query_as::<_, FileRow>(
            "INSERT INTO files (file_id, host_id, watch_root_id, canonical_path, last_hash, exists_now)
             VALUES ($1, $2, $3, $4, $5, true)
             ON CONFLICT (host_id, canonical_path) DO UPDATE SET
                 last_hash = EXCLUDED.last_hash,
                 last_seen_at = NOW(),
                 exists_now = true
             RETURNING *"
        )
        .bind(file_id)
        .bind(host_id)
        .bind(watch_root_id)
        .bind(canonical_path)
        .bind(last_hash)
        .fetch_one(pool)
        .await?;
        Ok(row)
    }

    pub async fn get_by_path(pool: &PgPool, host_id: Uuid, canonical_path: &str) -> anyhow::Result<Option<FileRow>> {
        let row = sqlx::query_as::<_, FileRow>(
            "SELECT * FROM files WHERE host_id = $1 AND canonical_path = $2"
        )
        .bind(host_id)
        .bind(canonical_path)
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }
}