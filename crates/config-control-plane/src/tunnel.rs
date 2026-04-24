use std::sync::Arc;

use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use axum::extract::ws::{Message, WebSocket};

use config_transport::tunnel::{QueryResponsePayload, TunnelMessage, TunnelMessageType};

use crate::metrics::ControlPlaneMetrics;
use crate::services::AppState;

const PING_INTERVAL_SECS: u64 = 30;

pub struct AgentConnection {
    pub tx: mpsc::Sender<String>,
    pub pending_queries: DashMap<String, oneshot::Sender<QueryResponsePayload>>,
}

pub struct AgentRegistry {
    connections: DashMap<Uuid, AgentConnection>,
    metrics: Arc<ControlPlaneMetrics>,
}

impl AgentRegistry {
    pub fn new(metrics: Arc<ControlPlaneMetrics>) -> Self {
        Self {
            connections: DashMap::new(),
            metrics,
        }
    }

    pub fn register(&self, host_id: Uuid, tx: mpsc::Sender<String>) {
        if self.connections.remove(&host_id).is_some() {
            tracing::warn!(host_id = %host_id, "replacing existing tunnel connection");
        }
        self.connections.insert(
            host_id,
            AgentConnection {
                tx,
                pending_queries: DashMap::new(),
            },
        );
        self.metrics
            .tunnel_connections_active
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(host_id = %host_id, "agent tunnel registered");
    }

    pub fn unregister(&self, host_id: Uuid) {
        if let Some((_, conn)) = self.connections.remove(&host_id) {
            for entry in conn.pending_queries {
                let _ = entry.1.send(QueryResponsePayload {
                    status: "error".into(),
                    data: None,
                    error: Some("agent disconnected".into()),
                });
            }
            self.metrics
                .tunnel_connections_active
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            tracing::info!(host_id = %host_id, "agent tunnel unregistered");
        }
    }

    pub fn send_query(
        &self,
        host_id: Uuid,
        request_id: String,
        message: &TunnelMessage,
    ) -> anyhow::Result<oneshot::Receiver<QueryResponsePayload>> {
        let conn = self
            .connections
            .get(&host_id)
            .ok_or_else(|| anyhow::anyhow!("agent not connected via tunnel"))?;

        let (tx, rx) = oneshot::channel();
        conn.pending_queries.insert(request_id.clone(), tx);

        let json = serde_json::to_string(message)
            .map_err(|e| anyhow::anyhow!("failed to serialize tunnel message: {}", e))?;

        if let Err(e) = conn.tx.try_send(json) {
            // Remove the pending query since we couldn't send
            conn.pending_queries.retain(|k, _| k != &request_id);
            return Err(anyhow::anyhow!("agent send channel closed: {}", e));
        }

        Ok(rx)
    }

    pub fn handle_response(&self, host_id: Uuid, request_id: &str, payload: QueryResponsePayload) {
        if let Some(conn) = self.connections.get(&host_id) {
            if let Some((_, sender)) = conn.pending_queries.remove(request_id) {
                let _ = sender.send(payload);
            } else {
                tracing::warn!(
                    host_id = %host_id,
                    request_id = %request_id,
                    "received response for unknown request"
                );
            }
        }
    }

    pub fn is_connected(&self, host_id: Uuid) -> bool {
        self.connections.contains_key(&host_id)
    }
}

pub async fn handle_agent_tunnel(socket: WebSocket, state: AppState, host_id: Uuid) {
    let (mut ws_sink, mut ws_stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(64);

    // Clone the sender before moving the original into the registry
    let out_tx_clone = out_tx.clone();
    state.tunnel_registry.register(host_id, out_tx);

    let registry = state.tunnel_registry.clone();
    let metrics = state.metrics.clone();

    let mut send_task = tokio::spawn(async move {
        let mut ping_interval =
            tokio::time::interval(std::time::Duration::from_secs(PING_INTERVAL_SECS));
        ping_interval.tick().await; // skip first immediate tick

        loop {
            tokio::select! {
                msg = out_rx.recv() => {
                    match msg {
                        Some(text) => {
                            if ws_sink.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = ping_interval.tick() => {
                    let ping = TunnelMessage::ping();
                    let json = serde_json::to_string(&ping).unwrap_or_default();
                    if ws_sink.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = ws_sink.close().await;
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Text(text) => {
                    let tunnel_msg: TunnelMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to parse tunnel message");
                            continue;
                        }
                    };

                    match tunnel_msg.msg_type {
                        TunnelMessageType::QueryResponse => {
                            if let Some(ref request_id) = tunnel_msg.request_id {
                                let payload: QueryResponsePayload = tunnel_msg
                                    .payload
                                    .and_then(|v| serde_json::from_value(v).ok())
                                    .unwrap_or(QueryResponsePayload {
                                        status: "error".into(),
                                        data: None,
                                        error: Some("malformed response".into()),
                                    });

                                registry.handle_response(host_id, request_id, payload);
                                metrics
                                    .tunnel_queries_routed
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        TunnelMessageType::Pong => {
                            tracing::trace!(host_id = %host_id, "tunnel pong received");
                        }
                        TunnelMessageType::Ping => {
                            let pong = TunnelMessage::pong();
                            let json = serde_json::to_string(&pong).unwrap_or_default();
                            let _ = out_tx_clone.send(json).await;
                        }
                        _ => {
                            tracing::warn!(
                                msg_type = ?tunnel_msg.msg_type,
                                "unexpected tunnel message from agent"
                            );
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => { recv_task.abort(); }
        _ = (&mut recv_task) => { send_task.abort(); }
    }

    state.tunnel_registry.unregister(host_id);
    tracing::info!(host_id = %host_id, "agent tunnel disconnected");
}
