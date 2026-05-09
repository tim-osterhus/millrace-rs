//! Headed markdown work-document parsing and rendering helpers.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::contracts::{
    IncidentDecision, IncidentDocument, IncidentSeverity, LearningRequestAction,
    LearningRequestDocument, LearningStageName, Plane, ProbeDocument, ProbeStatusHint,
    RootIntakeKind, SpecDocument, SpecSourceType, StageName, TaskDocument, TaskStatusHint,
    Timestamp, WORK_DOCUMENT_SCHEMA_VERSION, WorkDocument, WorkDocumentError, WorkItemKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Scalar,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentModel {
    Task,
    Probe,
    Spec,
    Incident,
    LearningRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FieldSpec {
    field_name: &'static str,
    kind: FieldKind,
}

#[derive(Debug, Default)]
struct ParsedFields {
    scalars: BTreeMap<&'static str, String>,
    lists: BTreeMap<&'static str, Vec<String>>,
}

/// Parse one human-facing markdown work document.
pub fn parse_work_document(raw: &str) -> Result<WorkDocument, WorkDocumentError> {
    parse_work_document_with_source(raw, "<memory>")
}

/// Parse one human-facing markdown work document with source context in errors.
pub fn parse_work_document_with_source(
    raw: &str,
    source_name: &str,
) -> Result<WorkDocument, WorkDocumentError> {
    let (heading_title, fields) = parse_markdown_fields(raw, source_name)?;
    match infer_model(&fields, source_name)? {
        DocumentModel::Task => parse_task_from_fields(&heading_title, &fields).map(Into::into),
        DocumentModel::Probe => parse_probe_from_fields(&heading_title, &fields).map(Into::into),
        DocumentModel::Spec => parse_spec_from_fields(&heading_title, &fields).map(Into::into),
        DocumentModel::Incident => {
            parse_incident_from_fields(&heading_title, &fields).map(Into::into)
        }
        DocumentModel::LearningRequest => {
            parse_learning_request_from_fields(&heading_title, &fields).map(Into::into)
        }
    }
}

/// Parse a markdown task document and reject other document kinds.
pub fn parse_task_document(raw: &str) -> Result<TaskDocument, WorkDocumentError> {
    parse_task_document_with_source(raw, "<memory>")
}

/// Parse a markdown task document with source context in errors.
pub fn parse_task_document_with_source(
    raw: &str,
    source_name: &str,
) -> Result<TaskDocument, WorkDocumentError> {
    let (heading_title, fields) = parse_markdown_fields(raw, source_name)?;
    ensure_model(&fields, DocumentModel::Task, source_name)?;
    parse_task_from_fields(&heading_title, &fields)
}

/// Parse a markdown probe document and reject other document kinds.
pub fn parse_probe_document(raw: &str) -> Result<ProbeDocument, WorkDocumentError> {
    parse_probe_document_with_source(raw, "<memory>")
}

/// Parse a markdown probe document with source context in errors.
pub fn parse_probe_document_with_source(
    raw: &str,
    source_name: &str,
) -> Result<ProbeDocument, WorkDocumentError> {
    let (heading_title, fields) = parse_markdown_fields(raw, source_name)?;
    ensure_model(&fields, DocumentModel::Probe, source_name)?;
    parse_probe_from_fields(&heading_title, &fields)
}

/// Parse a markdown spec document and reject other document kinds.
pub fn parse_spec_document(raw: &str) -> Result<SpecDocument, WorkDocumentError> {
    parse_spec_document_with_source(raw, "<memory>")
}

/// Parse a markdown spec document with source context in errors.
pub fn parse_spec_document_with_source(
    raw: &str,
    source_name: &str,
) -> Result<SpecDocument, WorkDocumentError> {
    let (heading_title, fields) = parse_markdown_fields(raw, source_name)?;
    ensure_model(&fields, DocumentModel::Spec, source_name)?;
    parse_spec_from_fields(&heading_title, &fields)
}

/// Parse a markdown incident document and reject other document kinds.
pub fn parse_incident_document(raw: &str) -> Result<IncidentDocument, WorkDocumentError> {
    parse_incident_document_with_source(raw, "<memory>")
}

/// Parse a markdown incident document with source context in errors.
pub fn parse_incident_document_with_source(
    raw: &str,
    source_name: &str,
) -> Result<IncidentDocument, WorkDocumentError> {
    let (heading_title, fields) = parse_markdown_fields(raw, source_name)?;
    ensure_model(&fields, DocumentModel::Incident, source_name)?;
    parse_incident_from_fields(&heading_title, &fields)
}

/// Parse a markdown learning-request document and reject other document kinds.
pub fn parse_learning_request_document(
    raw: &str,
) -> Result<LearningRequestDocument, WorkDocumentError> {
    parse_learning_request_document_with_source(raw, "<memory>")
}

/// Parse a markdown learning-request document with source context in errors.
pub fn parse_learning_request_document_with_source(
    raw: &str,
    source_name: &str,
) -> Result<LearningRequestDocument, WorkDocumentError> {
    let (heading_title, fields) = parse_markdown_fields(raw, source_name)?;
    ensure_model(&fields, DocumentModel::LearningRequest, source_name)?;
    parse_learning_request_from_fields(&heading_title, &fields)
}

/// Read and parse a markdown work document.
pub fn read_work_document(path: impl AsRef<Path>) -> Result<WorkDocument, WorkDocumentError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|error| WorkDocumentError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    parse_work_document_with_source(&raw, source_name_for_path(path).as_ref())
}

