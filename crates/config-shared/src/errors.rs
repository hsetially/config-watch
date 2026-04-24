use thiserror::Error;

use crate::ids::EventId;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("path not allowed: {path} ({reason})")]
    PathNotAllowed { path: String, reason: String },

    #[error("snapshot failed for {path}")]
    SnapshotFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("diff failed for {path}: {reason}")]
    DiffFailed { path: String, reason: String },

    #[error("spool full: {current}/{max} events")]
    SpoolFull { current: usize, max: usize },

    #[error("publish failed for event {event_id}: HTTP {status}")]
    PublishFailed { event_id: EventId, status: u16 },

    #[error("{entity} not found: {id}")]
    NotFound { entity: String, id: String },

    #[error("unauthorized: {action} on {subject}")]
    Unauthorized { action: String, subject: String },
}

impl AppError {
    pub fn http_status(&self) -> u16 {
        match self {
            Self::Validation(_) => 400,
            Self::PathNotAllowed { .. } => 403,
            Self::SnapshotFailed { .. } => 500,
            Self::DiffFailed { .. } => 500,
            Self::SpoolFull { .. } => 507,
            Self::PublishFailed { status, .. } => *status,
            Self::NotFound { .. } => 404,
            Self::Unauthorized { .. } => 401,
        }
    }
}