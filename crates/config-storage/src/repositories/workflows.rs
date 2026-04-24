use sqlx::PgPool;
use uuid::Uuid;

use crate::models::WorkflowRow;

pub struct WorkflowsRepo;

impl WorkflowsRepo {
    pub async fn insert(pool: &PgPool, workflow: &WorkflowRow) -> anyhow::Result<WorkflowRow> {
        let row = sqlx::query_as::<_, WorkflowRow>(
            "INSERT INTO workflows (
                workflow_id, status, repo_url, branch_name, base_branch, pr_title, pr_description,
                file_changes_json, error_message, pr_url, reviewers_json, event_ids_json
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING *"
        )
        .bind(workflow.workflow_id)
        .bind(&workflow.status)
        .bind(&workflow.repo_url)
        .bind(&workflow.branch_name)
        .bind(&workflow.base_branch)
        .bind(&workflow.pr_title)
        .bind(&workflow.pr_description)
        .bind(&workflow.file_changes_json)
        .bind(&workflow.error_message)
        .bind(&workflow.pr_url)
        .bind(&workflow.reviewers_json)
        .bind(&workflow.event_ids_json)
        .fetch_one(pool)
        .await?;

        Ok(row)
    }

    pub async fn get(pool: &PgPool, workflow_id: Uuid) -> anyhow::Result<Option<WorkflowRow>> {
        let row = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows WHERE workflow_id = $1"
        )
        .bind(workflow_id)
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }

    pub async fn list(pool: &PgPool, limit: i64, offset: i64) -> anyhow::Result<Vec<WorkflowRow>> {
        let rows = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows ORDER BY created_at DESC LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    pub async fn update_status(
        pool: &PgPool,
        workflow_id: Uuid,
        status: &str,
        error_message: Option<&str>,
        pr_url: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE workflows SET status = $1, error_message = $2, pr_url = $3, updated_at = NOW() WHERE workflow_id = $4"
        )
        .bind(status)
        .bind(error_message)
        .bind(pr_url)
        .bind(workflow_id)
        .execute(pool)
        .await?;
        Ok(())
    }
}