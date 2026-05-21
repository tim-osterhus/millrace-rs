//! Deterministic request-context artifact rendering.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::compiler::CompiledRunPlan;

use super::{StageRunRequest, StageRunRequestError};

/// Runtime render input for one stage request context bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContextRenderPlan {
    pub render_plan_id: String,
    pub context_bundle_path: String,
    pub rendered_prompt_context_path: String,
    pub profile_id: String,
    #[serde(default)]
    pub visible_context_refs: Vec<String>,
    #[serde(default)]
    pub operator_only_context_refs: Vec<String>,
    #[serde(default)]
    pub included_provider_ids: Vec<String>,
    #[serde(default)]
    pub redacted_provider_ids: Vec<String>,
    #[serde(default)]
    pub inline_sections: Vec<String>,
    #[serde(default)]
    pub omitted_provider_ids: Vec<String>,
    #[serde(default)]
    pub artifact_contract_source: Option<String>,
    #[serde(default)]
    pub output_artifact_contract_ids: Vec<String>,
}

/// Paths and text emitted by a deterministic context render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedRequestContext {
    pub context_bundle_path: String,
    pub rendered_prompt_context_path: String,
    pub render_manifest_path: String,
    pub text: String,
}

/// Write the default stage request context artifacts and return an enriched request.
pub fn attach_default_request_context(
    workspace_root: &Path,
    mut request: StageRunRequest,
    compiled_plan: Option<&CompiledRunPlan>,
) -> Result<StageRunRequest, StageRunRequestError> {
    let plan = request_context_plan(workspace_root, &request, compiled_plan);
    let rendered = render_request_context(workspace_root, &plan)?;
    request.request_context_profile_id = Some(plan.profile_id);
    request.context_bundle_path = Some(rendered.context_bundle_path);
    request.rendered_prompt_context_path = Some(rendered.rendered_prompt_context_path);
    request.context_render_plan_id = Some(plan.render_plan_id);
    request.context_artifact_refs = plan.visible_context_refs;
    request.validate()?;
    Ok(request)
}

/// Write context bundle, rendered markdown, and manifest for a render plan.
pub fn render_request_context(
    workspace_root: &Path,
    plan: &RequestContextRenderPlan,
) -> Result<RenderedRequestContext, StageRunRequestError> {
    validate_plan(plan)?;
    let bundle_path = resolve_workspace_path(workspace_root, &plan.context_bundle_path);
    let rendered_path = resolve_workspace_path(workspace_root, &plan.rendered_prompt_context_path);
    let manifest_path = rendered_path.with_file_name("render_manifest.json");
    let bundle_payload = context_bundle_payload(plan);
    let text = render_markdown(plan);
    let manifest = render_manifest(plan, &text);

    atomic_write_text(
        &bundle_path,
        &(serde_json::to_string_pretty(&bundle_payload).map_err(json_error)? + "\n"),
    )?;
    atomic_write_text(&rendered_path, &text)?;
    atomic_write_text(
        &manifest_path,
        &(serde_json::to_string_pretty(&manifest).map_err(json_error)? + "\n"),
    )?;

    Ok(RenderedRequestContext {
        context_bundle_path: bundle_path.display().to_string(),
        rendered_prompt_context_path: rendered_path.display().to_string(),
        render_manifest_path: manifest_path.display().to_string(),
        text,
    })
}

fn request_context_plan(
    workspace_root: &Path,
    request: &StageRunRequest,
    compiled_plan: Option<&CompiledRunPlan>,
) -> RequestContextRenderPlan {
    let context_dir = Path::new(&request.run_dir).join("context");
    let profile_id = request
        .request_context_profile_id
        .clone()
        .unwrap_or_else(|| format!("{}.default", request.stage_kind_id));
    let mut visible_context_refs = visible_context_refs(workspace_root, request);
    visible_context_refs.extend(preferred_output_refs(request, compiled_plan));
    unique_preserve_order(&mut visible_context_refs);
    RequestContextRenderPlan {
        render_plan_id: format!("{}.context.v1", request.stage_kind_id),
        context_bundle_path: context_dir
            .join("request_context.json")
            .display()
            .to_string(),
        rendered_prompt_context_path: context_dir.join("prompt_context.md").display().to_string(),
        profile_id,
        visible_context_refs,
        operator_only_context_refs: vec![
            format!("runtime_snapshot:{}", request.runtime_snapshot_path),
            format!("recovery_counters:{}", request.recovery_counters_path),
        ],
        included_provider_ids: included_provider_ids(request),
        redacted_provider_ids: vec!["runtime_control_state".to_owned()],
        inline_sections: inline_sections(request),
        omitted_provider_ids: Vec::new(),
        artifact_contract_source: None,
        output_artifact_contract_ids: Vec::new(),
    }
}

