use futures::StreamExt;
use gloo_net::websocket::{futures::WebSocket, Message};
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::models::{ConnectionStatus, FilterState, RealtimeMessage, WsMessage, WsMessageType};

pub fn connect(
    base_url: &str,
    filters: &FilterState,
    on_message: Callback<RealtimeMessage>,
    on_status: Callback<ConnectionStatus>,
) {
    let ws_url = format!(
        "wss://{}/v1/changes/stream{}",
        base_url,
        filters.to_query_string()
    );
    let on_msg = on_message.clone();
    let on_stat = on_status.clone();

    spawn_local(async move {
        on_stat.emit(ConnectionStatus::Connecting);

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
                                // Client is lagging, could request full refresh
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