/// Parse a task import payload from headed markdown or JSON.
pub fn read_task_import_document(
    path: impl AsRef<Path>,
) -> Result<TaskDocument, WorkDocumentError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|error| WorkDocumentError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let source_name = source_name_for_path(path);
    match path.extension().and_then(|value| value.to_str()) {
        Some("md") => parse_task_document_with_source(&raw, &source_name),
        Some("json") => parse_task_json_import_with_source(&raw, &source_name),
        _ => Err(WorkDocumentError::InvalidDocument {
            message: "task import path must end with .md or .json".to_owned(),
        }),
    }
}

/// Parse a probe import payload from headed markdown or JSON.
pub fn read_probe_import_document(
    path: impl AsRef<Path>,
) -> Result<ProbeDocument, WorkDocumentError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|error| WorkDocumentError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let source_name = source_name_for_path(path);
    match path.extension().and_then(|value| value.to_str()) {
        Some("md") => parse_probe_document_with_source(&raw, &source_name),
        Some("json") => parse_probe_json_import_with_source(&raw, &source_name),
        _ => Err(WorkDocumentError::InvalidDocument {
            message: "probe import path must end with .md or .json".to_owned(),
        }),
    }
}

/// Parse a spec import payload from headed markdown or JSON.
pub fn read_spec_import_document(
    path: impl AsRef<Path>,
) -> Result<SpecDocument, WorkDocumentError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|error| WorkDocumentError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let source_name = source_name_for_path(path);
    match path.extension().and_then(|value| value.to_str()) {
        Some("md") => parse_spec_document_with_source(&raw, &source_name),
        Some("json") => parse_spec_json_import_with_source(&raw, &source_name),
        _ => Err(WorkDocumentError::InvalidDocument {
            message: "spec import path must end with .md or .json".to_owned(),
        }),
    }
}

/// Parse an explicit task JSON import payload.
pub fn parse_task_json_import(raw: &str) -> Result<TaskDocument, WorkDocumentError> {
    parse_task_json_import_with_source(raw, "<memory>")
}

/// Parse an explicit task JSON import payload with source context in errors.
pub fn parse_task_json_import_with_source(
    raw: &str,
    source_name: &str,
) -> Result<TaskDocument, WorkDocumentError> {
    let document: TaskDocument = parse_json_import(raw, source_name, WorkItemKind::Task.as_str())?;
    document.validate()?;
    Ok(document)
}

/// Parse an explicit probe JSON import payload.
pub fn parse_probe_json_import(raw: &str) -> Result<ProbeDocument, WorkDocumentError> {
    parse_probe_json_import_with_source(raw, "<memory>")
}

/// Parse an explicit probe JSON import payload with source context in errors.
pub fn parse_probe_json_import_with_source(
    raw: &str,
    source_name: &str,
) -> Result<ProbeDocument, WorkDocumentError> {
    let document: ProbeDocument =
        parse_json_import(raw, source_name, WorkItemKind::Probe.as_str())?;
    document.validate()?;
    Ok(document)
}

/// Parse an explicit spec JSON import payload.
pub fn parse_spec_json_import(raw: &str) -> Result<SpecDocument, WorkDocumentError> {
    parse_spec_json_import_with_source(raw, "<memory>")
}

/// Parse an explicit spec JSON import payload with source context in errors.
pub fn parse_spec_json_import_with_source(
    raw: &str,
    source_name: &str,
) -> Result<SpecDocument, WorkDocumentError> {
    let document: SpecDocument = parse_json_import(raw, source_name, WorkItemKind::Spec.as_str())?;
    document.validate()?;
    Ok(document)
}

/// Render a canonical operator-facing markdown work document.
#[must_use]
pub fn render_work_document(document: &WorkDocument) -> String {
    match document {
        WorkDocument::Task(document) => render_task_document(document),
        WorkDocument::Probe(document) => render_probe_document(document),
        WorkDocument::Spec(document) => render_spec_document(document),
        WorkDocument::Incident(document) => render_incident_document(document),
        WorkDocument::LearningRequest(document) => render_learning_request_document(document),
    }
}

/// Render a canonical task markdown document.
#[must_use]
pub fn render_task_document(document: &TaskDocument) -> String {
    let mut lines = heading_lines(&document.title);

    push_scalar(&mut lines, "Task-ID", &document.task_id);
    push_scalar(&mut lines, "Title", &document.title);
    push_optional_non_empty_scalar(&mut lines, "Summary", &document.summary);
    push_optional_scalar(&mut lines, "Root-Idea-ID", document.root_idea_id.as_deref());
    push_optional_scalar(&mut lines, "Root-Spec-ID", document.root_spec_id.as_deref());
    if let Some(root_intake_kind) = document.root_intake_kind {
        push_scalar(&mut lines, "Root-Intake-Kind", root_intake_kind.as_str());
    }
    push_optional_scalar(
        &mut lines,
        "Root-Intake-ID",
        document.root_intake_id.as_deref(),
    );
    push_optional_scalar(&mut lines, "Spec-ID", document.spec_id.as_deref());
    push_optional_scalar(
        &mut lines,
        "Parent-Task-ID",
        document.parent_task_id.as_deref(),
    );
    push_optional_scalar(&mut lines, "Incident-ID", document.incident_id.as_deref());
    if let Some(status_hint) = document.status_hint {
        push_scalar(&mut lines, "Status-Hint", status_hint.as_str());
    }
    push_scalar(&mut lines, "Created-At", document.created_at.as_str());
    push_scalar(&mut lines, "Created-By", &document.created_by);
    if let Some(updated_at) = &document.updated_at {
        push_scalar(&mut lines, "Updated-At", updated_at.as_str());
    }

    push_list(&mut lines, "Depends-On", &document.depends_on);
    push_list(&mut lines, "Blocks", &document.blocks);
    push_list(&mut lines, "Tags", &document.tags);
    push_list(&mut lines, "Target-Paths", &document.target_paths);
    push_list(&mut lines, "Acceptance", &document.acceptance);
    push_list(&mut lines, "Required-Checks", &document.required_checks);
    push_list(&mut lines, "References", &document.references);
    push_list(&mut lines, "Risk", &document.risk);

    finish_lines(lines)
}

