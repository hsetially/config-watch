use sqlx::PgPool;
use uuid::Uuid;

use crate::models::SnapshotRow;

pub struct SnapshotsRepo;

impl SnapshotsRepo {
    pub async fn insert(
        pool: &PgPool,
        snapshot_id: Uuid,
        content_hash: &str,
        size_bytes: i64,
        storage_uri: &str,
        compression: &str,
    ) -> anyhow::Result<SnapshotRow> {
        let row = sqlx::query_as::<_, SnapshotRow>(
            "INSERT INTO snapshots (snapshot_id, content_hash, size_bytes, storage_uri, compression)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING *"
        )
        .bind(snapshot_id)
        .bind(content_hash)
        .bind(size_bytes)
        .bind(storage_uri)
        .bind(compression)
        .fetch_one(pool)
        .await?;
        Ok(row)
    }
}
