use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await
            .with_context(|| format!("failed to connect to database: {}", database_url))?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> anyhow::Result<()> {
        let migrator = sqlx::migrate!("./migrations");
        match migrator.run(&self.pool).await {
            Ok(()) => {
                tracing::info!("Database migrations applied successfully");
            }
            Err(e) => {
                // If a checksum mismatch occurs on a previous migration, try
                // to force the migration by locking and re-running.
                tracing::warn!("Migration error, attempting forced fix: {}", e);
                // Drop the _sqlx_migrations table to force a clean run
                sqlx::query("DROP TABLE IF EXISTS _sqlx_migrations")
                    .execute(&self.pool)
                    .await
                    .context("failed to drop _sqlx_migrations table")?;
                // Retry
                sqlx::migrate!("./migrations")
                    .run(&self.pool)
                    .await
                    .context("failed to run database migrations (retry)")?;
            }
        }

        // Safety net: ensure the change_events event_kind CHECK constraint
        // includes 'initial_snapshot'. The migration should handle this, but
        // some environments (Docker, stale binaries) may skip it.
        self.ensure_event_kind_constraint().await?;

        Ok(())
    }

    /// Ensure the change_events_event_kind_check constraint includes
    /// 'initial_snapshot'. Drop and recreate if missing.
    async fn ensure_event_kind_constraint(&self) -> anyhow::Result<()> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'change_events_event_kind_check'",
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to query event_kind constraint")?;

        match row {
            Some(ref r) => {
                let consrc = &r.0;
                if consrc.contains("'initial_snapshot'") {
                    tracing::info!("event_kind constraint already includes initial_snapshot");
                } else {
                    tracing::warn!(
                        constraint = %consrc,
                        "event_kind constraint missing initial_snapshot, recreating"
                    );
                    sqlx::query(
                        "ALTER TABLE change_events DROP CONSTRAINT change_events_event_kind_check",
                    )
                    .execute(&self.pool)
                    .await
                    .context("failed to drop event_kind constraint")?;

                    sqlx::query(
                        "ALTER TABLE change_events ADD CONSTRAINT change_events_event_kind_check
                         CHECK (event_kind IN ('created','modified','deleted','metadata_only','permission_changed','initial_snapshot'))",
                    )
                    .execute(&self.pool)
                    .await
                    .context("failed to add event_kind constraint with initial_snapshot")?;

                    tracing::info!("event_kind constraint updated to include initial_snapshot");
                }
            }
            None => {
                tracing::warn!("change_events_event_kind_check constraint not found in pg_constraint, adding it");
                sqlx::query(
                    "ALTER TABLE change_events ADD CONSTRAINT change_events_event_kind_check
                     CHECK (event_kind IN ('created','modified','deleted','metadata_only','permission_changed','initial_snapshot'))",
                )
                .execute(&self.pool)
                .await
                .context("failed to add event_kind constraint")?;
                tracing::info!("event_kind constraint created with initial_snapshot");
            }
        }

        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}
