use anyhow::Context;
use git2::{Cred, RemoteCallbacks, Repository};
use std::path::Path;

use crate::content_resolver::ContentResolver;
use crate::models::FileChange;

/// Derive a directory name from a repo URL (e.g. "https://github.com/myorg/myrepo.git" → "myorg-myrepo").
pub fn repo_dir_name(repo_url: &str) -> String {
    let url = repo_url.trim_end_matches(".git").trim_end_matches('/');
    let parts: Vec<&str> = url.rsplitn(3, '/').collect();
    if parts.len() >= 2 {
        format!("{}-{}", parts[1], parts[0])
    } else {
        url.replace([':', '/', '.'], "_")
    }
}

/// Open an existing repo and pull, or clone fresh if not present.
pub fn open_or_clone_repo(
    url: &str,
    repos_dir: &Path,
    base_branch: &str,
    token: Option<&str>,
) -> anyhow::Result<Repository> {
    let dir_name = repo_dir_name(url);
    let repo_path = repos_dir.join(&dir_name);

    if repo_path.join(".git").exists() {
        match Repository::open(&repo_path) {
            Ok(repo) => {
                if let Err(e) = pull_repo(&repo, base_branch, token) {
                    tracing::warn!(error = %e, "pull failed, re-cloning");
                    std::fs::remove_dir_all(&repo_path).ok();
                    std::fs::create_dir_all(&repo_path).context("create repos dir")?;
                    return clone_repo(url, &repo_path, token);
                }
                Ok(repo)
            }
            Err(e) => {
                tracing::warn!(error = %e, "open failed, re-cloning");
                std::fs::remove_dir_all(&repo_path).ok();
                std::fs::create_dir_all(&repo_path).context("create repos dir")?;
                clone_repo(url, &repo_path, token)
            }
        }
    } else {
        std::fs::create_dir_all(&repo_path).context("create repos dir")?;
        clone_repo(url, &repo_path, token)
    }
}

pub fn pull_repo(repo: &Repository, base_branch: &str, token: Option<&str>) -> anyhow::Result<()> {
    let mut callbacks = RemoteCallbacks::new();
    if let Some(t) = token {
        let token = t.to_string();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            Cred::userpass_plaintext("x-access-token", &token)
        });
    }

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let mut remote = repo.find_remote("origin").context("find origin")?;
    remote
        .fetch(&[base_branch], Some(&mut fetch_options), None)
        .context("git fetch")?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .context("find FETCH_HEAD")?;
    let fetch_commit = fetch_head.peel_to_commit().context("peel FETCH_HEAD")?;

    // Update the remote tracking ref
    let target_ref = format!("refs/remotes/origin/{base_branch}");
    if let Ok(mut r) = repo.find_reference(&target_ref) {
        r.set_target(fetch_commit.id(), "update remote ref")?;
    }

    // Hard reset to the fetched commit for a clean working tree
    repo.reset(fetch_commit.as_object(), git2::ResetType::Hard, None)
        .context("git reset --hard")?;

    Ok(())
}

pub fn clone_repo(url: &str, dest: &Path, token: Option<&str>) -> anyhow::Result<Repository> {
    let mut callbacks = RemoteCallbacks::new();
    if let Some(t) = token {
        let token = t.to_string();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            Cred::userpass_plaintext("x-access-token", &token)
        });
    }

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_options);

    builder.clone(url, dest).context("git clone failed")
}

/// Walk the repo working tree and find a file whose basename matches `filename`.
/// Returns the relative path from the repo root, or None if not found.
pub fn find_file_in_repo(repo: &Repository, filename: &str) -> Option<String> {
    let workdir = repo.workdir()?;
    let mut result: Option<String> = None;
    if let Ok(entries) = walk_dir_recursive(workdir) {
        for rel in entries {
            let basename = Path::new(&rel)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if basename == filename {
                result = Some(rel);
                break;
            }
        }
    }
    result
}

fn walk_dir_recursive(dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut results = Vec::new();
    walk_dir_recursive_inner(dir, dir, &mut results)?;
    Ok(results)
}

