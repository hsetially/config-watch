use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite};

pub async fn tail_changes(
    base_url: &str,
    env: Option<String>,
    host_id: Option<String>,
    path_prefix: Option<String>,
    show_diff: bool,
) -> Result<()> {
    let ws_url = base_url
        .replace("http://", "ws://")
        .replace("https://", "wss://");

    let mut url = format!("{}/v1/changes/stream", ws_url);
    let mut params = Vec::new();
    if let Some(ref e) = env {
        params.push(format!("environment={}", e));
    }
    if let Some(ref h) = host_id {
        params.push(format!("host_id={}", h));
    }
    if let Some(ref p) = path_prefix {
        params.push(format!("path_prefix={}", p));
    }
    if !params.is_empty() {
        url.push_str(&format!("?{}", params.join("&")));
    }

    println!("Connecting to {}...", url);

    loop {
        match connect_async(&url).await {
            Ok((mut ws_stream, _)) => {
                println!("Connected. Streaming change events...\n");
                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(tungstenite::Message::Text(text)) => {
                            let parsed: serde_json::Value = match serde_json::from_str(&text) {
                                Ok(v) => v,
                                Err(_) => {
                                    println!("{}", text);
                                    continue;
                                }
                            };

                            let msg_type = parsed.get("msg_type").and_then(|v| v.as_str()).unwrap_or("");

                            match msg_type {
                                "Change" => {
                                    if let Some(event) = parsed.get("event") {
                                        let _event_id = event.get("event_id").and_then(|v| v.as_str()).unwrap_or("-");
                                        let host_id = event.get("host_id").and_then(|v| v.as_str()).unwrap_or("-");
                                        let env = event.get("environment").and_then(|v| v.as_str()).unwrap_or("-");
                                        let path = event.get("path").and_then(|v| v.as_str()).unwrap_or("-");
                                        let kind = event.get("event_kind").and_then(|v| v.as_str()).unwrap_or("-");
                                        let severity = event.get("severity").and_then(|v| v.as_str()).unwrap_or("-");
                                        let time = event.get("event_time").and_then(|v| v.as_str()).unwrap_or("-");
                                        let author = event.get("author_display").and_then(|v| v.as_str()).unwrap_or("-");

                                        let severity_marker = match severity {
                                            "critical" => "[CRIT]",
                                            "warning" => "[WARN]",
                                            _ => "[INFO]",
                                        };

                                        println!(
                                            "{} {} {} {} {} {} author={}",
                                            severity_marker, time, kind, env, host_id, path, author
                                        );

                                        if show_diff {
                                            if let Some(diff) = event.get("diff_render").and_then(|v| v.as_str()) {
                                                if !diff.is_empty() {
                                                    println!("--- diff ---");
                                                    for line in diff.lines() {
                                                        println!("  {}", line);
                                                    }
                                                    println!("--- end diff ---");
                                                }
                                            }
                                        }
                                    }
                                }
                                "Gap" => {
                                    println!("[GAP] Some events may have been missed due to lag");
                                }
                                _ => {
                                    println!("{}", text);
                                }
                            }
                        }
                        Ok(tungstenite::Message::Close(_)) => {
                            println!("\nConnection closed by server.");
                            break;
                        }
                        Err(e) => {
                            eprintln!("\nWebSocket error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                eprintln!("Connection failed: {}. Retrying in 5s...", e);
            }
        }

        println!("Reconnecting in 5s...");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}