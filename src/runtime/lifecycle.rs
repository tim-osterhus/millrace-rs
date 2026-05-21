//! Runtime-owned lifecycle intent helpers for effect handlers.

use serde_json::Value;

use crate::{
    contracts::StageResultEnvelope,
    workspace::{SourceLifecycleAction, SourceLifecycleIntent},
};

/// Build a source lifecycle intent for a completed stage result.
#[must_use]
pub fn source_lifecycle_intent_for_effect(
    stage_result: &StageResultEnvelope,
    lifecycle_plan_id: impl Into<String>,
    action: SourceLifecycleAction,
) -> SourceLifecycleIntent {
    SourceLifecycleIntent {
        lifecycle_plan_id: lifecycle_plan_id.into(),
        action,
        work_item_family_id: source_work_item_family_id(stage_result),
        work_item_kind: Some(stage_result.work_item_kind),
        work_item_id: stage_result.work_item_id.clone(),
        reason: Some(format!(
            "runtime effect for {}",
            stage_result.terminal_result.as_str()
        )),
    }
}

/// Resolve the source family id recorded for a stage result.
#[must_use]
pub fn source_work_item_family_id(stage_result: &StageResultEnvelope) -> Option<String> {
    stage_result
        .metadata
        .get("active_work_item_family_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| Some(stage_result.work_item_kind.as_str().to_owned()))
}