fn walk_dir_recursive_inner(
    base: &Path,
    dir: &Path,
    results: &mut Vec<String>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let dirname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dirname == ".git" {
                continue;
            }
            walk_dir_recursive_inner(base, &path, results)?;
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                if let Some(s) = rel.to_str() {
                    results.push(s.replace('\\', "/"));
                }
            }
        }
    }
    Ok(())
}

pub fn apply_changes(
    repo: &Repository,
    changes: &[FileChange],
    resolver: &dyn ContentResolver,
) -> anyhow::Result<()> {
    for change in changes {
        // Use repo_filename if provided, otherwise extract basename from canonical path
        let filename = change
            .repo_filename
            .as_deref()
            .or_else(|| change.canonical_path.rsplit('/').next())
            .unwrap_or(&change.canonical_path);

        // Search the repo for an existing file with the same basename
        let repo_relative = find_file_in_repo(repo, filename)
            .ok_or_else(|| anyhow::anyhow!("file '{}' not found in repo", filename))?;

        let file_path = repo.workdir().context("no workdir")?.join(&repo_relative);

        match change.event_kind.as_str() {
            "deleted" => {
                if file_path.exists() {
                    std::fs::remove_file(&file_path).context("remove file")?;
                    let rel = Path::new(&repo_relative);
                    let _ = repo
                        .index()
                        .and_then(|mut idx| idx.remove_path(rel).map(|_| idx.write()))
                        .context("git rm")?;
                }
            }
            _ => {
                // When a previous snapshot is available, verify the repo file matches it.
                // If it does, writing the current snapshot produces a commit with only
                // this event's changes. If not, fall back to full snapshot write.
                let wrote_exact = if let Some(ref prev_hash) = change.previous_content_hash {
                    match write_event_content(
                        repo,
                        &file_path,
                        &repo_relative,
                        resolver,
                        &change.canonical_path,
                        prev_hash,
                        change.content_hash.as_deref(),
                    ) {
                        Ok(true) => {
                            tracing::debug!(path = %change.canonical_path, "wrote exact event content");
                            true
                        }
                        Ok(false) => {
                            tracing::warn!(path = %change.canonical_path, "repo file diverged from previous snapshot; PR will include cumulative changes");
                            false
                        }
                        Err(e) => {
                            tracing::warn!(path = %change.canonical_path, error = %e, "exact write failed, falling back to full content");
                            false
                        }
                    }
                } else {
                    false
                };

                if !wrote_exact {
                    let content = resolver
                        .resolve(&change.canonical_path, change.content_hash.as_deref())
                        .context(format!("resolve content for {}", change.canonical_path))?
                        .context(format!("no content for {}", change.canonical_path))?;

                    if let Some(parent) = file_path.parent() {
                        std::fs::create_dir_all(parent).context("create dir")?;
                    }
                    std::fs::write(&file_path, &content).context("write file")?;
                }

                let rel = Path::new(&repo_relative);
                let _ = repo
                    .index()
                    .and_then(|mut idx| idx.add_path(rel).map(|_| idx.write()))
                    .context("git add")?;
            }
        }
    }
    Ok(())
}

