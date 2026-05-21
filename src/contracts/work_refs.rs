//! Family-id based work item reference helpers.

use super::{ContractError, Plane, WorkItemKind, validate_safe_identifier};

/// Normalizes a work-item family id with the shared safe identifier contract.
pub fn normalize_work_item_family_id(
    value: &str,
    field_name: &str,
) -> Result<String, ContractError> {
    Ok(validate_safe_identifier(value, field_name)?.to_owned())
}

/// Returns the family id that preserves the legacy work-item kind value.
#[must_use]
pub fn family_id_for_work_item_kind(kind: WorkItemKind) -> &'static str {
    kind.as_str()
}

/// Returns the legacy work-item kind represented by a built-in family id.
pub fn legacy_work_item_kind_for_family_id(
    family_id: &str,
) -> Result<Option<WorkItemKind>, ContractError> {
    let normalized = normalize_work_item_family_id(family_id, "work_item_family_id")?;
    Ok(match normalized.as_str() {
        "task" => Some(WorkItemKind::Task),
        "probe" => Some(WorkItemKind::Probe),
        "spec" => Some(WorkItemKind::Spec),
        "incident" => Some(WorkItemKind::Incident),
        "learning_request" => Some(WorkItemKind::LearningRequest),
        "blueprint_draft" => Some(WorkItemKind::BlueprintDraft),
        _ => None,
    })
}

/// Returns the owning plane for built-in work-item family ids.
pub fn plane_for_work_item_family_id(family_id: &str) -> Result<Option<Plane>, ContractError> {
    let normalized = normalize_work_item_family_id(family_id, "work_item_family_id")?;
    Ok(match normalized.as_str() {
        "task" => Some(Plane::Execution),
        "probe" | "spec" | "incident" | "blueprint_draft" => Some(Plane::Planning),
        "learning_request" => Some(Plane::Learning),
        _ => None,
    })
}

/// Coerces optional family/kind pairs while preserving unknown family ids as data.
pub fn coerce_family_and_kind(
    family_id: Option<&str>,
    work_item_kind: Option<WorkItemKind>,
) -> Result<(Option<String>, Option<WorkItemKind>), ContractError> {
    let normalized_family = family_id
        .map(|value| normalize_work_item_family_id(value, "work_item_family_id"))
        .transpose()?;
    let resolved_kind = match (normalized_family.as_deref(), work_item_kind) {
        (Some(family), None) => legacy_work_item_kind_for_family_id(family)?,
        (_, kind) => kind,
    };
    let resolved_family = match (normalized_family, resolved_kind) {
        (Some(family), Some(kind)) if family != kind.as_str() => {
            return Err(ContractError::UnsafeIdentifier {
                field_name: "work_item_family_id".to_owned(),
                value: family,
                reason: super::IdentifierErrorReason::InvalidCharacters,
            });
        }
        (Some(family), kind) => (Some(family), kind),
        (None, Some(kind)) => (
            Some(family_id_for_work_item_kind(kind).to_owned()),
            Some(kind),
        ),
        (None, None) => (None, None),
    };
    Ok(resolved_family)
}
