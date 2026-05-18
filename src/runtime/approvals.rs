//! Durable execution capability approval storage.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Map;
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    contracts::{ExecutionCapabilityGrant, Plane, Timestamp, WorkDocumentError, WorkItemKind},
    workspace::{WorkspacePaths, atomic_write_text},
};

/// Result type for execution capability approval storage operations.
pub type ApprovalStorageResult<T> = Result<T, ApprovalStorageError>;

/// Failures produced while reading or writing approval records.
#[derive(Debug)]
pub enum ApprovalStorageError {
    /// Filesystem access failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying IO error message.
        message: String,
    },
    /// JSON decoding or encoding failed.
    Json {
        /// Path involved in the failure.
        path: PathBuf,
        /// Serde error message.
        message: String,
    },
    /// An approval record was invalid.
    Invalid {
        /// Human-readable failure reason.
        message: String,
    },
    /// The requested approval id was not found in pending storage.
    NotFound {
        /// Approval id that was requested.
        approval_id: String,
    },
    /// Timestamp parsing or arithmetic failed.
    Time {
        /// Field involved in the failure.
        field_name: &'static str,
        /// Human-readable failure reason.
        message: String,
    },
}

impl ApprovalStorageError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    fn json(path: impl Into<PathBuf>, error: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            message: error.to_string(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
}

impl fmt::Display for ApprovalStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(
                    f,
                    "approval storage filesystem error at {}: {message}",
                    path.display()
                )
            }
            Self::Json { path, message } => {
                write!(
                    f,
                    "approval storage JSON error at {}: {message}",
                    path.display()
                )
            }
            Self::Invalid { message } => f.write_str(message),
            Self::NotFound { approval_id } => write!(f, "approval not found: {approval_id}"),
            Self::Time {
                field_name,
                message,
            } => write!(f, "approval timestamp {field_name} failed: {message}"),
        }
    }
}

impl std::error::Error for ApprovalStorageError {}

/// Durable status for an execution capability approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCapabilityApprovalStatus {
    /// Awaiting operator decision.
    Pending,
    /// Operator approved the grant.
    Approved,
    /// Operator denied the grant.
    Denied,
    /// Approval expired before a decision.
    Expired,
    /// Approval was cancelled.
    Cancelled,
}

impl ExecutionCapabilityApprovalStatus {
    /// Stable snake-case status token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Durable approval record for one execution capability grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCapabilityApproval {
    pub schema_version: String,
    pub kind: String,
    pub approval_id: String,
    pub status: ExecutionCapabilityApprovalStatus,
    pub workspace_id: String,
    pub run_id: String,
    pub request_id: String,
    #[serde(default)]
    pub work_item_kind: Option<WorkItemKind>,
    #[serde(default)]
    pub work_item_id: Option<String>,
    pub plane: Plane,
    pub node_id: String,
    pub stage_kind_id: String,
    pub grant_id: String,
    pub capability_id: String,
    pub reason: String,
    pub requested_by: String,
    #[serde(default)]
    pub decided_by: Option<String>,
    pub created_at: Timestamp,
    #[serde(default)]
    pub decided_at: Option<Timestamp>,
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    #[serde(default)]
    pub decision_reason: Option<String>,
    pub grant: ExecutionCapabilityGrant,
    #[serde(default)]
    pub metadata: Map<String, serde_json::Value>,
}

