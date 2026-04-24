use anyhow::Result;

pub async fn file_stat(base_url: &str, host_id: &str, path: &str) -> Result<()> {
    let url = format!("{}/v1/file/stat", base_url);
    let body = serde_json::json!({
        "host_id": host_id,
        "path": path,
    });

    let resp = reqwest::Client::new().post(&url).json(&body).send().await?;

    let status = resp.status();
    let result: serde_json::Value = resp.json().await?;

    if !status.is_success() {
        let error = result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        eprintln!("Error ({}): {}", status, error);
        return Ok(());
    }

    println!("{:<20} VALUE", "FIELD");
    println!("{}", "-".repeat(60));

    if let Some(obj) = result.as_object() {
        for (key, value) in obj {
            println!("{:<20} {}", key, value);
        }
    }

    Ok(())
}
