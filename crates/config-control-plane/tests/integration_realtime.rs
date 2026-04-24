use config_control_plane::realtime::RealtimeService;
use config_transport::websocket::RealtimeMessage;
use uuid::Uuid;

fn make_message(path: &str) -> RealtimeMessage {
    RealtimeMessage {
        event_id: Uuid::new_v4(),
        host_id: Uuid::new_v4(),
        environment: "default".to_string(),
        path: path.to_string(),
        event_kind: "modified".to_string(),
        event_time: chrono::Utc::now().to_rfc3339(),
        severity: "info".to_string(),
        author_display: None,
        summary: None,
        diff_render: None,
        pr_url: None,
        pr_number: None,
    }
}

#[tokio::test]
async fn realtime_service_broadcast_to_subscriber() {
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);
    let mut rx = broadcast_tx.subscribe();
    let service = RealtimeService::new(broadcast_tx);

    let msg = make_message("/etc/test.yaml");

    service.publish(msg.clone());

    let received = rx.try_recv().unwrap();
    assert_eq!(received.event_id, msg.event_id);
    assert_eq!(received.path, "/etc/test.yaml");
}

#[tokio::test]
async fn realtime_service_multiple_subscribers() {
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);
    let mut rx1 = broadcast_tx.subscribe();
    let mut rx2 = broadcast_tx.subscribe();
    let service = RealtimeService::new(broadcast_tx);

    let msg = make_message("/etc/multi.yaml");
    service.publish(msg.clone());

    let received1 = rx1.try_recv().unwrap();
    let received2 = rx2.try_recv().unwrap();
    assert_eq!(received1.event_id, msg.event_id);
    assert_eq!(received2.event_id, msg.event_id);
}

#[tokio::test]
async fn realtime_service_closed_sender_ends_stream() {
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);
    let mut rx = broadcast_tx.subscribe();
    let service = RealtimeService::new(broadcast_tx);

    // Drop the service (which drops the sender)
    drop(service);

    // Receiver should get a Closed error when trying to receive
    match rx.try_recv() {
        Ok(_) => {} // May have one message if published before drop
        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {}
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
            // Channel is empty but not yet closed in try_recv
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            match rx.try_recv() {
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                other => panic!("Expected Closed or Empty after drop, got {:?}", other),
            }
        }
        Err(other) => panic!("Unexpected error: {:?}", other),
    }
}