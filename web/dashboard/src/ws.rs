use futures::StreamExt;
use gloo_net::websocket::{futures::WebSocket, Message};
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::models::{ConnectionStatus, FilterState, RealtimeMessage, WsMessage, WsMessageType};
use crate::url;

/// Response from the WS ticket endpoint.
#[derive(serde::Deserialize)]
struct WsTicketResponse {
    ticket: String,
}

pub fn connect(
    _base_url: &str,
    filters: &FilterState,
    csrf_token: Option<&str>,
    on_message: Callback<RealtimeMessage>,
    on_status: Callback<ConnectionStatus>,
) {
    let query_string = filters.to_query_string();
    let csrf_token_owned = csrf_token.map(|s| s.to_string());

    let on_msg = on_message.clone();
    let on_stat = on_status.clone();

    spawn_local(async move {
        on_stat.emit(ConnectionStatus::Connecting);

        // C4: Obtain a one-shot WS ticket via authenticated POST (with CSRF header).
        // The browser sends the session cookie automatically.
        let ticket = match fetch_ws_ticket(csrf_token_owned.as_deref()).await {
            Ok(t) => t,
            Err(e) => {
                on_stat.emit(ConnectionStatus::Error(format!(
                    "Failed to obtain WS ticket: {}",
                    e
                )));
                return;
            }
        };

        // Connect WebSocket with ?ticket=<one-shot-ticket> instead of ?token=<session>
        let ws_url = if query_string.is_empty() {
            url::ws_url("/v1/changes/stream", &format!("ticket={}", ticket))
        } else {
            url::ws_url(
                "/v1/changes/stream",
                &format!("{}&ticket={}", query_string, ticket),
            )
        };

        let ws = match WebSocket::open(&ws_url) {
            Ok(ws) => ws,
            Err(e) => {
                on_stat.emit(ConnectionStatus::Error(format!(
                    "WebSocket open failed: {}",
                    e
                )));
                return;
            }
        };

        on_stat.emit(ConnectionStatus::Connected);

        let (_, mut read) = ws.split();

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let parsed: Result<WsMessage, _> = serde_json::from_str(&text);
                    match parsed {
                        Ok(ws_msg) => match ws_msg.msg_type {
                            WsMessageType::Change => {
                                if let Some(event) = ws_msg.event {
                                    on_msg.emit(event);
                                }
                            }
                            WsMessageType::Gap => {
                                gloo::console::warn!(
                                    "WebSocket gap detected, some events may have been missed"
                                );
                            }
                            WsMessageType::Heartbeat => {}
                        },
                        Err(e) => {
                            gloo::console::warn!("Failed to parse WS message:", &e.to_string());
                        }
                    }
                }
                Ok(Message::Bytes(_)) => {
                    // Ignore binary messages
                }
                Err(e) => {
                    on_stat.emit(ConnectionStatus::Error(format!("WebSocket error: {}", e)));
                    break;
                }
            }
        }

        on_stat.emit(ConnectionStatus::Disconnected);
    });
}

/// Fetch a one-shot WS ticket from the server using cookie-based auth + CSRF.
async fn fetch_ws_ticket(csrf_token: Option<&str>) -> Result<String, String> {
    let ticket_url = url::api_url("/v1/ws-ticket");
    let mut req = gloo_net::http::Request::post(&ticket_url)
        .credentials(web_sys::RequestCredentials::Include);
    if let Some(csrf) = csrf_token {
        req = req.header("x-csrf-token", csrf);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.ok() {
        return Err(format!("WS ticket request failed (HTTP {})", resp.status()));
    }

    let ticket_data: WsTicketResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse WS ticket response: {}", e))?;

    Ok(ticket_data.ticket)
}