/// Render a canonical probe markdown document.
#[must_use]
pub fn render_probe_document(document: &ProbeDocument) -> String {
    let mut lines = heading_lines(&document.title);

    push_scalar(&mut lines, "Probe-ID", &document.probe_id);
    push_scalar(&mut lines, "Title", &document.title);
    push_optional_non_empty_scalar(&mut lines, "Summary", &document.summary);
    push_scalar(&mut lines, "Request", &document.request);
    if let Some(status_hint) = document.status_hint {
        push_scalar(&mut lines, "Status-Hint", status_hint.as_str());
    }
    push_scalar(&mut lines, "Created-At", document.created_at.as_str());
    push_scalar(&mut lines, "Created-By", &document.created_by);
    if let Some(updated_at) = &document.updated_at {
        push_scalar(&mut lines, "Updated-At", updated_at.as_str());
    }

    push_list(&mut lines, "Target-Paths", &document.target_paths);
    push_list(&mut lines, "Constraints", &document.constraints);
    push_list(&mut lines, "Acceptance", &document.acceptance);
    push_list(&mut lines, "Risk-Notes", &document.risk_notes);
    push_list(&mut lines, "References", &document.references);
    push_list(&mut lines, "Tags", &document.tags);

    finish_lines(lines)
}

/// Render a canonical spec markdown document.
#[must_use]
pub fn render_spec_document(document: &SpecDocument) -> String {
    let mut lines = heading_lines(&document.title);

    push_scalar(&mut lines, "Spec-ID", &document.spec_id);
    push_scalar(&mut lines, "Title", &document.title);
    push_scalar(&mut lines, "Summary", &document.summary);
    push_scalar(&mut lines, "Source-Type", document.source_type.as_str());
    push_optional_scalar(&mut lines, "Source-ID", document.source_id.as_deref());
    push_optional_scalar(
        &mut lines,
        "Parent-Spec-ID",
        document.parent_spec_id.as_deref(),
    );
    push_optional_scalar(&mut lines, "Root-Idea-ID", document.root_idea_id.as_deref());
    push_optional_scalar(&mut lines, "Root-Spec-ID", document.root_spec_id.as_deref());
    if let Some(root_intake_kind) = document.root_intake_kind {
        push_scalar(&mut lines, "Root-Intake-Kind", root_intake_kind.as_str());
    }
    push_optional_scalar(
        &mut lines,
        "Root-Intake-ID",
        document.root_intake_id.as_deref(),
    );
    push_scalar(&mut lines, "Created-At", document.created_at.as_str());
    push_scalar(&mut lines, "Created-By", &document.created_by);
    if let Some(updated_at) = &document.updated_at {
        push_scalar(&mut lines, "Updated-At", updated_at.as_str());
    }

    push_list(&mut lines, "Goals", &document.goals);
    push_list(&mut lines, "Non-Goals", &document.non_goals);
    push_list(&mut lines, "Scope", &document.scope);
    push_list(&mut lines, "Constraints", &document.constraints);
    push_list(&mut lines, "Assumptions", &document.assumptions);
    push_list(&mut lines, "Risks", &document.risks);
    push_list(&mut lines, "Target-Paths", &document.target_paths);
    push_list(&mut lines, "Entrypoints", &document.entrypoints);
    push_list(&mut lines, "Required-Skills", &document.required_skills);
    push_list(
        &mut lines,
        "Decomposition-Hints",
        &document.decomposition_hints,
    );
    push_list(&mut lines, "Acceptance", &document.acceptance);
    push_list(&mut lines, "References", &document.references);

    finish_lines(lines)
}

/// Render a canonical incident markdown document.
#[must_use]
pub fn render_incident_document(document: &IncidentDocument) -> String {
    let mut lines = heading_lines(&document.title);

    push_scalar(&mut lines, "Incident-ID", &document.incident_id);
    push_scalar(&mut lines, "Title", &document.title);
    push_scalar(&mut lines, "Summary", &document.summary);
    push_optional_scalar(&mut lines, "Root-Idea-ID", document.root_idea_id.as_deref());
    push_optional_scalar(&mut lines, "Root-Spec-ID", document.root_spec_id.as_deref());
    if let Some(root_intake_kind) = document.root_intake_kind {
        push_scalar(&mut lines, "Root-Intake-Kind", root_intake_kind.as_str());
    }
    push_optional_scalar(
        &mut lines,
        "Root-Intake-ID",
        document.root_intake_id.as_deref(),
    );
    push_optional_scalar(
        &mut lines,
        "Source-Task-ID",
        document.source_task_id.as_deref(),
    );
    push_optional_scalar(
        &mut lines,
        "Source-Spec-ID",
        document.source_spec_id.as_deref(),
    );
    push_scalar(&mut lines, "Source-Stage", document.source_stage.as_str());
    push_scalar(&mut lines, "Source-Plane", document.source_plane.as_str());
    push_scalar(&mut lines, "Failure-Class", &document.failure_class);
    if document.severity != IncidentSeverity::Medium {
        push_scalar(&mut lines, "Severity", document.severity.as_str());
    }
    if !document.needs_planning {
        push_scalar(&mut lines, "Needs-Planning", "false");
    }
    push_scalar(&mut lines, "Trigger-Reason", &document.trigger_reason);
    push_scalar(
        &mut lines,
        "Consultant-Decision",
        document.consultant_decision.as_str(),
    );
    push_scalar(&mut lines, "Opened-At", document.opened_at.as_str());
    push_scalar(&mut lines, "Opened-By", &document.opened_by);
    if let Some(updated_at) = &document.updated_at {
        push_scalar(&mut lines, "Updated-At", updated_at.as_str());
    }

    push_list(&mut lines, "Observed-Symptoms", &document.observed_symptoms);
    push_list(&mut lines, "Failed-Attempts", &document.failed_attempts);
    push_list(&mut lines, "Evidence-Paths", &document.evidence_paths);
    push_list(&mut lines, "Related-Run-IDs", &document.related_run_ids);
    push_list(
        &mut lines,
        "Related-Stage-Results",
        &document.related_stage_results,
    );
    push_list(&mut lines, "References", &document.references);

    finish_lines(lines)
}