impl ExecutionCapabilityApproval {
    /// Validates the approval record.
    pub fn validate(&self) -> ApprovalStorageResult<()> {
        require_value("schema_version", &self.schema_version)?;
        require_value("kind", &self.kind)?;
        if self.schema_version != "1.0" {
            return Err(ApprovalStorageError::invalid(
                "execution capability approval schema_version must be 1.0",
            ));
        }
        if self.kind != "execution_capability_approval" {
            return Err(ApprovalStorageError::invalid(
                "execution capability approval kind must be execution_capability_approval",
            ));
        }
        require_value("approval_id", &self.approval_id)?;
        require_value("workspace_id", &self.workspace_id)?;
        require_value("run_id", &self.run_id)?;
        require_value("request_id", &self.request_id)?;
        require_value("node_id", &self.node_id)?;
        require_value("stage_kind_id", &self.stage_kind_id)?;
        require_value("grant_id", &self.grant_id)?;
        require_value("capability_id", &self.capability_id)?;
        require_value("reason", &self.reason)?;
        require_value("requested_by", &self.requested_by)?;
        if matches!(
            self.status,
            ExecutionCapabilityApprovalStatus::Approved | ExecutionCapabilityApprovalStatus::Denied
        ) {
            if self
                .decided_by
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(ApprovalStorageError::invalid(
                    "resolved approvals require decided_by",
                ));
            }
            if self.decided_at.is_none() {
                return Err(ApprovalStorageError::invalid(
                    "resolved approvals require decided_at",
                ));
            }
            if self
                .decision_reason
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(ApprovalStorageError::invalid(
                    "resolved approvals require decision_reason",
                ));
            }
        }
        self.grant
            .validate()
            .map_err(|error| ApprovalStorageError::invalid(error.to_string()))?;
        if self.grant_id != self.grant.grant_id || self.capability_id != self.grant.capability_id {
            return Err(ApprovalStorageError::invalid(
                "approval grant identity must match embedded grant",
            ));
        }
        Ok(())
    }
}

/// Pending and resolved approval listing.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionCapabilityApprovalListing {
    pub pending: Vec<ExecutionCapabilityApproval>,
    pub resolved: Vec<ExecutionCapabilityApproval>,
}

/// Input needed to create a pending approval for one grant.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionCapabilityApprovalRequest<'a> {
    pub request_id: &'a str,
    pub run_id: &'a str,
    pub plane: Plane,
    pub node_id: &'a str,
    pub stage_kind_id: &'a str,
    pub work_item_kind: Option<WorkItemKind>,
    pub work_item_id: Option<&'a str>,
    pub grant: &'a ExecutionCapabilityGrant,
    pub now: &'a Timestamp,
    pub requested_by: &'a str,
}

/// Ensure a pending approval exists, or return an existing pending/resolved record for the grant.
pub fn ensure_execution_capability_approval(
    paths: &WorkspacePaths,
    request: ExecutionCapabilityApprovalRequest<'_>,
) -> ApprovalStorageResult<ExecutionCapabilityApproval> {
    if let Some(existing) = find_approval_for_grant(
        paths,
        request.run_id,
        request.request_id,
        &request.grant.grant_id,
    )? {
        return Ok(existing);
    }

    let approval = ExecutionCapabilityApproval {
        schema_version: "1.0".to_owned(),
        kind: "execution_capability_approval".to_owned(),
        approval_id: approval_id_for_grant(request.run_id, request.request_id, request.grant),
        status: ExecutionCapabilityApprovalStatus::Pending,
        workspace_id: workspace_id(paths),
        run_id: request.run_id.to_owned(),
        request_id: request.request_id.to_owned(),
        work_item_kind: request.work_item_kind,
        work_item_id: request.work_item_id.map(ToOwned::to_owned),
        plane: request.plane,
        node_id: request.node_id.to_owned(),
        stage_kind_id: request.stage_kind_id.to_owned(),
        grant_id: request.grant.grant_id.clone(),
        capability_id: request.grant.capability_id.clone(),
        reason: request.grant.decision_reason.clone(),
        requested_by: request.requested_by.to_owned(),
        decided_by: None,
        created_at: request.now.clone(),
        decided_at: None,
        expires_at: approval_expires_at(request.now, request.grant)?,
        decision_reason: None,
        grant: request.grant.clone(),
        metadata: Map::new(),
    };
    approval.validate()?;
    let path = pending_approval_path(paths, &approval.approval_id);
    write_approval(&path, &approval)?;
    Ok(approval)
}

