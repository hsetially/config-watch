use serde::{Deserialize, Serialize};

use crate::ids::HostId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryKind {
    Stat,
    Preview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadataQuery {
    pub host_id: HostId,
    pub path: String,
    pub query_kind: QueryKind,
    pub requester_id: String,
}