fn validate_plan(plan: &RequestContextRenderPlan) -> Result<(), StageRunRequestError> {
    require_non_blank("render_plan_id", &plan.render_plan_id)?;
    require_non_blank("context_bundle_path", &plan.context_bundle_path)?;
    require_non_blank(
        "rendered_prompt_context_path",
        &plan.rendered_prompt_context_path,
    )?;
    require_non_blank("profile_id", &plan.profile_id)?;
    Ok(())
}

fn context_bundle_payload(plan: &RequestContextRenderPlan) -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "request_context_bundle",
        "profile_id": plan.profile_id,
        "render_plan_id": plan.render_plan_id,
        "visible_context_refs": plan.visible_context_refs,
        "operator_only_context_refs": plan.operator_only_context_refs,
        "included_provider_ids": plan.included_provider_ids,
        "redacted_provider_ids": plan.redacted_provider_ids,
        "inline_sections": plan.inline_sections,
        "omitted_provider_ids": plan.omitted_provider_ids,
        "artifact_contract_source": plan.artifact_contract_source,
        "output_artifact_contract_ids": plan.output_artifact_contract_ids,
    })
}

fn render_markdown(plan: &RequestContextRenderPlan) -> String {
    let mut lines = vec![
        "# Request Context".to_owned(),
        String::new(),
        format!("Render Plan ID: {}", plan.render_plan_id),
        format!("Profile ID: {}", plan.profile_id),
        String::new(),
        "Included Providers:".to_owned(),
    ];
    push_list(&mut lines, &plan.included_provider_ids);
    lines.extend([String::new(), "Redacted Providers:".to_owned()]);
    push_list(&mut lines, &plan.redacted_provider_ids);
    lines.extend([String::new(), "Visible Context References:".to_owned()]);
    push_list(&mut lines, &plan.visible_context_refs);
    lines.extend([String::new(), "Inline Sections:".to_owned()]);
    push_list(&mut lines, &plan.inline_sections);
    lines.extend([String::new(), "Omitted Providers:".to_owned()]);
    push_list(&mut lines, &plan.omitted_provider_ids);
    lines.push(String::new());
    lines.join("\n")
}

fn render_manifest(plan: &RequestContextRenderPlan, text: &str) -> Value {
    let content_sha256 = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    json!({
        "schema_version": "1.0",
        "kind": "request_context_render_manifest",
        "render_plan_id": plan.render_plan_id,
        "profile_id": plan.profile_id,
        "visible_context_refs": plan.visible_context_refs,
        "redacted_context_refs": plan.operator_only_context_refs,
        "included_provider_ids": plan.included_provider_ids,
        "redacted_provider_ids": plan.redacted_provider_ids,
        "omitted_provider_ids": plan.omitted_provider_ids,
        "artifact_contract_source": plan.artifact_contract_source,
        "output_artifact_contract_ids": plan.output_artifact_contract_ids,
        "content_sha256": content_sha256,
    })
}

fn visible_context_refs(workspace_root: &Path, request: &StageRunRequest) -> Vec<String> {
    let mut refs = Vec::new();
    if let (Some(family_id), Some(work_item_id)) = (
        request.active_work_item_family_id.as_deref(),
        request.active_work_item_id.as_deref(),
    ) {
        refs.push(format!("{family_id}:{work_item_id}"));
    } else if let (Some(kind), Some(work_item_id)) = (
        request.active_work_item_kind,
        request.active_work_item_id.as_deref(),
    ) {
        refs.push(format!("{}:{work_item_id}", kind.as_str()));
    }
    if let Some(path) = request.active_work_item_path.as_deref() {
        refs.push(format!("active_work_item_path:{path}"));
    }
    refs.extend(blueprint_context_refs(workspace_root, request));
    if let Some(root_spec_id) = request.closure_target_root_spec_id.as_deref() {
        refs.push(format!("closure_target:{root_spec_id}"));
    }
    if let Some(path) = request.skill_revision_evidence_path.as_deref() {
        refs.push(format!("skill_revision_evidence:{path}"));
    }
    refs
}

