use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Pending,
    Cloning,
    Applying,
    Committing,
    Pushing,
    CreatingPR,
    Completed,
    Failed,
}

impl WorkflowStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Cloning => "cloning",
            Self::Applying => "applying",
            Self::Committing => "committing",
            Self::Pushing => "pushing",
            Self::CreatingPR => "creating_pr",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub canonical_path: String,
    pub content_hash: Option<String>,
    pub previous_content_hash: Option<String>,
    pub event_kind: String,
    pub repo_filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub workflow_id: Uuid,
    pub repo_url: String,
    pub branch_name: String,
    pub base_branch: String,
    pub pr_title: String,
    pub pr_description: Option<String>,
    pub file_changes: Vec<FileChange>,
    pub reviewers: Option<Vec<String>>,
    pub repos_dir: String,
    #[serde(skip_serializing)]
    pub github_token: Option<String>,
    pub event_ids: Vec<Uuid>,
}