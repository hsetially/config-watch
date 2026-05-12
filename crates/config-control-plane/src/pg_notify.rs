use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tracing;
use uuid::Uuid;

use config_storage::models::ChangeEventRow;
use config_transport::websocket::RealtimeMessage;

use crate::services::LocalEventDedup;

/// Convert a `ChangeEventRow` (from DB) to a `RealtimeMessage` for broadcast.
/// Requires the host's `environment` string, which is fetched separately from the `hosts` table.
pub fn row_to_realtime_message(row: &ChangeEventRow, environment: String) -> RealtimeMessage {
    RealtimeMessage {
        event_id: row.event_id,
        host_id: row.host_id,
        environment,
        path: row.canonical_path.clone().unwrap_or_default(),
        event_kind: row.event_kind.clone(),
        event_time: row.event_time.to_rfc3339(),
        severity: row.severity.clone(),
        author_display: row.author_name.clone(),
        summary: row.diff_summary_json.clone(),
        diff_render: row.diff_render.clone(),
        pr_url: row.pr_url.clone(),
        pr_number: row.pr_number,
    }
}

/// Start a background task that listens for Postgres NOTIFY events on the
/// `config_watch_changes` channel and forwards them to the local broadcast channel.
///
/// When a pod ingests a change event, the `notify_change_event` trigger fires
/// `pg_notify('config_watch_changes', event_id::text)`. This listener picks up
/// the notification, re-fetches the full event from the database, and pushes it
/// into the local `broadcast_tx` so that WebSocket clients on this pod receive
/// events that were ingested on other pods.
///
/// Events that originated on this pod are deduplicated using the shared
/// `local_event_dedup` set (which the ingest handler also pushes to) to avoid
/// double-broadcasting.
pub async fn start_pg_listener(
    database_url: &str,
    broadcast_tx: broadcast::Sender<RealtimeMessage>,
    local_event_dedup: LocalEventDedup,
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut listener = PgListener::connect(database_url).await?;
    listener.listen("config_watch_changes").await?;
    tracing::info!("PgListener started on config_watch_changes");

    let mut backoff = std::time::Duration::from_millis(100);
    let max_backoff = std::time::Duration::from_secs(30);

    loop {
        match listener.recv().await {
            Ok(notification) => {
                backoff = std::time::Duration::from_millis(100);
                let payload = notification.payload();
                let event_id = match Uuid::parse_str(payload) {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(payload, error = %e, "invalid event_id in NOTIFY payload");
                        continue;
                    }
                };

                // Skip if this pod already broadcast this event via the ingest handler
                if let Ok(dedup) = local_event_dedup.lock() {
                    if dedup.contains(&event_id) {
                        tracing::trace!(%event_id, "deduplicating locally-originated event");
                        continue;
                    }
                }

                // Re-fetch the event from the database
                match fetch_event(&pool, event_id).await {
                    Ok(Some((row, environment))) => {
                        let msg = row_to_realtime_message(&row, environment);
                        let _ = broadcast_tx.send(msg);
                        tracing::debug!(%event_id, "forwarded remote event to local broadcast");
                    }
                    Ok(None) => {
                        tracing::warn!(%event_id, "event not found in database after NOTIFY");
                    }
                    Err(e) => {
                        tracing::error!(%event_id, error = %e, "failed to fetch event after NOTIFY");
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "PgListener recv error, reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);

                // Reconnect the listener
                match PgListener::connect(database_url).await {
                    Ok(new_listener) => {
                        listener = new_listener;
                        if let Err(e) = listener.listen("config_watch_changes").await {
                            tracing::error!(error = %e, "failed to re-listen after reconnect");
                        } else {
                            tracing::info!("PgListener reconnected successfully");
                        }
                    }
                    Err(e2) => {
                        tracing::error!(error = %e2, "failed to reconnect PgListener");
                    }
                }
            }
        }
    }
}

/// Fetch a `ChangeEventRow` by `event_id` and its host's `environment`.
async fn fetch_event(
    pool: &PgPool,
    event_id: Uuid,
) -> anyhow::Result<Option<(ChangeEventRow, String)>> {
    let row =
        sqlx::query_as::<_, ChangeEventRow>("SELECT * FROM change_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_optional(pool)
            .await?;

    match row {
        Some(row) => {
            let environment =
                sqlx::query_scalar::<_, String>("SELECT environment FROM hosts WHERE host_id = $1")
                    .bind(row.host_id)
                    .fetch_optional(pool)
                    .await?
                    .unwrap_or_default();
            Ok(Some((row, environment)))
        }
        None => Ok(None),
    }
}