/// Render a canonical learning-request markdown document.
#[must_use]
pub fn render_learning_request_document(document: &LearningRequestDocument) -> String {
    let mut lines = heading_lines(&document.title);

    push_scalar(
        &mut lines,
        "Learning-Request-ID",
        &document.learning_request_id,
    );
    push_scalar(&mut lines, "Title", &document.title);
    push_optional_non_empty_scalar(&mut lines, "Summary", &document.summary);
    push_scalar(
        &mut lines,
        "Requested-Action",
        document.requested_action.as_str(),
    );
    push_optional_scalar(
        &mut lines,
        "Target-Skill-ID",
        document.target_skill_id.as_deref(),
    );
    if let Some(target_stage) = document.target_stage {
        push_scalar(&mut lines, "Target-Stage", target_stage.as_str());
    }
    if !document
        .trigger_metadata
        .as_object()
        .is_some_and(Map::is_empty)
    {
        push_scalar(
            &mut lines,
            "Trigger-Metadata",
            &render_json_scalar(&document.trigger_metadata),
        );
    }
    push_scalar(&mut lines, "Created-At", document.created_at.as_str());
    push_scalar(&mut lines, "Created-By", &document.created_by);
    if let Some(updated_at) = &document.updated_at {
        push_scalar(&mut lines, "Updated-At", updated_at.as_str());
    }

    push_list(&mut lines, "Source-Refs", &document.source_refs);
    push_list(
        &mut lines,
        "Preferred-Output-Paths",
        &document.preferred_output_paths,
    );
    push_list(
        &mut lines,
        "Originating-Run-IDs",
        &document.originating_run_ids,
    );
    push_list(&mut lines, "Artifact-Paths", &document.artifact_paths);
    push_list(&mut lines, "References", &document.references);

    finish_lines(lines)
}

fn parse_markdown_fields(
    raw: &str,
    source_name: &str,
) -> Result<(String, ParsedFields), WorkDocumentError> {
    let lines: Vec<&str> = raw.lines().collect();
    if lines.is_empty() {
        return Err(WorkDocumentError::InvalidDocument {
            message: format!("work document {source_name} is empty"),
        });
    }

    if lines[0].trim() == "---" {
        return Err(WorkDocumentError::InvalidDocument {
            message: format!("work document {source_name} must not use JSON frontmatter"),
        });
    }

    let heading_title =
        parse_h1_title(lines[0]).ok_or_else(|| WorkDocumentError::InvalidDocument {
            message: format!("work document {source_name} must start with a markdown H1 title"),
        })?;

    if heading_title.is_empty() {
        return Err(WorkDocumentError::InvalidDocument {
            message: format!("work document {source_name} has an empty H1 title"),
        });
    }

    let mut fields = ParsedFields::default();
    let mut index = 1;
    while index < lines.len() {
        let stripped = lines[index].trim();
        index += 1;
        if stripped.is_empty() {
            continue;
        }

        let Some((label, inline_value)) = parse_field_line(stripped) else {
            continue;
        };
        let Some(field_spec) = field_spec_by_label(label) else {
            index = skip_unknown_field_block(&lines, index);
            continue;
        };

        if fields.scalars.contains_key(field_spec.field_name)
            || fields.lists.contains_key(field_spec.field_name)
        {
            return Err(WorkDocumentError::InvalidDocument {
                message: format!("work document {source_name} repeats field `{label}`"),
            });
        }

        if !inline_value.is_empty() {
            if field_spec.kind == FieldKind::List {
                fields.lists.insert(
                    field_spec.field_name,
                    normalize_list_items(field_spec.field_name, vec![inline_value.to_owned()]),
                );
            } else {
                fields
                    .scalars
                    .insert(field_spec.field_name, inline_value.to_owned());
            }
            continue;
        }

        if field_spec.kind == FieldKind::Scalar {
            index = skip_blank_scalar_block(&lines, index, label, source_name)?;
            continue;
        }

        let mut items = Vec::new();
        while index < lines.len() {
            let next_line = lines[index].trim();
            if next_line.is_empty() {
                index += 1;
                if !items.is_empty() {
                    break;
                }
                continue;
            }
            if parse_field_line(next_line).is_some() {
                break;
            }
            let Some(item) = parse_list_item(next_line) else {
                return Err(WorkDocumentError::InvalidDocument {
                    message: format!(
                        "work document {source_name} has invalid list item under `{label}`"
                    ),
                });
            };
            items.push(item.to_owned());
            index += 1;
        }
        fields.lists.insert(
            field_spec.field_name,
            normalize_list_items(field_spec.field_name, items),
        );
    }

    Ok((heading_title, fields))
}