/// Find an approval by the idempotency key `(run_id, request_id, grant_id)`.
pub fn find_approval_for_grant(
    paths: &WorkspacePaths,
    run_id: &str,
    request_id: &str,
    grant_id: &str,
) -> ApprovalStorageResult<Option<ExecutionCapabilityApproval>> {
    let listing = list_execution_capability_approvals(paths)?;
    Ok(listing
        .pending
        .into_iter()
        .chain(listing.resolved)
        .find(|approval| {
            approval.run_id == run_id
                && approval.request_id == request_id
                && approval.grant_id == grant_id
        }))
}

/// List durable approval records from pending and resolved storage.
pub fn list_execution_capability_approvals(
    paths: &WorkspacePaths,
) -> ApprovalStorageResult<ExecutionCapabilityApprovalListing> {
    let mut pending = list_approvals_in_dir(&paths.approvals_pending_dir)?;
    let mut resolved = list_approvals_in_dir(&paths.approvals_resolved_dir)?;
    pending.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
    resolved.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
    Ok(ExecutionCapabilityApprovalListing { pending, resolved })
}

/// Resolve a pending approval as approved.
pub fn approve_execution_capability_request(
    paths: &WorkspacePaths,
    approval_id: &str,
    decided_by: &str,
    reason: &str,
    now: &Timestamp,
) -> ApprovalStorageResult<ExecutionCapabilityApproval> {
    resolve_approval(
        paths,
        approval_id,
        ExecutionCapabilityApprovalStatus::Approved,
        decided_by,
        reason,
        now,
    )
}

/// Resolve a pending approval as denied.
pub fn deny_execution_capability_request(
    paths: &WorkspacePaths,
    approval_id: &str,
    decided_by: &str,
    reason: &str,
    now: &Timestamp,
) -> ApprovalStorageResult<ExecutionCapabilityApproval> {
    resolve_approval(
        paths,
        approval_id,
        ExecutionCapabilityApprovalStatus::Denied,
        decided_by,
        reason,
        now,
    )
}

fn resolve_approval(
    paths: &WorkspacePaths,
    approval_id: &str,
    status: ExecutionCapabilityApprovalStatus,
    decided_by: &str,
    reason: &str,
    now: &Timestamp,
) -> ApprovalStorageResult<ExecutionCapabilityApproval> {
    require_value("approval_id", approval_id)?;
    require_value("decided_by", decided_by)?;
    require_value("reason", reason)?;
    let pending_path = pending_approval_path(paths, approval_id);
    if !pending_path.is_file() {
        return Err(ApprovalStorageError::NotFound {
            approval_id: approval_id.to_owned(),
        });
    }
    let mut approval = read_approval(&pending_path)?;
    approval.status = status;
    approval.decided_by = Some(decided_by.trim().to_owned());
    approval.decided_at = Some(now.clone());
    approval.decision_reason = Some(reason.trim().to_owned());
    approval.validate()?;
    let resolved_path = resolved_approval_path(paths, approval_id);
    write_approval(&resolved_path, &approval)?;
    fs::remove_file(&pending_path)
        .map_err(|error| ApprovalStorageError::io(&pending_path, error))?;
    Ok(approval)
}

