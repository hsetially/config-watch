use camino::Utf8Path;

use crate::config::ControlPlaneConfig;
use crate::http::{middleware, routes};
use crate::services::AppState;

pub async fn run(cfg: ControlPlaneConfig) -> anyhow::Result<()> {
    tracing::info!(bind_addr = %cfg.bind_addr, "control plane starting");

    let db = config_storage::db::Database::connect(&cfg.database_url).await?;
    db.run_migrations().await?;

    let snapshot_store = config_snapshot::store::SnapshotStore::new(Utf8Path::new(&cfg.snapshot_data_dir))?;

    let state = AppState::new(db, cfg.control_plane_secret.clone(), snapshot_store)
        .with_query_timeout(cfg.query_timeout_secs)
        .with_repos_dir(cfg.repos_dir.clone())
        .with_github_token(cfg.github_token.clone());

    let app = routes::build_router(state);
    let app = middleware::apply_middleware(app);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, "control plane listening");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}