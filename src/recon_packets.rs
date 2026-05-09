//! Recon packet markdown parsing and rendering helpers.

use std::{collections::BTreeMap, fs, path::Path};

use crate::contracts::{
    ReconConfidence, ReconDecision, ReconHandoffTarget, ReconPacketDocument, ReconPacketError,
    ReconPathFinding, ReconRiskLevel, ReconVerificationPlan, Timestamp,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Scalar,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FieldSpec {
    field_name: &'static str,
    kind: FieldKind,
}

#[derive(Debug, Default)]
struct ParsedReconFields {
    scalars: BTreeMap<&'static str, String>,
    lists: BTreeMap<&'static str, Vec<String>>,
}

/// Render a canonical operator-facing Recon packet.
#[must_use]
pub fn render_recon_packet(packet: &ReconPacketDocument) -> String {
    let mut lines = vec![
        format!("# Recon Packet {}", packet.recon_packet_id),
        String::new(),
    ];

    push_scalar(&mut lines, "Recon-Packet-ID", &packet.recon_packet_id);
    push_scalar(&mut lines, "Probe-ID", &packet.probe_id);
    push_scalar(&mut lines, "Decision", packet.decision.as_str());
    push_scalar(&mut lines, "Confidence", packet.confidence.as_str());
    push_scalar(&mut lines, "Risk-Level", packet.risk_level.as_str());
    push_scalar(&mut lines, "Request-Summary", &packet.request_summary);
    push_scalar(&mut lines, "Interpreted-Goal", &packet.interpreted_goal);
    push_scalar(&mut lines, "Handoff-Target", packet.handoff_target.as_str());
    push_optional_scalar(
        &mut lines,
        "Emitted-Task-ID",
        packet.emitted_task_id.as_deref(),
    );
    push_optional_scalar(
        &mut lines,
        "Emitted-Spec-ID",
        packet.emitted_spec_id.as_deref(),
    );
    push_scalar(&mut lines, "Created-At", packet.created_at.as_str());
    push_scalar(&mut lines, "Created-By", &packet.created_by);

    push_path_findings(&mut lines, "Relevant-Paths", &packet.relevant_paths);
    push_list(&mut lines, "Relevant-Symbols", &packet.relevant_symbols);
    push_path_findings(&mut lines, "Relevant-Tests", &packet.relevant_tests);
    push_list(
        &mut lines,
        "Semantic-Invariants",
        &packet.semantic_invariants,
    );
    push_list(
        &mut lines,
        "Edge-Cases-To-Preserve",
        &packet.edge_cases_to_preserve,
    );
    push_list(
        &mut lines,
        "Required-Commands",
        &packet.verification_plan.required_commands,
    );
    push_list(
        &mut lines,
        "Focused-Checks",
        &packet.verification_plan.focused_checks,
    );
    push_list(
        &mut lines,
        "Fallback-Checks",
        &packet.verification_plan.fallback_checks,
    );
    push_list(&mut lines, "Open-Questions", &packet.open_questions);

    finish_lines(lines)
}

/// Parse one canonical Recon packet markdown artifact.
pub fn parse_recon_packet(raw: &str) -> Result<ReconPacketDocument, ReconPacketError> {
    parse_recon_packet_with_source(raw, "<memory>")
}

/// Parse one canonical Recon packet markdown artifact with source context.
pub fn parse_recon_packet_with_source(
    raw: &str,
    source_name: &str,
) -> Result<ReconPacketDocument, ReconPacketError> {
    let fields = parse_markdown_fields(raw, source_name)?;
    let mut packet = ReconPacketDocument {
        schema_version: "1.0".to_owned(),
        kind: "recon_packet".to_owned(),
        recon_packet_id: required_scalar(&fields, "recon_packet_id")?,
        probe_id: required_scalar(&fields, "probe_id")?,
        decision: required_enum(&fields, "decision", ReconDecision::from_value)?,
        confidence: required_enum(&fields, "confidence", ReconConfidence::from_value)?,
        risk_level: required_enum(&fields, "risk_level", ReconRiskLevel::from_value)?,
        request_summary: required_scalar(&fields, "request_summary")?,
        interpreted_goal: required_scalar(&fields, "interpreted_goal")?,
        relevant_paths: required_path_findings(&fields, "relevant_paths")?,
        relevant_symbols: optional_list(&fields, "relevant_symbols"),
        relevant_tests: optional_path_findings(&fields, "relevant_tests")?,
        semantic_invariants: required_list(&fields, "semantic_invariants")?,
        edge_cases_to_preserve: optional_list(&fields, "edge_cases_to_preserve"),
        verification_plan: ReconVerificationPlan {
            required_commands: optional_list(&fields, "required_commands"),
            focused_checks: optional_list(&fields, "focused_checks"),
            fallback_checks: optional_list(&fields, "fallback_checks"),
        },
        open_questions: optional_list(&fields, "open_questions"),
        handoff_target: required_enum(&fields, "handoff_target", ReconHandoffTarget::from_value)?,
        emitted_task_id: optional_scalar(&fields, "emitted_task_id"),
        emitted_spec_id: optional_scalar(&fields, "emitted_spec_id"),
        created_at: required_timestamp(&fields, "created_at")?,
        created_by: optional_scalar(&fields, "created_by").unwrap_or_else(|| "recon".to_owned()),
    };
    packet.validate()?;
    Ok(packet)
}

/// Read and parse one Recon packet artifact.
pub fn read_recon_packet(path: &Path) -> Result<ReconPacketDocument, ReconPacketError> {
    let raw = fs::read_to_string(path).map_err(|error| ReconPacketError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    parse_recon_packet_with_source(&raw, source_name_for_path(path).as_ref())
}

fn parse_markdown_fields(
    raw: &str,
    source_name: &str,
) -> Result<ParsedReconFields, ReconPacketError> {
    let lines: Vec<&str> = raw.lines().collect();
    if lines.is_empty() {
        return Err(ReconPacketError::InvalidDocument {
            message: format!("recon packet {source_name} is empty"),
        });
    }
    if parse_h1_title(lines[0]).is_none() {
        return Err(ReconPacketError::InvalidDocument {
            message: format!("recon packet {source_name} must start with a markdown H1 title"),
        });
    }

    let mut fields = ParsedReconFields::default();
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
            return Err(ReconPacketError::InvalidDocument {
                message: format!("recon packet {source_name} repeats field `{label}`"),
            });
        }

        if field_spec.kind == FieldKind::Scalar {
            if inline_value.is_empty() {
                return Err(ReconPacketError::InvalidDocument {
                    message: format!("recon packet {source_name} has empty scalar `{label}`"),
                });
            }
            fields
                .scalars
                .insert(field_spec.field_name, inline_value.to_owned());
            continue;
        }

        let mut items = Vec::new();
        if !inline_value.is_empty() {
            items.push(inline_value.to_owned());
        }
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
                return Err(ReconPacketError::InvalidDocument {
                    message: format!(
                        "recon packet {source_name} has invalid list item under `{label}`"
                    ),
                });
            };
            items.push(item.to_owned());
            index += 1;
        }
        fields.lists.insert(field_spec.field_name, items);
    }

    Ok(fields)
}

