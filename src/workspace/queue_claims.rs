//! Shared queue claim value objects.

use std::path::PathBuf;

use crate::contracts::{Plane, WorkItemKind};

/// Claimed queue ownership for one work item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueClaim {
    /// Backward-compatible built-in work item kind.
    pub work_item_kind: WorkItemKind,
    /// Work item family id used by compiled workflow primitives.
    pub family_id: String,
    /// Runtime plane that owns the claimed item.
    pub plane: Plane,
    /// Canonical work item id.
    pub work_item_id: String,
    /// Active path now owning the work item.
    pub path: PathBuf,
    /// Lifecycle state the artifact was claimed from.
    pub source_state: Option<String>,
    /// Source path before the claim rename.
    pub source_path: Option<PathBuf>,
    /// Compiled queue claim policy that selected this item.
    pub claim_policy_id: Option<String>,
    /// Family order within the compiled claim policy.
    pub claim_order: Option<u64>,
}

impl QueueClaim {
    /// Build a claim for a built-in work-item kind.
    #[must_use]
    pub fn for_builtin(
        work_item_kind: WorkItemKind,
        work_item_id: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            family_id: work_item_kind.as_str().to_owned(),
            plane: plane_for_builtin_kind(work_item_kind),
            work_item_kind,
            work_item_id: work_item_id.into(),
            path: path.into(),
            source_state: None,
            source_path: None,
            claim_policy_id: None,
            claim_order: None,
        }
    }

    /// Attach source lifecycle evidence to the claim.
    #[must_use]
    pub fn with_source(mut self, source_state: impl Into<String>, source_path: PathBuf) -> Self {
        self.source_state = Some(source_state.into());
        self.source_path = Some(source_path);
        self
    }

    /// Attach compiled claim-policy evidence to the claim.
    #[must_use]
    pub fn with_claim_policy(mut self, policy_id: impl Into<String>, claim_order: u64) -> Self {
        self.claim_policy_id = Some(policy_id.into());
        self.claim_order = Some(claim_order);
        self
    }
}

const fn plane_for_builtin_kind(kind: WorkItemKind) -> Plane {
    match kind {
        WorkItemKind::Task => Plane::Execution,
        WorkItemKind::Probe
        | WorkItemKind::Spec
        | WorkItemKind::Incident
        | WorkItemKind::BlueprintDraft => Plane::Planning,
        WorkItemKind::LearningRequest => Plane::Learning,
    }
}
