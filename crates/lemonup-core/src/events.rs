use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Initialize,
    Scan,
    CheckUpdates,
    Install,
    Update,
    Remove,
    Backup,
    Import,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStage {
    Queued,
    Started,
    Discovering,
    Downloading,
    Extracting,
    Copying,
    Scanning,
    Persisting,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationProgress {
    pub operation_id: String,
    pub kind: OperationKind,
    pub stage: OperationStage,
    pub target: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub at: OffsetDateTime,
}

impl OperationProgress {
    pub fn new(
        operation_id: impl Into<String>,
        kind: OperationKind,
        stage: OperationStage,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            kind,
            stage,
            target: None,
            current: None,
            total: None,
            message: None,
            error: None,
            at: OffsetDateTime::now_utc(),
        }
    }
}