fn list_approvals_in_dir(dir: &Path) -> ApprovalStorageResult<Vec<ExecutionCapabilityApproval>> {
    fs::create_dir_all(dir).map_err(|error| ApprovalStorageError::io(dir, error))?;
    let mut approvals = Vec::new();
    for entry in fs::read_dir(dir).map_err(|error| ApprovalStorageError::io(dir, error))? {
        let entry = entry.map_err(|error| ApprovalStorageError::io(dir, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        approvals.push(read_approval(&path)?);
    }
    Ok(approvals)
}

fn read_approval(path: &Path) -> ApprovalStorageResult<ExecutionCapabilityApproval> {
    let raw = fs::read_to_string(path).map_err(|error| ApprovalStorageError::io(path, error))?;
    let approval: ExecutionCapabilityApproval =
        serde_json::from_str(&raw).map_err(|error| ApprovalStorageError::json(path, error))?;
    approval.validate()?;
    Ok(approval)
}

fn write_approval(
    path: &Path,
    approval: &ExecutionCapabilityApproval,
) -> ApprovalStorageResult<()> {
    approval.validate()?;
    let rendered = serde_json::to_string_pretty(approval)
        .map_err(|error| ApprovalStorageError::json(path, error))?
        + "\n";
    atomic_write_text(path, &rendered).map_err(|error| ApprovalStorageError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn pending_approval_path(paths: &WorkspacePaths, approval_id: &str) -> PathBuf {
    paths
        .approvals_pending_dir
        .join(format!("{approval_id}.json"))
}

fn resolved_approval_path(paths: &WorkspacePaths, approval_id: &str) -> PathBuf {
    paths
        .approvals_resolved_dir
        .join(format!("{approval_id}.json"))
}

fn approval_id_for_grant(
    run_id: &str,
    request_id: &str,
    grant: &ExecutionCapabilityGrant,
) -> String {
    let mut digest = Sha256::new();
    digest.update(run_id.as_bytes());
    digest.update(b"\0");
    digest.update(request_id.as_bytes());
    digest.update(b"\0");
    digest.update(grant.grant_id.as_bytes());
    let digest = format!("{:x}", digest.finalize());
    format!(
        "approval-{}-{}",
        safe_id_part(&grant.grant_id),
        &digest[..8]
    )
}

fn safe_id_part(value: &str) -> String {
    let mut rendered = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            rendered.push(ch);
        } else if !rendered.ends_with('-') {
            rendered.push('-');
        }
    }
    if rendered
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
    {
        rendered
    } else {
        format!("grant-{rendered}")
    }
}

fn approval_expires_at(
    now: &Timestamp,
    grant: &ExecutionCapabilityGrant,
) -> ApprovalStorageResult<Option<Timestamp>> {
    let Some(policy) = &grant.approval_policy_ref else {
        return Ok(None);
    };
    let Some(expiration_seconds) = policy.expiration_seconds else {
        return Ok(None);
    };
    let parsed = OffsetDateTime::parse(now.as_str(), &Rfc3339).map_err(|error| {
        ApprovalStorageError::Time {
            field_name: "created_at",
            message: error.to_string(),
        }
    })?;
    let expiration_seconds =
        i64::try_from(expiration_seconds).map_err(|error| ApprovalStorageError::Time {
            field_name: "expires_at",
            message: error.to_string(),
        })?;
    let expires = parsed
        .checked_add(Duration::seconds(expiration_seconds))
        .ok_or_else(|| ApprovalStorageError::Time {
            field_name: "expires_at",
            message: "timestamp overflow".to_owned(),
        })?;
    let rendered = expires
        .format(&Rfc3339)
        .map_err(|error| ApprovalStorageError::Time {
            field_name: "expires_at",
            message: error.to_string(),
        })?;
    Timestamp::parse("expires_at", &rendered)
        .map(Some)
        .map_err(timestamp_error("expires_at"))
}

fn timestamp_error(
    field_name: &'static str,
) -> impl FnOnce(WorkDocumentError) -> ApprovalStorageError {
    move |error| ApprovalStorageError::Time {
        field_name,
        message: error.to_string(),
    }
}

fn workspace_id(paths: &WorkspacePaths) -> String {
    paths
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("workspace")
        .to_owned()
}

fn require_value(field_name: &'static str, value: &str) -> ApprovalStorageResult<()> {
    if value.trim().is_empty() {
        Err(ApprovalStorageError::invalid(format!(
            "{field_name} is required"
        )))
    } else {
        Ok(())
    }
}
