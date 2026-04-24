use anyhow::Result;

pub async fn list_hosts(base_url: &str, status_filter: Option<&str>) -> Result<()> {
    let url = format!("{}/v1/hosts", base_url);
    let resp = reqwest::get(&url).await?;
    let body: serde_json::Value = resp.json().await?;

    let hosts = body
        .get("hosts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if hosts.is_empty() {
        println!("No hosts registered.");
        return Ok(());
    }

    println!(
        "{:<40} {:<20} {:<15} {:<12} {:<20}",
        "HOST_ID", "HOSTNAME", "ENVIRONMENT", "STATUS", "LAST_HEARTBEAT"
    );
    println!("{}", "-".repeat(110));

    for host in &hosts {
        let host_id = host.get("host_id").and_then(|v| v.as_str()).unwrap_or("-");
        let hostname = host.get("hostname").and_then(|v| v.as_str()).unwrap_or("-");
        let env = host
            .get("environment")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let status = host.get("status").and_then(|v| v.as_str()).unwrap_or("-");
        let last_hb = host
            .get("last_heartbeat_at")
            .and_then(|v| v.as_str())
            .unwrap_or("-");

        if let Some(filter) = status_filter {
            if status != filter {
                continue;
            }
        }

        println!(
            "{:<40} {:<20} {:<15} {:<12} {:<20}",
            host_id, hostname, env, status, last_hb
        );
    }

    Ok(())
}
