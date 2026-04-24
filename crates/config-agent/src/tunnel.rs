use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{tungstenite::client::IntoClientRequest, connect_async};

use config_transport::tunnel::{QueryKind, QueryRequestPayload, QueryResponsePayload, TunnelMessage, TunnelMessageType};

use crate::config::AgentConfig;
use crate::query_handler::QueryHandler;

pub struct AgentTunnel {
    config: AgentConfig,
    query_handler: Arc<QueryHandler>,
}

impl AgentTunnel {
    pub fn new(config: AgentConfig, query_handler: Arc<QueryHandler>) -> Self {
        Self { config, query_handler }
    }

    pub async fn run(&self, auth_token: String) {
        let ws_url = derive_ws_url(&self.config.control_plane_base_url);
        let mut backoff_secs = self.config.tunnel_reconnect_base_secs;
        let max_backoff = self.config.tunnel_reconnect_max_secs;

        loop {
            match self.connect_and_serve(&ws_url, &auth_token).await {
                Ok(()) => {
                    tracing::info!("tunnel disconnected, reconnecting");
                    backoff_secs = self.config.tunnel_reconnect_base_secs;
                }
                Err(e) => {
                    tracing::warn!(error = %e, backoff_secs, "tunnel connection failed, reconnecting");
                }
            }

            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(max_backoff);
        }
    }

    async fn connect_and_serve(&self, ws_url: &str, auth_token: &str) -> anyhow::Result<()> {
        let mut request = ws_url.into_client_request()?;
        let headers = request.headers_mut();
        headers.insert("X-Agent-Token", auth_token.parse()?);

        let (stream, _) = connect_async(request).await?;
        let (mut sink, mut stream) = stream.split();

        tracing::info!(url = %ws_url, "tunnel connected to control plane");

        while let Some(msg) = stream.next().await {
            let msg = msg?;

            match msg {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    let tunnel_msg: TunnelMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to parse tunnel message");
                            continue;
                        }
                    };

                    match tunnel_msg.msg_type {
                        TunnelMessageType::QueryRequest => {
                            let request_id = tunnel_msg.request_id.clone().unwrap_or_default();
                            let payload: QueryRequestPayload = tunnel_msg
                                .payload
                                .and_then(|v| serde_json::from_value(v).ok())
                                .unwrap_or(QueryRequestPayload {
                                    kind: QueryKind::Stat,
                                    path: String::new(),
                                    offset: None,
                                    limit: None,
                                });

                            let response = self.handle_query(&payload).await;
                            let msg = TunnelMessage::query_response(request_id, response);
                            let json = serde_json::to_string(&msg)?;
                            sink.send(tokio_tungstenite::tungstenite::Message::Text(json)).await?;
                        }
                        TunnelMessageType::Ping => {
                            let pong = TunnelMessage::pong();
                            let json = serde_json::to_string(&pong)?;
                            sink.send(tokio_tungstenite::tungstenite::Message::Text(json)).await?;
                        }
                        TunnelMessageType::Pong => {
                            tracing::trace!("tunnel pong received");
                        }
                        _ => {
                            tracing::warn!(msg_type = ?tunnel_msg.msg_type, "unexpected message from control plane");
                        }
                    }
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => {
                    tracing::info!("tunnel closed by control plane");
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_query(&self, request: &QueryRequestPayload) -> QueryResponsePayload {
        match request.kind {
            QueryKind::Stat => {
                map_query_result(self.query_handler.stat(&request.path).await)
            }
            QueryKind::Preview => {
                map_query_result(self.query_handler.preview(&request.path))
            }
            QueryKind::Content => {
                map_query_result(self.query_handler.content(&request.path, request.offset, request.limit))
            }
        }
    }
}

fn map_query_result<T: serde::Serialize>(result: anyhow::Result<T>) -> QueryResponsePayload {
    match result {
        Ok(data) => QueryResponsePayload {
            status: "success".into(),
            data: Some(serde_json::to_value(&data).unwrap_or_default()),
            error: None,
        },
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("denied") { "denied" } else { "error" };
            QueryResponsePayload {
                status: status.into(),
                data: None,
                error: Some(msg),
            }
        }
    }
}

fn derive_ws_url(base_url: &str) -> String {
    let url = if base_url.starts_with("https://") {
        base_url.replacen("https://", "wss://", 1)
    } else if base_url.starts_with("http://") {
        base_url.replacen("http://", "ws://", 1)
    } else {
        format!("ws://{}", base_url)
    };
    format!("{}/v1/agents/tunnel", url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_ws_url_http() {
        assert_eq!(
            derive_ws_url("http://127.0.0.1:8082"),
            "ws://127.0.0.1:8082/v1/agents/tunnel"
        );
    }

    #[test]
    fn test_derive_ws_url_https() {
        assert_eq!(
            derive_ws_url("https://cp.example.com"),
            "wss://cp.example.com/v1/agents/tunnel"
        );
    }

    #[test]
    fn test_derive_ws_url_no_scheme() {
        assert_eq!(
            derive_ws_url("cp.example.com:8082"),
            "ws://cp.example.com:8082/v1/agents/tunnel"
        );
    }

    #[test]
    fn test_derive_ws_url_trailing_slash() {
        assert_eq!(
            derive_ws_url("http://127.0.0.1:8082/"),
            "ws://127.0.0.1:8082/v1/agents/tunnel"
        );
    }
}