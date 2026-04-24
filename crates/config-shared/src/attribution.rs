use serde::{Deserialize, Serialize};

use crate::ids::HostId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionConfidence {
    Exact,
    Probable,
    Weak,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionSource {
    SshLog,
    ProcessTable,
    DeploymentMarker,
    FileSystemMetadata,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribution {
    pub author_name: Option<String>,
    pub author_source: AttributionSource,
    pub confidence: AttributionConfidence,
    pub process_hint: Option<String>,
    pub ssh_session_hint: Option<String>,
    pub deployment_hint: Option<String>,
}

impl Attribution {
    pub fn unknown() -> Self {
        Self {
            author_name: None,
            author_source: AttributionSource::Unknown,
            confidence: AttributionConfidence::Unknown,
            process_hint: None,
            ssh_session_hint: None,
            deployment_hint: None,
        }
    }

    pub fn with_host_id(host_id: &HostId) -> Self {
        Self {
            author_name: Some(format!("host:{}", host_id)),
            author_source: AttributionSource::FileSystemMetadata,
            confidence: AttributionConfidence::Weak,
            process_hint: None,
            ssh_session_hint: None,
            deployment_hint: None,
        }
    }
}