fn parse_h1_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('#')?;
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_whitespace() {
        return None;
    }
    let title = rest[first.len_utf8()..].trim();
    Some(title.to_owned())
}

fn parse_field_line(line: &str) -> Option<(&str, &str)> {
    let (label, value) = line.split_once(':')?;
    if is_field_label(label) {
        Some((label, value.trim()))
    } else {
        None
    }
}

fn is_field_label(label: &str) -> bool {
    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn parse_list_item(line: &str) -> Option<&str> {
    let value = line.strip_prefix("- ")?.trim();
    if value.is_empty() { None } else { Some(value) }
}

fn skip_blank_scalar_block(
    lines: &[&str],
    mut index: usize,
    label: &str,
    source_name: &str,
) -> Result<usize, WorkDocumentError> {
    while index < lines.len() {
        let candidate = lines[index].trim();
        if candidate.is_empty() {
            index += 1;
            continue;
        }
        if parse_field_line(candidate).is_some() {
            break;
        }
        if parse_list_item(candidate).is_some() {
            return Err(WorkDocumentError::InvalidDocument {
                message: format!(
                    "work document {source_name} has list item under scalar field `{label}`"
                ),
            });
        }
        break;
    }
    Ok(index)
}

fn skip_unknown_field_block(lines: &[&str], mut index: usize) -> usize {
    while index < lines.len() {
        let candidate = lines[index].trim();
        if candidate.is_empty() {
            index += 1;
            continue;
        }
        if parse_field_line(candidate).is_some() {
            break;
        }
        index += 1;
    }
    index
}

fn infer_model(
    fields: &ParsedFields,
    source_name: &str,
) -> Result<DocumentModel, WorkDocumentError> {
    if fields.scalars.contains_key("task_id") {
        Ok(DocumentModel::Task)
    } else if fields.scalars.contains_key("probe_id") {
        Ok(DocumentModel::Probe)
    } else if fields.scalars.contains_key("learning_request_id") {
        Ok(DocumentModel::LearningRequest)
    } else if fields.scalars.contains_key("incident_id") {
        Ok(DocumentModel::Incident)
    } else if fields.scalars.contains_key("spec_id") {
        Ok(DocumentModel::Spec)
    } else {
        Err(WorkDocumentError::InvalidDocument {
            message: format!(
                "work document {source_name} must include one canonical document identifier"
            ),
        })
    }
}

fn ensure_model(
    fields: &ParsedFields,
    expected: DocumentModel,
    source_name: &str,
) -> Result<(), WorkDocumentError> {
    let actual = infer_model(fields, source_name)?;
    if actual == expected {
        Ok(())
    } else {
        Err(WorkDocumentError::InvalidDocument {
            message: format!(
                "work document {source_name} has kind {}, expected {}",
                model_name(actual),
                model_name(expected)
            ),
        })
    }
}

fn parse_task_from_fields(
    heading_title: &str,
    fields: &ParsedFields,
) -> Result<TaskDocument, WorkDocumentError> {
    let document = TaskDocument {
        task_id: required_scalar(fields, "task_id")?,
        title: required_title(fields, heading_title)?,
        summary: optional_scalar(fields, "summary").unwrap_or_default(),
        root_idea_id: optional_scalar(fields, "root_idea_id"),
        root_spec_id: optional_scalar(fields, "root_spec_id"),
        root_intake_kind: optional_enum(fields, "root_intake_kind", RootIntakeKind::from_value)?,
        root_intake_id: optional_scalar(fields, "root_intake_id"),
        spec_id: optional_scalar(fields, "spec_id"),
        parent_task_id: optional_scalar(fields, "parent_task_id"),
        incident_id: optional_scalar(fields, "incident_id"),
        target_paths: required_list(fields, "target_paths")?,
        acceptance: required_list(fields, "acceptance")?,
        required_checks: required_list(fields, "required_checks")?,
        references: required_list(fields, "references")?,
        risk: required_list(fields, "risk")?,
        depends_on: optional_list(fields, "depends_on"),
        blocks: optional_list(fields, "blocks"),
        tags: optional_list(fields, "tags"),
        status_hint: optional_enum(fields, "status_hint", TaskStatusHint::from_value)?,
        created_at: required_timestamp(fields, "created_at")?,
        created_by: required_scalar(fields, "created_by")?,
        updated_at: optional_timestamp(fields, "updated_at")?,
    };
    document.validate()?;
    Ok(document)
}

fn parse_probe_from_fields(
    heading_title: &str,
    fields: &ParsedFields,
) -> Result<ProbeDocument, WorkDocumentError> {
    let document = ProbeDocument {
        probe_id: required_scalar(fields, "probe_id")?,
        title: required_title(fields, heading_title)?,
        summary: optional_scalar(fields, "summary").unwrap_or_default(),
        request: required_scalar(fields, "request")?,
        target_paths: optional_list(fields, "target_paths"),
        constraints: optional_list(fields, "constraints"),
        acceptance: optional_list(fields, "acceptance"),
        risk_notes: optional_list(fields, "risk_notes"),
        references: optional_list(fields, "references"),
        tags: optional_list(fields, "tags"),
        status_hint: optional_enum(fields, "status_hint", ProbeStatusHint::from_value)?,
        created_at: required_timestamp(fields, "created_at")?,
        created_by: required_scalar(fields, "created_by")?,
        updated_at: optional_timestamp(fields, "updated_at")?,
    };
    document.validate()?;
    Ok(document)
}

fn parse_spec_from_fields(
    heading_title: &str,
    fields: &ParsedFields,
) -> Result<SpecDocument, WorkDocumentError> {
    let document = SpecDocument {
        spec_id: required_scalar(fields, "spec_id")?,
        title: required_title(fields, heading_title)?,
        summary: required_scalar(fields, "summary")?,
        source_type: required_enum(fields, "source_type", SpecSourceType::from_value)?,
        source_id: optional_scalar(fields, "source_id"),
        parent_spec_id: optional_scalar(fields, "parent_spec_id"),
        root_idea_id: optional_scalar(fields, "root_idea_id"),
        root_spec_id: optional_scalar(fields, "root_spec_id"),
        root_intake_kind: optional_enum(fields, "root_intake_kind", RootIntakeKind::from_value)?,
        root_intake_id: optional_scalar(fields, "root_intake_id"),
        goals: required_list(fields, "goals")?,
        non_goals: optional_list(fields, "non_goals"),
        scope: optional_list(fields, "scope"),
        constraints: required_list(fields, "constraints")?,
        assumptions: optional_list(fields, "assumptions"),
        risks: optional_list(fields, "risks"),
        target_paths: optional_list(fields, "target_paths"),
        entrypoints: optional_list(fields, "entrypoints"),
        required_skills: optional_list(fields, "required_skills"),
        decomposition_hints: optional_list(fields, "decomposition_hints"),
        acceptance: required_list(fields, "acceptance")?,
        references: required_list(fields, "references")?,
        created_at: required_timestamp(fields, "created_at")?,
        created_by: required_scalar(fields, "created_by")?,
        updated_at: optional_timestamp(fields, "updated_at")?,
    };
    document.validate()?;
    Ok(document)
}

fn parse_incident_from_fields(
    heading_title: &str,
    fields: &ParsedFields,
) -> Result<IncidentDocument, WorkDocumentError> {
    let document = IncidentDocument {
        incident_id: required_scalar(fields, "incident_id")?,
        title: required_title(fields, heading_title)?,
        summary: required_scalar(fields, "summary")?,
        root_idea_id: optional_scalar(fields, "root_idea_id"),
        root_spec_id: optional_scalar(fields, "root_spec_id"),
        root_intake_kind: optional_enum(fields, "root_intake_kind", RootIntakeKind::from_value)?,
        root_intake_id: optional_scalar(fields, "root_intake_id"),
        source_task_id: optional_scalar(fields, "source_task_id"),
        source_spec_id: optional_scalar(fields, "source_spec_id"),
        source_stage: required_enum(fields, "source_stage", StageName::from_value)?,
        source_plane: required_enum(fields, "source_plane", Plane::from_value)?,
        failure_class: required_scalar(fields, "failure_class")?,
        severity: optional_enum(fields, "severity", IncidentSeverity::from_value)?
            .unwrap_or(IncidentSeverity::Medium),
        needs_planning: optional_bool(fields, "needs_planning")?.unwrap_or(true),
        trigger_reason: required_scalar(fields, "trigger_reason")?,
        observed_symptoms: optional_list(fields, "observed_symptoms"),
        failed_attempts: optional_list(fields, "failed_attempts"),
        consultant_decision: required_enum(
            fields,
            "consultant_decision",
            IncidentDecision::from_value,
        )?,
        evidence_paths: optional_list(fields, "evidence_paths"),
        related_run_ids: optional_list(fields, "related_run_ids"),
        related_stage_results: optional_list(fields, "related_stage_results"),
        references: optional_list(fields, "references"),
        opened_at: required_timestamp(fields, "opened_at")?,
        opened_by: required_scalar(fields, "opened_by")?,
        updated_at: optional_timestamp(fields, "updated_at")?,
    };
    document.validate()?;
    Ok(document)
}

fn parse_learning_request_from_fields(
    heading_title: &str,
    fields: &ParsedFields,
) -> Result<LearningRequestDocument, WorkDocumentError> {
    let document = LearningRequestDocument {
        learning_request_id: required_scalar(fields, "learning_request_id")?,
        title: required_title(fields, heading_title)?,
        summary: optional_scalar(fields, "summary").unwrap_or_default(),
        requested_action: required_enum(
            fields,
            "requested_action",
            LearningRequestAction::from_value,
        )?,
        target_skill_id: optional_scalar(fields, "target_skill_id"),
        target_stage: optional_enum(fields, "target_stage", LearningStageName::from_value)?,
        source_refs: optional_list(fields, "source_refs"),
        preferred_output_paths: optional_list(fields, "preferred_output_paths"),
        trigger_metadata: optional_trigger_metadata(fields)?,
        originating_run_ids: optional_list(fields, "originating_run_ids"),
        artifact_paths: optional_list(fields, "artifact_paths"),
        references: optional_list(fields, "references"),
        created_at: required_timestamp(fields, "created_at")?,
        created_by: required_scalar(fields, "created_by")?,
        updated_at: optional_timestamp(fields, "updated_at")?,
    };
    document.validate()?;
    Ok(document)
}

fn required_title(fields: &ParsedFields, heading_title: &str) -> Result<String, WorkDocumentError> {
    let title = required_scalar(fields, "title")?;
    if title.trim() != heading_title {
        return Err(WorkDocumentError::InvalidDocument {
            message: "markdown H1 title must match the `Title` field".to_owned(),
        });
    }
    Ok(title)
}

fn required_scalar(
    fields: &ParsedFields,
    field_name: &'static str,
) -> Result<String, WorkDocumentError> {
    fields
        .scalars
        .get(field_name)
        .cloned()
        .ok_or(WorkDocumentError::MissingRequiredField { field_name })
}

fn optional_scalar(fields: &ParsedFields, field_name: &'static str) -> Option<String> {
    fields.scalars.get(field_name).cloned()
}

fn required_list(
    fields: &ParsedFields,
    field_name: &'static str,
) -> Result<Vec<String>, WorkDocumentError> {
    let values = optional_list(fields, field_name);
    if values.is_empty() {
        Err(WorkDocumentError::EmptyRequiredList { field_name })
    } else {
        Ok(values)
    }
}

fn optional_list(fields: &ParsedFields, field_name: &'static str) -> Vec<String> {
    fields.lists.get(field_name).cloned().unwrap_or_default()
}

fn required_timestamp(
    fields: &ParsedFields,
    field_name: &'static str,
) -> Result<Timestamp, WorkDocumentError> {
    Timestamp::parse(field_name, &required_scalar(fields, field_name)?)
}

fn optional_timestamp(
    fields: &ParsedFields,
    field_name: &'static str,
) -> Result<Option<Timestamp>, WorkDocumentError> {
    optional_scalar(fields, field_name)
        .as_deref()
        .map(|value| Timestamp::parse(field_name, value))
        .transpose()
}

fn required_enum<T>(
    fields: &ParsedFields,
    field_name: &'static str,
    parse: fn(&str) -> Result<T, crate::contracts::ContractError>,
) -> Result<T, WorkDocumentError> {
    parse(&required_scalar(fields, field_name)?).map_err(Into::into)
}

fn optional_enum<T>(
    fields: &ParsedFields,
    field_name: &'static str,
    parse: fn(&str) -> Result<T, crate::contracts::ContractError>,
) -> Result<Option<T>, WorkDocumentError> {
    optional_scalar(fields, field_name)
        .as_deref()
        .map(parse)
        .transpose()
        .map_err(Into::into)
}

fn optional_bool(
    fields: &ParsedFields,
    field_name: &'static str,
) -> Result<Option<bool>, WorkDocumentError> {
    optional_scalar(fields, field_name)
        .as_deref()
        .map(|value| match value {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(WorkDocumentError::InvalidField {
                field_name,
                value: value.to_owned(),
                message: "must be `true` or `false`".to_owned(),
            }),
        })
        .transpose()
}

fn optional_trigger_metadata(fields: &ParsedFields) -> Result<Value, WorkDocumentError> {
    let Some(raw) = optional_scalar(fields, "trigger_metadata") else {
        return Ok(Value::Object(Map::new()));
    };
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| WorkDocumentError::InvalidField {
            field_name: "trigger_metadata",
            value: raw.clone(),
            message: error.to_string(),
        })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(WorkDocumentError::InvalidField {
            field_name: "trigger_metadata",
            value: raw,
            message: "must be a JSON object".to_owned(),
        })
    }
}

