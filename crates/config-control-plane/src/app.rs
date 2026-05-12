use camino::Utf8Path;

use crate::auth;
use crate::config::{BroadcastMode, ControlPlaneConfig};
use crate::http::{middleware, routes};
use crate::services::AppState;

pub async fn run(cfg: ControlPlaneConfig) -> anyhow::Result<()> {
    tracing::info!(bind_addr = %cfg.bind_addr, "control plane starting");

    let db = config_storage::db::Database::connect(&cfg.database_url).await?;
    tracing::info!("Running database migrations...");
    db.run_migrations().await?;
    tracing::info!("Database migrations complete");

    let auth_state = auth::create_auth(db.pool().clone(), &cfg.auth).await?;

    let snapshot_store =
        config_snapshot::store::SnapshotStore::new(Utf8Path::new(&cfg.snapshot_data_dir))?;

    let state = AppState::new_with_diff_config(
        db,
        cfg.control_plane_secret.clone(),
        snapshot_store,
        cfg.diff.clone(),
        auth_state.clone(),
    )
    .with_query_timeout(cfg.query_timeout_secs)
    .with_repos_dir(cfg.repos_dir.clone())
    .with_github_token(cfg.github_token.clone())
    .with_admin_api_secret(cfg.auth.admin_api_secret.clone())
    .with_require_approval(cfg.auth.require_approval)
    .with_tls_required(cfg.auth.tls_required);

    // M3: Start Postgres LISTEN/NOTIFY listener for cross-pod event fan-out
    if matches!(cfg.broadcast_mode, BroadcastMode::Postgres) {
        let database_url = cfg.database_url.clone();
        let broadcast_tx = state.broadcast_tx.clone();
        let local_event_dedup = state.local_event_dedup.clone();
        let pool = state.db.pool().clone();
        tokio::spawn(async move {
            if let Err(e) = crate::pg_notify::start_pg_listener(
                &database_url,
                broadcast_tx,
                local_event_dedup,
                pool,
            )
            .await
            {
                tracing::error!(error = %e, "PgListener task exited with error");
            }
        });
        tracing::info!("Broadcast mode: Postgres (cross-pod fan-out enabled)");
    } else {
        tracing::info!("Broadcast mode: Local (single-pod, no cross-pod fan-out)");
    }

    let app = routes::build_router(state.clone(), auth_state);
    let app = middleware::apply_middleware(app, &state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, "control plane listening");

    axum::serve(listener, app).await?;

    Ok(())
}