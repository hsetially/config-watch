use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use config_storage::repositories::workflows::WorkflowsRepo;

use crate::content_resolver::ContentResolver;
use crate::git_ops;
use crate::github_client;
use crate::models::WorkflowRun;
use crate::models::WorkflowStatus;

pub async fn run_workflow(
    run: WorkflowRun,
    pool: PgPool,
    resolver: Arc<dyn ContentResolver>,
) {
    let workflow_id = run.workflow_id;

    if let Err(e) = run_workflow_inner(run, &pool, resolver.as_ref()).await {
        tracing::error!(workflow_id = %workflow_id, error = format!("{:#}", e), "workflow failed");
        let _ = WorkflowsRepo::update_status(&pool, workflow_id, WorkflowStatus::Failed.as_str(), Some(&format!("{:#}", e)), None).await;
    }
}

async fn run_workflow_inner(
    run: WorkflowRun,
    pool: &PgPool,
    resolver: &dyn ContentResolver,
) -> anyhow::Result<()> {
    let workflow_id = run.workflow_id;

    // 1. Cloning / Pulling
    update_status(pool, workflow_id, WorkflowStatus::Cloning).await?;
    let repos_dir = std::path::Path::new(&run.repos_dir);
    let repo = git_ops::open_or_clone_repo(&run.repo_url, repos_dir, &run.base_branch, run.github_token.as_deref())
        .map_err(|e| anyhow::anyhow!("open/pull repo failed: {}", e))?;

    // 2. Applying — search for files in repo by basename
    update_status(pool, workflow_id, WorkflowStatus::Applying).await?;

    // Resolve content hashes from the database if not provided by the client
    let mut file_changes = run.file_changes;
    if file_changes.iter().any(|c| c.content_hash.is_none()) && !run.event_ids.is_empty() {
        match config_storage::repositories::change_events::ChangeEventsRepo
            ::get_content_hashes_by_event_ids(pool, &run.event_ids)
            .await
        {
            Ok(rows) => {
                let map: std::collections::HashMap<String, (String, Option<String>)> = rows
                    .into_iter()
                    .filter_map(|(path, hash, prev_hash)| hash.map(|h| (path, (h, prev_hash))))
                    .collect();
                for change in &mut file_changes {
                    if change.content_hash.is_none() {
                        if let Some((hash, prev_hash)) = map.get(&change.canonical_path) {
                            change.content_hash = Some(hash.clone());
                            change.previous_content_hash = prev_hash.clone();
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to resolve content hashes from events");
            }
        }
    }

    git_ops::apply_changes(&repo, &file_changes, resolver)
        .map_err(|e| anyhow::anyhow!("apply changes failed: {}", e))?;

    // 3. Committing — create branch from base_branch
    update_status(pool, workflow_id, WorkflowStatus::Committing).await?;
    let commit_msg = format!("config-watch: {}\n\n{}", run.pr_title, run.pr_description.as_deref().unwrap_or(""));
    git_ops::commit_changes(&repo, &run.branch_name, &run.base_branch, &commit_msg)
        .map_err(|e| anyhow::anyhow!("commit failed: {}", e))?;

    // 4. Pushing
    update_status(pool, workflow_id, WorkflowStatus::Pushing).await?;
    git_ops::push_branch(&repo, &run.branch_name, run.github_token.as_deref())
        .map_err(|e| anyhow::anyhow!("push failed: {:#}", e))?;

    // 5. Creating PR — use base_branch as the PR base
    update_status(pool, workflow_id, WorkflowStatus::CreatingPR).await?;
    let (owner, repo_name) = github_client::parse_owner_repo(&run.repo_url)?;
    let pr_result = github_client::create_pr(
        run.github_token.as_deref().unwrap_or(""),
        &owner,
        &repo_name,
        &run.pr_title,
        &run.branch_name,
        &run.base_branch,
        run.pr_description.as_deref(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("create PR failed: {}", e))?;

    // Add reviewers if specified (best-effort)
    if let Some(ref reviewers) = run.reviewers {
        if !reviewers.is_empty() {
            if let Err(e) = github_client::add_reviewers(
                run.github_token.as_deref().unwrap_or(""),
                &owner,
                &repo_name,
                pr_result.number,
                reviewers,
            ).await {
                tracing::warn!(error = %e, "failed to add reviewers");
            }
        }
    }

    let pr_url = pr_result.html_url;
    let pr_number = pr_result.number;

    // 6. Completed
    WorkflowsRepo::update_status(pool, workflow_id, WorkflowStatus::Completed.as_str(), None, Some(&pr_url))
        .await
        .map_err(|e| anyhow::anyhow!("update status failed: {}", e))?;

    // Update associated change events with PR info
    if !run.event_ids.is_empty() {
        if let Err(e) = config_storage::repositories::change_events::ChangeEventsRepo::update_pr_url_batch(
            pool, &run.event_ids, &pr_url, pr_number,
        ).await {
            tracing::warn!(error = %e, "failed to update change events with pr_url");
        }
    }

    tracing::info!(workflow_id = %workflow_id, pr_url = %pr_url, "workflow completed");
    Ok(())
}

async fn update_status(pool: &PgPool, workflow_id: Uuid, status: WorkflowStatus) -> anyhow::Result<()> {
    tracing::info!(workflow_id = %workflow_id, status = status.as_str(), "workflow progress");
    WorkflowsRepo::update_status(pool, workflow_id, status.as_str(), None, None).await?;
    Ok(())
}