fn field_spec_by_label(label: &str) -> Option<FieldSpec> {
    let (field_name, kind) = match label {
        "Task-ID" => ("task_id", FieldKind::Scalar),
        "Probe-ID" => ("probe_id", FieldKind::Scalar),
        "Spec-ID" => ("spec_id", FieldKind::Scalar),
        "Incident-ID" => ("incident_id", FieldKind::Scalar),
        "Learning-Request-ID" => ("learning_request_id", FieldKind::Scalar),
        "Title" => ("title", FieldKind::Scalar),
        "Summary" => ("summary", FieldKind::Scalar),
        "Request" => ("request", FieldKind::Scalar),
        "Root-Idea-ID" => ("root_idea_id", FieldKind::Scalar),
        "Root-Spec-ID" => ("root_spec_id", FieldKind::Scalar),
        "Root-Intake-Kind" => ("root_intake_kind", FieldKind::Scalar),
        "Root-Intake-ID" => ("root_intake_id", FieldKind::Scalar),
        "Parent-Task-ID" => ("parent_task_id", FieldKind::Scalar),
        "Status-Hint" => ("status_hint", FieldKind::Scalar),
        "Created-At" => ("created_at", FieldKind::Scalar),
        "Created-By" => ("created_by", FieldKind::Scalar),
        "Updated-At" => ("updated_at", FieldKind::Scalar),
        "Source-Type" => ("source_type", FieldKind::Scalar),
        "Source-ID" => ("source_id", FieldKind::Scalar),
        "Parent-Spec-ID" => ("parent_spec_id", FieldKind::Scalar),
        "Source-Task-ID" => ("source_task_id", FieldKind::Scalar),
        "Source-Spec-ID" => ("source_spec_id", FieldKind::Scalar),
        "Source-Stage" => ("source_stage", FieldKind::Scalar),
        "Source-Plane" => ("source_plane", FieldKind::Scalar),
        "Failure-Class" => ("failure_class", FieldKind::Scalar),
        "Severity" => ("severity", FieldKind::Scalar),
        "Needs-Planning" => ("needs_planning", FieldKind::Scalar),
        "Trigger-Reason" => ("trigger_reason", FieldKind::Scalar),
        "Consultant-Decision" => ("consultant_decision", FieldKind::Scalar),
        "Opened-At" => ("opened_at", FieldKind::Scalar),
        "Opened-By" => ("opened_by", FieldKind::Scalar),
        "Requested-Action" => ("requested_action", FieldKind::Scalar),
        "Target-Skill-ID" => ("target_skill_id", FieldKind::Scalar),
        "Target-Stage" => ("target_stage", FieldKind::Scalar),
        "Trigger-Metadata" => ("trigger_metadata", FieldKind::Scalar),
        "Depends-On" => ("depends_on", FieldKind::List),
        "Blocks" => ("blocks", FieldKind::List),
        "Tags" => ("tags", FieldKind::List),
        "Target-Paths" => ("target_paths", FieldKind::List),
        "Acceptance" => ("acceptance", FieldKind::List),
        "Required-Checks" => ("required_checks", FieldKind::List),
        "References" => ("references", FieldKind::List),
        "Risk" => ("risk", FieldKind::List),
        "Goals" => ("goals", FieldKind::List),
        "Non-Goals" => ("non_goals", FieldKind::List),
        "Scope" => ("scope", FieldKind::List),
        "Constraints" => ("constraints", FieldKind::List),
        "Assumptions" => ("assumptions", FieldKind::List),
        "Risks" => ("risks", FieldKind::List),
        "Risk-Notes" => ("risk_notes", FieldKind::List),
        "Entrypoints" => ("entrypoints", FieldKind::List),
        "Required-Skills" => ("required_skills", FieldKind::List),
        "Decomposition-Hints" => ("decomposition_hints", FieldKind::List),
        "Observed-Symptoms" => ("observed_symptoms", FieldKind::List),
        "Failed-Attempts" => ("failed_attempts", FieldKind::List),
        "Evidence-Paths" => ("evidence_paths", FieldKind::List),
        "Related-Run-IDs" => ("related_run_ids", FieldKind::List),
        "Related-Stage-Results" => ("related_stage_results", FieldKind::List),
        "Source-Refs" => ("source_refs", FieldKind::List),
        "Preferred-Output-Paths" => ("preferred_output_paths", FieldKind::List),
        "Originating-Run-IDs" => ("originating_run_ids", FieldKind::List),
        "Artifact-Paths" => ("artifact_paths", FieldKind::List),
        _ => return None,
    };

    Some(FieldSpec { field_name, kind })
}

