use anyhow::Result;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use config_storage::models::ChangeEventRow;
use config_transport::websocket::RealtimeMessage;

pub enum IngestOutcome {
    Accepted { event_id: Uuid },
    Duplicate { event_id: Uuid },
    Rejected { reason: String },
}

pub struct IngestService;

impl IngestService {
    pub async fn ingest_change(
        pool: &PgPool,
        broadcast_tx: &broadcast::Sender<RealtimeMessage>,
        snapshot_store: &config_snapshot::store::SnapshotStore,
        body: serde_json::Value,
    ) -> Result<IngestOutcome> {
        let schema_version = body
            .get("schema_version")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if schema_version != "1.0" {
            return Ok(IngestOutcome::Rejected {
                reason: format!("unsupported schema version: {}", schema_version),
            });
        }

        let event_data = match body.get("event") {
            Some(e) => e,
            None => {
                return Ok(IngestOutcome::Rejected {
                    reason: "missing event".into(),
                })
            }
        };

        let idempotency_key = event_data
            .get("idempotency_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if idempotency_key.is_empty() {
            return Ok(IngestOutcome::Rejected {
                reason: "missing idempotency_key".into(),
            });
        }

        if config_storage::repositories::change_events::ChangeEventsRepo::exists_by_idempotency_key(
            pool,
            &idempotency_key,
        )
        .await?
        {
            let existing = sqlx::query_as::<_, (Uuid,)>(
                "SELECT event_id FROM change_events WHERE idempotency_key = $1",
            )
            .bind(&idempotency_key)
            .fetch_one(pool)
            .await?;
            return Ok(IngestOutcome::Duplicate {
                event_id: existing.0,
            });
        }

        let event_id = event_data
            .get("event_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);

        let host_id = event_data
            .get("host_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let event_kind = event_data
            .get("event_kind")
            .and_then(|v| v.as_str())
            .unwrap_or("modified")
            .to_string();

        let canonical_path = event_data
            .get("canonical_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Look up the previous change event for this file to establish the snapshot chain
        let previous_snapshot_id: Option<Uuid> = if !canonical_path.is_empty() {
            match sqlx::query_scalar::<_, Option<Uuid>>(
                "SELECT current_snapshot_id FROM change_events
                 WHERE host_id = $1 AND canonical_path = $2
                 ORDER BY event_time DESC LIMIT 1",
            )
            .bind(host_id)
            .bind(&canonical_path)
            .fetch_optional(pool)
            .await
            {
                Ok(Some(Some(id))) => Some(id),
                _ => None,
            }
        } else {
            None
        };

        let severity = event_data
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("info")
            .to_string();

        let author_name = event_data
            .get("attribution")
            .and_then(|a| a.get("author_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let author_confidence = event_data
            .get("attribution")
            .and_then(|a| a.get("confidence"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let diff_summary = event_data.get("diff_summary").cloned();

        let diff_render = event_data
            .get("diff_render")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // --- Store snapshot content if provided ---
        let mut current_snapshot_id: Option<Uuid> = None;
        if let Some(b64) = event_data.get("content_b64").and_then(|v| v.as_str()) {
            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                Ok(data) => {
                    let content_hash = config_snapshot::hash::compute_blake3(&data);
                    match snapshot_store.write_snapshot(&content_hash, &data).await {
                        Ok(snapshot_ref) => {
                            current_snapshot_id = Some(snapshot_ref.snapshot_id.0);
                            let storage_uri = format!(
                                "{}/{}/{}",
                                snapshot_store.base_dir(),
                                &content_hash[..2.min(content_hash.len())],
                                content_hash
                            );
                            let _ = config_storage::repositories::snapshots::SnapshotsRepo::insert(
                                pool,
                                snapshot_ref.snapshot_id.0,
                                &content_hash,
                                data.len() as i64,
                                &storage_uri,
                                "none",
                            )
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to write snapshot during ingest");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to decode content_b64 during ingest");
                }
            }
        }

        let row = ChangeEventRow {
            event_id,
            idempotency_key,
            host_id,
            file_id: None,
            event_time: chrono::Utc::now(),
            event_kind,
            previous_snapshot_id,
            current_snapshot_id,
            diff_artifact_uri: None,
            diff_summary_json: diff_summary,
            author_name,
            author_source: None,
            author_confidence,
            process_hint: None,
            severity,
            created_at: chrono::Utc::now(),
            diff_render,
            canonical_path: Some(canonical_path.clone()),
            pr_url: None,
            pr_number: None,
        };

        config_storage::repositories::change_events::ChangeEventsRepo::insert(pool, &row).await?;

        let host_env =
            sqlx::query_as::<_, (String,)>("SELECT environment FROM hosts WHERE host_id = $1")
                .bind(host_id)
                .fetch_optional(pool)
                .await?
                .map(|r| r.0)
                .unwrap_or_default();

        let msg = RealtimeMessage {
            event_id: row.event_id,
            host_id: row.host_id,
            environment: host_env,
            path: canonical_path,
            event_kind: row.event_kind.clone(),
            event_time: row.event_time.to_rfc3339(),
            severity: row.severity.clone(),
            author_display: row.author_name.clone(),
            summary: row.diff_summary_json.clone(),
            diff_render: row.diff_render.clone(),
            pr_url: None,
            pr_number: None,
        };

        let _ = broadcast_tx.send(msg);

        let _ = config_storage::repositories::hosts::HostsRepo::heartbeat(pool, host_id).await;

        Ok(IngestOutcome::Accepted { event_id })
    }
}
