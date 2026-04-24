use sqlx::PgPool;
use uuid::Uuid;

use crate::models::WatchRootRow;

pub struct WatchRootsRepo;

impl WatchRootsRepo {
    pub async fn upsert(
        pool: &PgPool,
        watch_root_id: Uuid,
        host_id: Uuid,
        root_path: &str,
    ) -> anyhow::Result<WatchRootRow> {
        let row = sqlx::query_as::<_, WatchRootRow>(
            "INSERT INTO watch_roots (watch_root_id, host_id, root_path)
             VALUES ($1, $2, $3)
             ON CONFLICT (watch_root_id) DO UPDATE SET root_path = EXCLUDED.root_path
             RETURNING *"
        )
        .bind(watch_root_id)
        .bind(host_id)
        .bind(root_path)
        .fetch_one(pool)
        .await?;
        Ok(row)
    }

    pub async fn list_by_host(pool: &PgPool, host_id: Uuid) -> anyhow::Result<Vec<WatchRootRow>> {
        let rows = sqlx::query_as::<_, WatchRootRow>(
            "SELECT * FROM watch_roots WHERE host_id = $1 AND active = true"
        )
        .bind(host_id)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}