fn normalize_list_items(field_name: &str, items: Vec<String>) -> Vec<String> {
    if !matches!(field_name, "depends_on" | "blocks") {
        return items;
    }

    items
        .into_iter()
        .filter(|item| {
            !matches!(
                item.trim().to_ascii_lowercase().as_str(),
                "none" | "n/a" | "-"
            )
        })
        .collect()
}

fn heading_lines(title: &str) -> Vec<String> {
    vec![format!("# {title}"), String::new()]
}

fn push_scalar(lines: &mut Vec<String>, label: &str, value: &str) {
    lines.push(format!("{label}: {value}"));
}

fn push_optional_scalar(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_scalar(lines, label, value);
    }
}

fn push_optional_non_empty_scalar(lines: &mut Vec<String>, label: &str, value: &str) {
    if !value.is_empty() {
        push_scalar(lines, label, value);
    }
}

fn push_list(lines: &mut Vec<String>, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{label}:"));
    lines.extend(values.iter().map(|item| format!("- {item}")));
}

fn finish_lines(lines: Vec<String>) -> String {
    let mut rendered = lines.join("\n");
    rendered.push('\n');
    rendered
}

fn render_json_scalar(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn parse_json_import<T>(
    raw: &str,
    source_name: &str,
    expected_kind: &str,
) -> Result<T, WorkDocumentError>
where
    T: DeserializeOwned,
{
    let value =
        serde_json::from_str::<Value>(raw).map_err(|error| WorkDocumentError::InvalidDocument {
            message: format!("work document JSON import {source_name} failed to decode: {error}"),
        })?;
    validate_json_import_metadata(&value, source_name, expected_kind)?;
    serde_json::from_value(value).map_err(|error| WorkDocumentError::InvalidDocument {
        message: format!("work document JSON import {source_name} failed to decode: {error}"),
    })
}

fn validate_json_import_metadata(
    value: &Value,
    source_name: &str,
    expected_kind: &str,
) -> Result<(), WorkDocumentError> {
    let Some(object) = value.as_object() else {
        return Err(WorkDocumentError::InvalidDocument {
            message: format!("work document JSON import {source_name} must be a JSON object"),
        });
    };

    if let Some(schema_version) = object.get("schema_version") {
        if schema_version != WORK_DOCUMENT_SCHEMA_VERSION {
            return Err(WorkDocumentError::InvalidField {
                field_name: "schema_version",
                value: schema_version.to_string(),
                message: format!("must be literal `{WORK_DOCUMENT_SCHEMA_VERSION}`"),
            });
        }
    }
    if let Some(kind) = object.get("kind") {
        if kind != expected_kind {
            return Err(WorkDocumentError::InvalidField {
                field_name: "kind",
                value: kind.to_string(),
                message: format!("must be literal `{expected_kind}`"),
            });
        }
    }
    Ok(())
}

fn model_name(model: DocumentModel) -> &'static str {
    match model {
        DocumentModel::Task => WorkItemKind::Task.as_str(),
        DocumentModel::Probe => WorkItemKind::Probe.as_str(),
        DocumentModel::Spec => WorkItemKind::Spec.as_str(),
        DocumentModel::Incident => WorkItemKind::Incident.as_str(),
        DocumentModel::LearningRequest => WorkItemKind::LearningRequest.as_str(),
    }
}

fn source_name_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<path>")
        .to_owned()
}
