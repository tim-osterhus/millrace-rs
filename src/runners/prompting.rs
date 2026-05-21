//! Shared Millrace-owned prompt construction for runner adapters.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::runtime::StageRunRequest;

use super::{RunnerError, RunnerResult};

/// Returns the legal terminal markers the adapter prompt must preserve.
#[must_use]
pub fn legal_terminal_markers(request: &StageRunRequest) -> &[String] {
    &request.legal_terminal_markers
}

/// Builds the canonical stage prompt consumed by concrete runner adapters.
#[must_use]
pub fn build_stage_prompt(request: &StageRunRequest) -> String {
    let legal_markers = legal_terminal_markers(request)
        .iter()
        .map(|marker| format!("`{marker}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![
        "You are executing one Millrace runtime stage request.".to_owned(),
        format!(
            "Open `{}` and follow instructions exactly.",
            request.entrypoint_path
        ),
        String::new(),
        "Stage Request Context:".to_owned(),
    ];
    lines.extend(request.render_context_lines());
    if let Some(path) = request.rendered_prompt_context_path.as_deref()
        && let Ok(rendered_context) = fs::read_to_string(path)
    {
        lines.extend([
            String::new(),
            "Rendered Request Context:".to_owned(),
            rendered_context.trim_end().to_owned(),
        ]);
    }
    lines.extend([
        String::new(),
        "When done, print exactly one legal terminal marker defined by the opened entrypoint contract."
            .to_owned(),
        format!("Legal markers for this stage: {legal_markers}."),
        "Do not invent or rename terminal markers.".to_owned(),
        "Do not print multiple terminal markers.".to_owned(),
    ]);
    lines.join("\n")
}

/// Returns the canonical prompt artifact path for one stage request.
#[must_use]
pub fn runner_prompt_path(request: &StageRunRequest) -> PathBuf {
    Path::new(&request.run_dir).join(format!("runner_prompt.{}.md", request.request_id))
}

/// Builds and persists the canonical stage prompt under the request run directory.
pub fn write_stage_prompt_artifact(request: &StageRunRequest) -> RunnerResult<PathBuf> {
    let path = runner_prompt_path(request);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| RunnerError::Io {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    fs::write(&path, build_stage_prompt(request)).map_err(|error| RunnerError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(path)
}
