use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    work_documents::{
        read_probe_import_document, read_spec_import_document, read_task_import_document,
    },
    workspace::{RuntimeControl, RuntimeControlActionResult, RuntimeControlMode, WorkspacePaths},
};

pub fn add_task_lines(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    add_task_lines_inner(paths, input_path).map_err(|error| format!("failed to add task: {error}"))
}

pub fn add_spec_lines(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    add_spec_lines_inner(paths, input_path).map_err(|error| format!("failed to add spec: {error}"))
}

pub fn add_probe_lines(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    add_probe_lines_inner(paths, input_path)
        .map_err(|error| format!("failed to add probe: {error}"))
}

pub fn add_idea_lines(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    add_idea_lines_inner(paths, input_path).map_err(|error| format!("failed to add idea: {error}"))
}

fn add_task_lines_inner(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    let input_path = existing_file_path(input_path, "task")?;
    let document = read_task_import_document(&input_path).map_err(|error| error.to_string())?;
    ensure_filename_matches(&input_path, "task_id", &document.task_id)?;
    let result = RuntimeControl::from_paths(paths.clone())
        .map_err(|error| error.to_string())?
        .add_task(&document)
        .map_err(|error| error.to_string())?;
    render_intake_result(&result, "enqueued_task")
}

fn add_probe_lines_inner(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    let input_path = existing_file_path(input_path, "probe")?;
    let document = read_probe_import_document(&input_path).map_err(|error| error.to_string())?;
    ensure_filename_matches(&input_path, "probe_id", &document.probe_id)?;
    let result = RuntimeControl::from_paths(paths.clone())
        .map_err(|error| error.to_string())?
        .add_probe(&document)
        .map_err(|error| error.to_string())?;
    render_intake_result(&result, "enqueued_probe")
}

fn add_spec_lines_inner(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    let input_path = existing_file_path(input_path, "spec")?;
    let document = read_spec_import_document(&input_path).map_err(|error| error.to_string())?;
    ensure_filename_matches(&input_path, "spec_id", &document.spec_id)?;
    let result = RuntimeControl::from_paths(paths.clone())
        .map_err(|error| error.to_string())?
        .add_spec(&document)
        .map_err(|error| error.to_string())?;
    render_intake_result(&result, "enqueued_spec")
}

fn add_idea_lines_inner(paths: &WorkspacePaths, input_path: &str) -> Result<Vec<String>, String> {
    let input_path = existing_file_path(input_path, "idea")?;
    let source_name = input_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("idea path has no UTF-8 filename: {}", input_path.display()))?
        .to_owned();
    let markdown = fs::read_to_string(&input_path)
        .map_err(|error| format!("failed to read {}: {error}", input_path.display()))?;
    let result = RuntimeControl::from_paths(paths.clone())
        .map_err(|error| error.to_string())?
        .add_idea_markdown(&source_name, &markdown)
        .map_err(|error| error.to_string())?;
    render_intake_result(&result, "enqueued_idea")
}

fn existing_file_path(input_path: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(input_path);
    if !path.is_file() {
        return Err(format!(
            "{label} path must be an existing file: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn ensure_filename_matches(path: &Path, field_name: &str, document_id: &str) -> Result<(), String> {
    let filename_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("input path has no UTF-8 filename stem: {}", path.display()))?;
    if filename_id == document_id {
        return Ok(());
    }
    Err(format!(
        "filename stem does not match {field_name}: expected {document_id}, found {filename_id}"
    ))
}

fn render_intake_result(
    result: &RuntimeControlActionResult,
    direct_label: &str,
) -> Result<Vec<String>, String> {
    if result.mode == RuntimeControlMode::Direct {
        let artifact_path = result
            .artifact_path
            .as_ref()
            .ok_or_else(|| "missing artifact path".to_owned())?;
        return Ok(vec![format!("{direct_label}: {}", artifact_path.display())]);
    }

    let mut lines = vec![
        format!("action: {}", result.action.as_str()),
        format!("mode: {}", result.mode.as_str()),
        format!("applied: {}", bool_text(result.applied)),
        format!("detail: {}", result.detail),
    ];
    if let Some(command_id) = &result.command_id {
        lines.push(format!("command_id: {command_id}"));
    }
    if let Some(mailbox_path) = &result.mailbox_path {
        lines.push(format!("mailbox_path: {}", mailbox_path.display()));
    }
    if let Some(artifact_path) = &result.artifact_path {
        lines.push(format!("artifact_path: {}", artifact_path.display()));
    }
    Ok(lines)
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