/// If the repo file matches the previous snapshot, write the current snapshot so the
/// commit diff contains only this event's changes. Returns true if the exact write
/// was performed.
fn write_event_content(
    _repo: &Repository,
    file_path: &Path,
    _repo_relative: &str,
    resolver: &dyn ContentResolver,
    canonical_path: &str,
    previous_hash: &str,
    current_hash: Option<&str>,
) -> anyhow::Result<bool> {
    let previous_content = resolver
        .resolve(canonical_path, Some(previous_hash))
        .with_context(|| format!("resolve previous content for {}", canonical_path))?
        .with_context(|| format!("no previous content for {}", canonical_path))?;

    let current_content = resolver
        .resolve(canonical_path, current_hash)
        .with_context(|| format!("resolve current content for {}", canonical_path))?
        .with_context(|| format!("no current content for {}", canonical_path))?;

    let repo_content = if file_path.exists() {
        std::fs::read(file_path).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Compare ignoring line endings so CRLF vs LF doesn't cause false mismatches
    if normalize_newlines(&repo_content) != normalize_newlines(&previous_content) {
        tracing::debug!(
            path = %canonical_path,
            repo_bytes = repo_content.len(),
            prev_bytes = previous_content.len(),
            "repo file does not match previous snapshot"
        );
        return Ok(false);
    }

    // Repo file matches previous snapshot — write current snapshot for a clean diff
    std::fs::write(file_path, &current_content)
        .with_context(|| format!("write current content to {}", file_path.display()))?;

    Ok(true)
}

fn normalize_newlines(content: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(content)
        .replace("\r\n", "\n")
        .into_bytes()
}

/// Create a new branch from `base_branch` and commit the staged changes.
pub fn commit_changes(
    repo: &Repository,
    branch_name: &str,
    base_branch: &str,
    message: &str,
) -> anyhow::Result<git2::Oid> {
    // Resolve the base branch (e.g. "main") — try remote ref first, then local
    let base_commit =
        if let Ok(refname) = repo.find_reference(&format!("refs/remotes/origin/{base_branch}")) {
            refname
                .peel_to_commit()
                .context(format!("peel origin/{} to commit", base_branch))?
        } else if let Ok(refname) = repo.find_reference(&format!("refs/heads/{base_branch}")) {
            refname
                .peel_to_commit()
                .context(format!("peel {} to commit", base_branch))?
        } else {
            // Fallback: try HEAD (the default clone checkout)
            let head = repo.head().context("get HEAD")?;
            head.peel_to_commit().context("peel HEAD to commit")?
        };

    let branch = repo
        .branch(branch_name, &base_commit, false)
        .context("create branch")?;

    let tree_id = repo
        .index()
        .and_then(|mut idx| idx.write_tree())
        .context("write tree")?;
    let tree = repo.find_tree(tree_id).context("find tree")?;
    let sig = repo.signature().unwrap_or_else(|_| {
        git2::Signature::now("config-watch", "bot@config-watch.local").unwrap()
    });

    let branch_ref = branch.get().name().context("branch name")?;
    let oid = repo
        .commit(
            Some(branch_ref),
            &sig,
            &sig,
            message,
            &tree,
            &[&base_commit],
        )
        .context("git commit")?;

    repo.set_head(branch_ref).context("set HEAD to branch")?;
    Ok(oid)
}

pub fn push_branch(
    repo: &Repository,
    branch_name: &str,
    token: Option<&str>,
) -> anyhow::Result<()> {
    let mut remote = repo.find_remote("origin").context("find origin")?;

    let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
    let remote_url = remote.url().unwrap_or("");
    tracing::debug!(refspec = %refspec, has_token = token.is_some(), remote_url = %remote_url, "pushing branch");

    if token.is_none() && (remote_url.starts_with("https://") || remote_url.starts_with("http://"))
    {
        anyhow::bail!("github_token is required to push to HTTPS remotes");
    }

    if remote_url.starts_with("git@") || remote_url.starts_with("ssh://") {
        anyhow::bail!(
            "SSH remote URLs are not supported for push; use an HTTPS URL with a github_token"
        );
    }

    let mut callbacks = RemoteCallbacks::new();
    if let Some(t) = token {
        let token = t.to_string();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            Cred::userpass_plaintext("x-access-token", &token)
        });
    }
    callbacks.push_update_reference(|refname, status| {
        if let Some(s) = status {
            Err(git2::Error::from_str(&format!(
                "push rejected for {}: {}",
                refname, s
            )))
        } else {
            Ok(())
        }
    });

    let mut push_options = git2::PushOptions::new();
    push_options.remote_callbacks(callbacks);

    remote
        .push(&[&refspec], Some(&mut push_options))
        .with_context(|| {
            format!(
                "git push {} to {}",
                refspec,
                remote.url().unwrap_or("(no url)")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_dir_name() {
        assert_eq!(
            repo_dir_name("https://github.com/myorg/myrepo.git"),
            "myorg-myrepo"
        );
        assert_eq!(
            repo_dir_name("https://github.com/myorg/myrepo"),
            "myorg-myrepo"
        );
        assert_eq!(
            repo_dir_name("https://github.com/my-org/my-repo.git"),
            "my-org-my-repo"
        );
    }

    #[test]
    fn test_repo_dir_name_fallback() {
        let result = repo_dir_name("not-a-url");
        assert!(!result.is_empty());
    }
}
