use anyhow::Context;

pub struct CreatePrResult {
    pub html_url: String,
    pub number: i64,
}

pub async fn create_pr(
    token: &str,
    owner: &str,
    repo_name: &str,
    title: &str,
    head: &str,
    base: &str,
    body: Option<&str>,
) -> anyhow::Result<CreatePrResult> {
    let url = format!("https://api.github.com/repos/{owner}/{repo_name}/pulls");

    let mut map = serde_json::Map::new();
    map.insert(
        "title".to_string(),
        serde_json::Value::String(title.to_string()),
    );
    map.insert(
        "head".to_string(),
        serde_json::Value::String(head.to_string()),
    );
    map.insert(
        "base".to_string(),
        serde_json::Value::String(base.to_string()),
    );
    if let Some(b) = body {
        map.insert("body".to_string(), serde_json::Value::String(b.to_string()));
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "config-watch")
        .json(&map)
        .send()
        .await
        .context("github api request")?;

    let status = resp.status();
    let text = resp.text().await.context("read response body")?;

    if !status.is_success() {
        anyhow::bail!("GitHub API error ({}): {}", status, text);
    }

    let json: serde_json::Value = serde_json::from_str(&text).context("parse github response")?;
    let html_url = json["html_url"]
        .as_str()
        .context("missing html_url in github response")?
        .to_string();
    let number = json["number"]
        .as_i64()
        .context("missing number in github response")?;

    Ok(CreatePrResult { html_url, number })
}

/// Request reviewers on a PR. Best-effort — errors are logged but not fatal.
pub async fn add_reviewers(
    token: &str,
    owner: &str,
    repo_name: &str,
    pr_number: i64,
    reviewers: &[String],
) -> anyhow::Result<()> {
    let url = format!(
        "https://api.github.com/repos/{owner}/{repo_name}/pulls/{pr_number}/requested_reviewers"
    );

    let body = serde_json::json!({
        "reviewers": reviewers
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "config-watch")
        .json(&body)
        .send()
        .await
        .context("github reviewers api request")?;

    let status = resp.status();
    let text = resp.text().await.context("read response body")?;

    if !status.is_success() {
        tracing::warn!(
            pr_number = pr_number,
            status = %status,
            body = %text,
            "failed to add reviewers (non-fatal)"
        );
    }

    Ok(())
}

pub fn parse_owner_repo(repo_url: &str) -> anyhow::Result<(String, String)> {
    let url = repo_url.trim_end_matches(".git").trim_end_matches('/');
    let parts: Vec<&str> = url.rsplitn(3, '/').collect();
    if parts.len() < 2 {
        anyhow::bail!("cannot parse owner/repo from URL: {}", repo_url);
    }
    let repo_name = parts[0].to_string();
    let owner = parts[1].to_string();
    Ok((owner, repo_name))
}

pub struct GitHubFileContent {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
    pub sha: Option<String>,
}

/// Fetch file contents from GitHub via the Contents API.
/// Token is optional — public repos work without one.
pub async fn fetch_file_contents(
    token: Option<&str>,
    owner: &str,
    repo: &str,
    path: &str,
    branch: &str,
) -> anyhow::Result<GitHubFileContent> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={branch}");

    let mut req = reqwest::Client::new()
        .get(&url)
        .header("Accept", "application/vnd.github.raw+json")
        .header("User-Agent", "config-watch");

    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }

    let resp = req.send().await.context("github contents api request")?;
    let status = resp.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("file not found on github: {path}");
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("github auth failed — configure github_token");
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("github api error ({}): {}", status, text);
    }

    let content = resp.text().await.context("read github response body")?;
    let size_bytes = content.len() as u64;

    Ok(GitHubFileContent {
        path: path.to_string(),
        content,
        size_bytes,
        sha: None,
    })
}

/// Parse a GitHub blob URL like `https://github.com/owner/repo/blob/branch/path/to/file.yaml`
/// Returns (owner, repo, branch, path).
///
/// Note: Branch names containing `/` are not reliably parseable from a URL alone.
/// This function splits at the first `/` after `/blob/`, which works for simple
/// branch names (main, develop, v1.0) but not for names like `feature/auth`.
/// For branches with `/`, users should use the raw comparison or the server
/// could fall back to trying progressively longer prefixes.
pub fn parse_github_blob_url(url: &str) -> anyhow::Result<(String, String, String, String)> {
    let url = url.trim().trim_end_matches('/');
    let blob_marker = "/blob/";
    let blob_pos = url
        .find(blob_marker)
        .ok_or_else(|| anyhow::anyhow!("not a github blob URL (missing /blob/): {}", url))?;

    let repo_part = &url[..blob_pos];
    let after_blob = &url[blob_pos + blob_marker.len()..];

    let (owner, repo) = parse_owner_repo(repo_part)?;

    let slash_pos = after_blob
        .find('/')
        .ok_or_else(|| anyhow::anyhow!("cannot separate branch from path in: {}", url))?;
    let branch = after_blob[..slash_pos].to_string();
    let path = after_blob[slash_pos + 1..].to_string();

    if path.is_empty() {
        anyhow::bail!("empty file path in github url: {}", url);
    }

    Ok((owner, repo, branch, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_owner_repo() {
        let (owner, repo) = parse_owner_repo("https://github.com/myorg/myrepo.git").unwrap();
        assert_eq!(owner, "myorg");
        assert_eq!(repo, "myrepo");

        let (owner, repo) = parse_owner_repo("https://github.com/myorg/myrepo").unwrap();
        assert_eq!(owner, "myorg");
        assert_eq!(repo, "myrepo");

        let (owner, repo) = parse_owner_repo("https://github.com/my-org/my-repo.git").unwrap();
        assert_eq!(owner, "my-org");
        assert_eq!(repo, "my-repo");
    }

    #[test]
    fn test_parse_owner_repo_invalid() {
        assert!(parse_owner_repo("not-a-url").is_err());
    }

    #[test]
    fn test_parse_github_blob_url() {
        let (owner, repo, branch, path) =
            parse_github_blob_url("https://github.com/myorg/myrepo/blob/main/config/app.yaml")
                .unwrap();
        assert_eq!(owner, "myorg");
        assert_eq!(repo, "myrepo");
        assert_eq!(branch, "main");
        assert_eq!(path, "config/app.yaml");
    }

    #[test]
    fn test_parse_github_blob_url_feature_branch() {
        // Branch names with / are not reliably parseable from URL alone.
        // The function splits at the first /, treating "feature" as the branch
        // and "auth/config.toml" as the path. This is a known limitation.
        let (owner, repo, branch, path) =
            parse_github_blob_url("https://github.com/myorg/myrepo/blob/feature/auth/config.toml")
                .unwrap();
        assert_eq!(owner, "myorg");
        assert_eq!(repo, "myrepo");
        assert_eq!(branch, "feature");
        assert_eq!(path, "auth/config.toml");
    }

    #[test]
    fn test_parse_github_blob_url_invalid() {
        assert!(parse_github_blob_url("https://github.com/owner/repo").is_err());
        assert!(parse_github_blob_url("https://github.com/owner/repo/blob/main").is_err());
        assert!(parse_github_blob_url("not-a-url").is_err());
    }
}