fn blueprint_context_refs(workspace_root: &Path, request: &StageRunRequest) -> Vec<String> {
    if request.active_work_item_family_id.as_deref() != Some("blueprint_draft") {
        return Vec::new();
    }
    let Some(active_path) = request.active_work_item_path.as_deref() else {
        return Vec::new();
    };
    let draft_path = resolve_workspace_path(workspace_root, active_path);
    let Ok(raw) = fs::read_to_string(&draft_path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let Some(manifest_id) = value.get("manifest_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let manifests_dir = workspace_root.join("millrace-agents/blueprints/manifests");
    let Some(manifest_path) = resolve_blueprint_manifest_context_path(&manifests_dir, manifest_id)
    else {
        return Vec::new();
    };
    vec![format!(
        "blueprint_manifest:{}",
        manifest_path
            .strip_prefix(workspace_root)
            .unwrap_or(&manifest_path)
            .to_string_lossy()
            .replace('\\', "/")
    )]
}

fn resolve_blueprint_manifest_context_path(
    manifests_dir: &Path,
    manifest_id: &str,
) -> Option<PathBuf> {
    let canonical = manifests_dir.join(format!("{manifest_id}.json"));
    if manifest_file_embeds_id(&canonical, manifest_id) {
        return Some(canonical);
    }
    let mut matches = Vec::new();
    let entries = fs::read_dir(manifests_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && manifest_file_embeds_id(&path, manifest_id)
        {
            matches.push(path);
        }
    }
    matches.sort();
    matches.into_iter().next()
}

fn manifest_file_embeds_id(path: &Path, manifest_id: &str) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value.get("manifest_id").and_then(Value::as_str) == Some(manifest_id)
}

fn preferred_output_refs(
    request: &StageRunRequest,
    compiled_plan: Option<&CompiledRunPlan>,
) -> Vec<String> {
    let Some(plan) = compiled_plan else {
        return Vec::new();
    };
    let Some(graph) = plan.graphs_by_plane.get(&request.plane) else {
        return Vec::new();
    };
    let Some(node) = graph
        .nodes
        .iter()
        .find(|node| node.node_id == request.node_id)
    else {
        return Vec::new();
    };
    node.declared_output_artifacts
        .iter()
        .map(|artifact| format!("declared_output:{artifact}"))
        .collect()
}

fn included_provider_ids(request: &StageRunRequest) -> Vec<String> {
    let mut providers = Vec::new();
    if request.active_work_item_id.is_some() {
        providers.push("active_work_item".to_owned());
    }
    if request.closure_target_root_spec_id.is_some() {
        providers.push("closure_target".to_owned());
    }
    providers.push("runtime_stage_request".to_owned());
    providers
}

fn inline_sections(request: &StageRunRequest) -> Vec<String> {
    let mut sections = Vec::new();
    if request.active_work_item_id.is_some() {
        sections.push("active_work_item".to_owned());
    }
    if request.closure_target_root_spec_id.is_some() {
        sections.push("closure_target".to_owned());
    }
    sections
}

fn unique_preserve_order(values: &mut Vec<String>) {
    let mut unique = Vec::new();
    for value in values.drain(..) {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    *values = unique;
}

fn push_list(lines: &mut Vec<String>, values: &[String]) {
    if values.is_empty() {
        lines.push("- none".to_owned());
    } else {
        lines.extend(values.iter().map(|value| format!("- {value}")));
    }
}

fn resolve_workspace_path(workspace_root: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    }
}

fn atomic_write_text(path: &Path, contents: &str) -> Result<(), StageRunRequestError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| StageRunRequestError::InvalidDocument {
            message: format!("failed to create {}: {error}", parent.display()),
        })?;
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, contents).map_err(|error| StageRunRequestError::InvalidDocument {
        message: format!("failed to write {}: {error}", temp.display()),
    })?;
    fs::rename(&temp, path).map_err(|error| StageRunRequestError::InvalidDocument {
        message: format!("failed to replace {}: {error}", path.display()),
    })
}

fn require_non_blank(field_name: &'static str, value: &str) -> Result<(), StageRunRequestError> {
    if value.trim().is_empty() {
        Err(StageRunRequestError::InvalidField {
            field_name,
            message: "must not be blank".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn json_error(error: serde_json::Error) -> StageRunRequestError {
    StageRunRequestError::Json {
        message: error.to_string(),
    }
}