fn parse_h1_title(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('#')?;
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_whitespace() {
        return None;
    }
    let title = rest[first.len_utf8()..].trim();
    if title.is_empty() { None } else { Some(title) }
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

fn field_spec_by_label(label: &str) -> Option<FieldSpec> {
    let (field_name, kind) = match label {
        "Recon-Packet-ID" => ("recon_packet_id", FieldKind::Scalar),
        "Probe-ID" => ("probe_id", FieldKind::Scalar),
        "Decision" => ("decision", FieldKind::Scalar),
        "Confidence" => ("confidence", FieldKind::Scalar),
        "Risk-Level" => ("risk_level", FieldKind::Scalar),
        "Request-Summary" => ("request_summary", FieldKind::Scalar),
        "Interpreted-Goal" => ("interpreted_goal", FieldKind::Scalar),
        "Handoff-Target" => ("handoff_target", FieldKind::Scalar),
        "Emitted-Task-ID" => ("emitted_task_id", FieldKind::Scalar),
        "Emitted-Spec-ID" => ("emitted_spec_id", FieldKind::Scalar),
        "Created-At" => ("created_at", FieldKind::Scalar),
        "Created-By" => ("created_by", FieldKind::Scalar),
        "Relevant-Paths" => ("relevant_paths", FieldKind::List),
        "Relevant-Symbols" => ("relevant_symbols", FieldKind::List),
        "Relevant-Tests" => ("relevant_tests", FieldKind::List),
        "Semantic-Invariants" => ("semantic_invariants", FieldKind::List),
        "Edge-Cases-To-Preserve" => ("edge_cases_to_preserve", FieldKind::List),
        "Required-Commands" => ("required_commands", FieldKind::List),
        "Focused-Checks" => ("focused_checks", FieldKind::List),
        "Fallback-Checks" => ("fallback_checks", FieldKind::List),
        "Open-Questions" => ("open_questions", FieldKind::List),
        _ => return None,
    };
    Some(FieldSpec { field_name, kind })
}

fn required_scalar(
    fields: &ParsedReconFields,
    field_name: &'static str,
) -> Result<String, ReconPacketError> {
    fields
        .scalars
        .get(field_name)
        .cloned()
        .ok_or(ReconPacketError::MissingRequiredField { field_name })
}

fn optional_scalar(fields: &ParsedReconFields, field_name: &'static str) -> Option<String> {
    fields.scalars.get(field_name).cloned()
}

fn required_list(
    fields: &ParsedReconFields,
    field_name: &'static str,
) -> Result<Vec<String>, ReconPacketError> {
    let values = optional_list(fields, field_name);
    if values.is_empty() {
        Err(ReconPacketError::EmptyRequiredList { field_name })
    } else {
        Ok(values)
    }
}

fn optional_list(fields: &ParsedReconFields, field_name: &'static str) -> Vec<String> {
    fields.lists.get(field_name).cloned().unwrap_or_default()
}

fn required_path_findings(
    fields: &ParsedReconFields,
    field_name: &'static str,
) -> Result<Vec<ReconPathFinding>, ReconPacketError> {
    let values = optional_path_findings(fields, field_name)?;
    if values.is_empty() {
        Err(ReconPacketError::EmptyRequiredList { field_name })
    } else {
        Ok(values)
    }
}

fn optional_path_findings(
    fields: &ParsedReconFields,
    field_name: &'static str,
) -> Result<Vec<ReconPathFinding>, ReconPacketError> {
    optional_list(fields, field_name)
        .into_iter()
        .map(parse_path_finding)
        .collect()
}

fn parse_path_finding(value: String) -> Result<ReconPathFinding, ReconPacketError> {
    let (path, reason) = value
        .split_once('|')
        .ok_or_else(|| ReconPacketError::InvalidField {
            field_name: "path_finding",
            value: value.clone(),
            message: "must use `path | reason`".to_owned(),
        })?;
    let finding = ReconPathFinding {
        path: path.trim().to_owned(),
        reason: reason.trim().to_owned(),
    };
    finding.validate()?;
    Ok(finding)
}

fn required_enum<T>(
    fields: &ParsedReconFields,
    field_name: &'static str,
    parse: fn(&str) -> Result<T, ReconPacketError>,
) -> Result<T, ReconPacketError> {
    parse(&required_scalar(fields, field_name)?)
}

fn required_timestamp(
    fields: &ParsedReconFields,
    field_name: &'static str,
) -> Result<Timestamp, ReconPacketError> {
    Timestamp::parse(field_name, &required_scalar(fields, field_name)?).map_err(|error| {
        ReconPacketError::InvalidField {
            field_name,
            value: error.to_string(),
            message: "must be an RFC 3339 timestamp".to_owned(),
        }
    })
}

fn push_scalar(lines: &mut Vec<String>, label: &str, value: &str) {
    lines.push(format!("{label}: {value}"));
}

fn push_optional_scalar(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value {
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

fn push_path_findings(lines: &mut Vec<String>, label: &str, values: &[ReconPathFinding]) {
    if values.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{label}:"));
    lines.extend(
        values
            .iter()
            .map(|finding| format!("- {} | {}", finding.path, finding.reason)),
    );
}

fn finish_lines(lines: Vec<String>) -> String {
    let mut rendered = lines.join("\n");
    rendered.push('\n');
    rendered
}

fn source_name_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<path>")
        .to_owned()
}
