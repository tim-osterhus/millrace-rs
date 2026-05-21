mod support;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process,
};

use millrace_ai::contracts::{
    ActiveRunRequestKind, ActiveRunState, ApprovalPolicyRef, CapabilityDecisionState,
    CapabilityEnforcementMode, CapabilityEvidenceStatus, CapabilityScope, ClosureTargetState,
    ExecutionCapabilityGrant, Plane, RuntimeMode, StageName, Timestamp, WorkItemKind,
};
use millrace_ai::workspace::{
    BaselineManifestEntry, QueueStore, RuntimeOwnershipLockOptions,
    acquire_runtime_ownership_lock_with_options, build_baseline_manifest, load_baseline_manifest,
    load_snapshot, save_closure_target_state, save_snapshot, workspace_paths,
    write_baseline_manifest,
};
use millrace_ai::{
    ExecutionCapabilityApprovalRequest, ensure_execution_capability_approval,
    list_execution_capability_approvals,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::parity::{
    CommandOutput, ParityWorkspace, fixture_path, parse_version_line, read_fixture,
    run_python_reference_cli, run_python_reference_version_probe, run_rust_millrace,
    run_rust_millrace_with_env,
};
use tempfile::TempDir;

fn assert_exit_code(output: &CommandOutput, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected exit status\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn run_init_for(root: &Path) {
    run_rust_millrace(["init", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace init")
        .assert_success();
}

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn active_lock_options(session_id: &str) -> RuntimeOwnershipLockOptions {
    RuntimeOwnershipLockOptions::new(
        process::id(),
        "test-host",
        session_id,
        "2026-04-15T00:00:00Z",
    )
    .unwrap()
}

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse("test timestamp", value).unwrap()
}

fn approval_grant() -> ExecutionCapabilityGrant {
    ExecutionCapabilityGrant {
        grant_id: "grant-package-install".to_owned(),
        request_id: "request-package-install".to_owned(),
        capability_id: "package.install".to_owned(),
        access: "execute".to_owned(),
        scope: CapabilityScope {
            kind: "package_manager".to_owned(),
            value: "cargo".to_owned(),
            metadata: Default::default(),
        },
        required: true,
        decision_state: CapabilityDecisionState::ApprovalRequired,
        enforcement_mode: CapabilityEnforcementMode::NotApplicable,
        approval_policy_ref: Some(ApprovalPolicyRef {
            policy_id: "operator.package_install".to_owned(),
            gate_scope: "stage".to_owned(),
            expiration_seconds: None,
            required_decision: "approved".to_owned(),
        }),
        evidence_requirements: Vec::new(),
        evidence_status: CapabilityEvidenceStatus::NotRequired,
        decision_reason: "package installs require operator approval".to_owned(),
        resolved_by: "test".to_owned(),
        fingerprint: String::new(),
    }
}

fn create_pending_approval(paths: &millrace_ai::WorkspacePaths, request_id: &str) -> String {
    let grant = approval_grant();
    ensure_execution_capability_approval(
        paths,
        ExecutionCapabilityApprovalRequest {
            request_id,
            run_id: "run-approval",
            plane: Plane::Execution,
            node_id: "builder",
            stage_kind_id: "builder",
            work_item_kind: Some(WorkItemKind::Task),
            work_item_id: Some("task-approval"),
            grant: &grant,
            now: &timestamp("2026-04-15T00:00:00Z"),
            requested_by: "runtime",
        },
    )
    .unwrap()
    .approval_id
}

fn closure_target_state(root_spec_id: &str, root_idea_id: &str) -> ClosureTargetState {
    ClosureTargetState {
        schema_version: "1.0".to_owned(),
        kind: "closure_target_state".to_owned(),
        root_spec_id: root_spec_id.to_owned(),
        root_idea_id: root_idea_id.to_owned(),
        root_intake_kind: None,
        root_intake_id: None,
        root_spec_path: format!("millrace-agents/arbiter/contracts/root-specs/{root_spec_id}.md"),
        root_idea_path: format!("millrace-agents/arbiter/contracts/ideas/{root_idea_id}.md"),
        rubric_path: format!("millrace-agents/arbiter/rubrics/{root_spec_id}.md"),
        latest_verdict_path: Some(format!(
            "millrace-agents/arbiter/verdicts/{root_spec_id}.json"
        )),
        latest_report_path: Some("millrace-agents/arbiter/reports/run-001.md".to_owned()),
        closure_open: true,
        closure_blocked_by_lineage_work: false,
        blocking_work_ids: Vec::new(),
        blocking_work_refs: Vec::new(),
        opened_at: timestamp("2026-04-15T00:00:00Z"),
        closed_at: None,
        last_arbiter_run_id: Some("run-001".to_owned()),
    }
}

fn lineage_task_markdown(task_id: &str, root_spec_id: &str) -> String {
    read_fixture("work_documents/task.md")
        .unwrap()
        .replace("task-fixture", task_id)
        .replace("spec-root-001", root_spec_id)
}

fn lineage_spec_markdown(spec_id: &str, root_spec_id: &str) -> String {
    read_fixture("work_documents/spec.md")
        .unwrap()
        .replace("spec-fixture", spec_id)
        .replace("spec-root-001", root_spec_id)
}

fn lineage_incident_markdown(incident_id: &str, root_spec_id: &str) -> String {
    read_fixture("work_documents/incident.md")
        .unwrap()
        .replace("inc-fixture", incident_id)
        .replace(
            "Root-Spec-ID: spec-root-001",
            &format!("Root-Spec-ID: {root_spec_id}"),
        )
}

fn runnable_task_markdown(task_id: &str) -> String {
    format!(
        "# Bounded daemon task\n\n\
         Task-ID: {task_id}\n\
         Title: Bounded daemon task\n\
         Summary: Exercise bounded daemon CLI\n\
         Root-Idea-ID: idea-001\n\
         Root-Spec-ID: spec-root-001\n\
         Spec-ID: spec-root-001\n\
         Created-At: 2026-04-15T00:00:00Z\n\
         Created-By: tests\n\n\
         Tags:\n\
         - cli\n\n\
         Target-Paths:\n\
         - src/cli/mod.rs\n\n\
         Acceptance:\n\
         - millrace run daemon --max-ticks 1 executes one fake runner stage\n\n\
         Required-Checks:\n\
         - cargo test --test parity_cli\n\n\
         References:\n\
         - tests/parity_cli.rs\n\n\
         Risk:\n\
         - CLI wiring drifts\n"
    )
}

fn write_mock_codex_runtime_config(paths: &millrace_ai::workspace::WorkspacePaths, root: &Path) {
    let script_path = root.join("mock-codex.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\n\
         output_last_message=\"\"\n\
         while [ \"$#\" -gt 0 ]; do\n\
           if [ \"$1\" = \"--output-last-message\" ]; then\n\
             shift\n\
             output_last_message=\"$1\"\n\
           fi\n\
           shift || true\n\
         done\n\
         if [ -z \"$output_last_message\" ]; then\n\
           echo 'missing --output-last-message' >&2\n\
           exit 2\n\
         fi\n\
         printf '### BUILDER_COMPLETE\\n' > \"$output_last_message\"\n\
         printf '{\"type\":\"thread.started\",\"thread_id\":\"mock-thread\"}\\n'\n",
    )
    .unwrap();
    let script_arg = serde_json::to_string(&script_path.display().to_string()).unwrap();
    fs::write(
        &paths.runtime_config_file,
        format!("[runners.codex]\ncommand = \"sh\"\nargs = [{script_arg}]\n"),
    )
    .unwrap();
}

fn stdout_line_value<'a>(stdout: &'a str, prefix: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing stdout line prefix {prefix:?}\nstdout:\n{stdout}"))
}

fn lineage_repair_report_path(stdout: &str) -> PathBuf {
    PathBuf::from(stdout_line_value(stdout, "repair_report: "))
}

fn assert_lineage_report_applied(path: &Path, applied: bool) -> Value {
    let report: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("read lineage repair report"))
            .expect("parse lineage repair report");
    assert_eq!(report["kind"], "closure_lineage_repair_plan");
    assert_eq!(report["applied"], applied);
    report
}

fn read_json_lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read JSONL {}: {error}", path.display()))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("parse JSONL entry"))
        .collect()
}

fn read_mailbox_json_values(paths: &millrace_ai::workspace::WorkspacePaths) -> Vec<Value> {
    let mut entries: Vec<_> = fs::read_dir(&paths.mailbox_incoming_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    entries.sort();
    entries
        .iter()
        .map(|path| {
            serde_json::from_str(
                &fs::read_to_string(path).unwrap_or_else(|error| {
                    panic!("read mailbox command {}: {error}", path.display())
                }),
            )
            .expect("parse mailbox command")
        })
        .collect()
}

fn archive_contains(directory: &Path, prefix: &str) -> bool {
    fs::read_dir(directory)
        .map(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".md"))
            })
        })
        .unwrap_or(false)
}

fn write_blocked_task(paths: &millrace_ai::workspace::WorkspacePaths, task_id: &str) {
    fs::write(
        paths.tasks_blocked_dir.join(format!("{task_id}.md")),
        lineage_task_markdown(task_id, "spec-root-001"),
    )
    .unwrap();
}

fn write_blocked_task_metadata(
    paths: &millrace_ai::workspace::WorkspacePaths,
    task_id: &str,
    failure_class: &str,
    auto_requeue_candidate: bool,
) {
    let destination = paths
        .runtime_root
        .join("diagnostics")
        .join("blocked")
        .join(format!("task-{task_id}.json"));
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(
        destination,
        serde_json::to_string_pretty(&json!({
            "work_item_kind": "task",
            "work_item_id": task_id,
            "root_spec_id": "spec-root-001",
            "root_idea_id": "idea-001",
            "blocked_at": "2026-04-15T00:00:00Z",
            "blocked_origin": "runner_failure",
            "failure_class": failure_class,
            "failure_scope": "environment",
            "auto_requeue_candidate": auto_requeue_candidate,
            "failure_classifier_code": "network_unavailable",
            "source_run_id": "run-001",
            "source_plane": "execution",
            "source_stage": "builder",
            "terminal_result": "BLOCKED",
            "stage_result_path": "millrace-agents/runs/run-001/stage_result.json",
            "stdout_path": "millrace-agents/runs/run-001/stdout.txt",
            "stderr_path": "millrace-agents/runs/run-001/stderr.txt"
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();
}

fn mailbox_json_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
}

fn runtime_tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let runtime_root = root.join("millrace-agents");
    let mut files = BTreeMap::new();
    if runtime_root.exists() {
        collect_file_snapshot(&runtime_root, &runtime_root, &mut files);
    }
    files
}

fn collect_file_snapshot(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_file_snapshot(root, &path, files);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, fs::read(path).unwrap());
        }
    }
}

fn rust_test_functions_by_file(test_files: &[&str]) -> BTreeMap<String, BTreeSet<String>> {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut functions = BTreeMap::new();
    for test_file in test_files {
        let contents =
            fs::read_to_string(repo_root.join(test_file)).expect("read Rust test source file");
        let mut names = BTreeSet::new();
        let mut pending_test_attr = false;
        for line in contents.lines() {
            let line = line.trim_start();
            if line.starts_with("#[test]") || line.starts_with("#[tokio::test") {
                pending_test_attr = true;
                continue;
            }
            if pending_test_attr && (line.is_empty() || line.starts_with("#[")) {
                continue;
            }
            if pending_test_attr {
                let raw_name = line
                    .strip_prefix("fn ")
                    .or_else(|| line.strip_prefix("async fn "))
                    .and_then(|rest| rest.split_once('(').map(|(name, _)| name.trim()));
                if let Some(name) = raw_name {
                    names.insert(name.to_owned());
                }
                pending_test_attr = false;
            }
        }
        functions.insert((*test_file).to_owned(), names);
    }
    functions
}

fn is_snake_case_rust_test_name(value: &str) -> bool {
    value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !value.ends_with('_')
        && !value.contains("__")
}

fn assert_repo_relative_target_exists(target_path: &str, context: &str) {
    assert!(
        !target_path.is_empty()
            && !target_path.contains('\\')
            && !Path::new(target_path).is_absolute()
            && !target_path.split('/').any(|segment| segment == ".."),
        "{context} references invalid Rust target path {target_path}"
    );
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(target_path)
            .exists(),
        "{context} references missing Rust target path {target_path}"
    );
}

fn is_evidence_only_target_kind(kind: &str) -> bool {
    matches!(
        kind,
        "package_evidence" | "unsupported_gap_evidence" | "reference_evidence"
    )
}

fn changed_paths_from_generated_scout(generated_scout: &str) -> BTreeSet<String> {
    let mut changed_paths = BTreeSet::new();
    let mut in_changed_paths = false;
    for line in generated_scout.lines() {
        if line == "Changed Python paths:" {
            in_changed_paths = true;
            continue;
        }
        if !in_changed_paths {
            continue;
        }
        if line == "Diff stat:" {
            break;
        }
        if let Some(path) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            changed_paths.insert(path.to_owned());
        }
    }
    changed_paths
}

fn live_python_diff_changed_paths(diff_range: &str) -> BTreeSet<String> {
    let output = process::Command::new("git")
        .args(["-C", "../millrace-py", "diff", "--name-only", diff_range])
        .output()
        .expect("run live Python changed-path diff");
    assert!(
        output.status.success(),
        "live Python changed-path diff failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Python changed-path diff is UTF-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn rust_version_command_has_millrace_shape() {
    let output = run_rust_millrace(["--version"]).expect("run Rust millrace --version");

    output.assert_success();

    let version_line =
        parse_version_line(output.stdout_trimmed()).expect("parse Rust version line");
    assert_eq!(version_line.binary_name, "millrace");
    assert_eq!(version_line.version, "0.5.0");
    assert_eq!(version_line.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn rust_version_subcommand_matches_version_flag() {
    let flag = run_rust_millrace(["--version"]).expect("run Rust millrace --version");
    let subcommand = run_rust_millrace(["version"]).expect("run Rust millrace version");

    flag.assert_success();
    subcommand.assert_success();
    assert_eq!(flag.stdout_trimmed(), subcommand.stdout_trimmed());
}

#[test]
fn rust_status_and_about_outputs_are_stable() {
    let implicit = run_rust_millrace(std::iter::empty::<&str>()).expect("run Rust millrace");
    let status = run_rust_millrace(["--status"]).expect("run Rust millrace --status");
    let about = run_rust_millrace(["about"]).expect("run Rust millrace about");

    implicit.assert_success();
    status.assert_success();
    about.assert_success();

    let expected = format!(
        "Millrace Rust runtime {}\n\
         package: millrace-ai\n\
         crate: millrace_ai\n\
         binary: millrace\n\
         status: experimental\n\
         production runtime: Python package millrace-ai\n",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(implicit.stdout, expected);
    assert_eq!(status.stdout, expected);
    assert_eq!(about.stdout, expected);
    assert_eq!(implicit.stderr, "");
    assert_eq!(status.stderr, "");
    assert_eq!(about.stderr, "");
}

#[test]
fn committed_slice4_cli_parity_evidence_covers_required_axes() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/slice4_cli_parity_evidence.json")
            .expect("read CLI parity evidence fixture"),
    )
    .expect("parse CLI parity evidence fixture");
    assert_eq!(fixture["kind"], "slice4_cli_parity_evidence");

    let axes = [
        "exit_codes",
        "key_output_lines",
        "read_only_guarantees",
        "file_mutations",
        "mailbox_artifacts",
        "initialized_workspace_refusal",
        "parse_failures",
    ];
    let coverage = fixture["coverage"]
        .as_array()
        .expect("coverage entries are present");
    let mut by_surface = BTreeMap::new();
    for entry in coverage {
        let surface = entry["surface"].as_str().expect("surface name");
        for axis in axes {
            assert!(
                entry.get(axis).is_some(),
                "missing evidence axis {axis} for {surface}"
            );
        }
        by_surface.insert(surface, entry);
    }

    for surface in [
        "version-about-init-doctor",
        "compile",
        "read-only-operator",
        "queue-intake",
        "control-planning-config",
        "skills",
        "upgrade",
        "queue-repair-lineage",
        "run-placeholders",
    ] {
        assert!(
            by_surface.contains_key(surface),
            "missing CLI parity evidence for {surface}"
        );
    }
    for axis in axes {
        assert!(
            coverage
                .iter()
                .any(|entry| entry[axis].as_bool() == Some(true)),
            "no CLI parity evidence entry covers {axis}"
        );
    }

    let non_goals = fixture["non_goals"]
        .as_array()
        .expect("non-goal list is present");
    for non_goal in [
        "runtime ticks",
        "daemon scheduling",
        "runner dispatch",
        "mailbox consumption",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing non-goal {non_goal}"
        );
    }
}

#[test]
fn committed_slice5_serial_runtime_parity_evidence_covers_python_sources_and_axes() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/slice5_serial_runtime_parity_evidence.json")
            .expect("read Slice 5 serial runtime parity evidence fixture"),
    )
    .expect("parse Slice 5 serial runtime parity evidence fixture");
    assert_eq!(fixture["kind"], "slice5_serial_runtime_parity_evidence");

    let scenarios = fixture["scenarios"]
        .as_array()
        .expect("scenario entries are present");
    assert!(!scenarios.is_empty());

    for source_path in [
        "../millrace-py/tests/runtime/test_runtime.py",
        "../millrace-py/tests/runtime/test_result_application.py",
        "../millrace-py/tests/runtime/test_router.py",
        "../millrace-py/tests/integration/test_e2e_handoffs.py",
    ] {
        assert!(
            scenarios.iter().any(|scenario| {
                scenario["python_sources"]
                    .as_array()
                    .expect("python_sources array")
                    .iter()
                    .any(|source| source["path"].as_str() == Some(source_path))
            }),
            "missing Slice 5 parity evidence source {source_path}"
        );
    }

    let normalized_fields = fixture["normalized_fields"]
        .as_array()
        .expect("normalized field list is present");
    for field in [
        "request_id",
        "run_id",
        "timestamp",
        "absolute_workspace_path",
        "incident_id",
    ] {
        assert!(
            normalized_fields
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing normalized volatile field {field}"
        );
    }

    let mut covered_axes = BTreeSet::new();
    for scenario in scenarios {
        assert!(
            !scenario["rust_tests"]
                .as_array()
                .expect("rust_tests array")
                .is_empty(),
            "scenario is missing Rust coverage: {scenario:?}"
        );
        assert!(
            !scenario["python_sources"]
                .as_array()
                .expect("python_sources array")
                .is_empty(),
            "scenario is missing Python source coverage: {scenario:?}"
        );
        for axis in scenario["coverage"].as_array().expect("coverage array") {
            covered_axes.insert(axis.as_str().expect("coverage axis").to_owned());
        }
    }

    for axis in [
        "startup",
        "queue_transitions",
        "snapshot_status_persistence",
        "stage_request_fields",
        "result_envelopes",
        "lane_scoped_result_application",
        "recovery_counters",
        "closure_activation",
        "lock_contention",
        "no_work_outcomes",
        "cli_run_once",
        "no_real_runner_invocation",
    ] {
        assert!(
            covered_axes.contains(axis),
            "missing Slice 5 parity evidence coverage axis {axis}"
        );
    }

    let runner_guarantees = fixture["runner_guarantees"]
        .as_array()
        .expect("runner guarantee list is present");
    for guarantee in [
        "fake_runner_only",
        "no_codex_cli_process",
        "no_pi_rpc_transport",
    ] {
        assert!(
            runner_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing runner guarantee {guarantee}"
        );
    }

    let non_goals = fixture["non_goals"]
        .as_array()
        .expect("non-goal list is present");
    for non_goal in [
        "daemon scheduling",
        "real Codex runner adapters",
        "real Pi runner adapters",
        "usage governance",
        "monitor streaming",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing non-goal {non_goal}"
        );
    }
}

#[test]
fn committed_slice6_daemon_runtime_parity_evidence_covers_python_sources_and_axes() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/slice6_daemon_runtime_parity_evidence.json")
            .expect("read Slice 6 daemon runtime parity evidence fixture"),
    )
    .expect("parse Slice 6 daemon runtime parity evidence fixture");
    assert_eq!(fixture["kind"], "slice6_daemon_runtime_parity_evidence");

    let scenarios = fixture["scenarios"]
        .as_array()
        .expect("scenario entries are present");
    assert!(!scenarios.is_empty());

    for source_path in [
        "../millrace-py/src/millrace_ai/cli/commands/run.py",
        "../millrace-py/src/millrace_ai/cli/monitoring.py",
        "../millrace-py/src/millrace_ai/runtime/supervisor.py",
        "../millrace-py/src/millrace_ai/runtime/mailbox_intake.py",
        "../millrace-py/src/millrace_ai/runtime/watcher_intake.py",
        "../millrace-py/tests/runtime/test_supervisor.py",
        "../millrace-py/tests/runtime/test_watchers.py",
        "../millrace-py/tests/cli/test_cli.py",
        "../millrace-py/tests/cli/test_monitoring.py",
    ] {
        assert!(
            scenarios.iter().any(|scenario| {
                scenario["python_sources"]
                    .as_array()
                    .expect("python_sources array")
                    .iter()
                    .any(|source| source["path"].as_str() == Some(source_path))
            }),
            "missing Slice 6 daemon parity evidence source {source_path}"
        );
    }

    let normalized_fields = fixture["normalized_fields"]
        .as_array()
        .expect("normalized field list is present");
    for field in [
        "timestamp",
        "run_id",
        "request_id",
        "absolute_workspace_path",
        "process_id",
        "generated_command_id",
        "compact_run_handle",
        "compiled_plan_id",
    ] {
        assert!(
            normalized_fields
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing normalized volatile field {field}"
        );
    }

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axis list is present");
    let mut covered_axes = BTreeSet::new();
    for scenario in scenarios {
        assert!(
            !scenario["rust_tests"]
                .as_array()
                .expect("rust_tests array")
                .is_empty(),
            "scenario is missing Rust coverage: {scenario:?}"
        );
        assert!(
            !scenario["python_sources"]
                .as_array()
                .expect("python_sources array")
                .is_empty(),
            "scenario is missing Python source coverage: {scenario:?}"
        );
        for axis in scenario["coverage"].as_array().expect("coverage array") {
            covered_axes.insert(axis.as_str().expect("coverage axis").to_owned());
        }
    }

    for axis in required_axes {
        let axis = axis.as_str().expect("required coverage axis");
        assert!(
            covered_axes.contains(axis),
            "missing Slice 6 daemon parity evidence coverage axis {axis}"
        );
    }

    let runner_guarantees = fixture["runner_guarantees"]
        .as_array()
        .expect("runner guarantee list is present");
    for guarantee in [
        "fake_runner_only",
        "no_codex_cli_process",
        "no_pi_rpc_transport",
        "no_real_stage_agent_execution",
    ] {
        assert!(
            runner_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing runner guarantee {guarantee}"
        );
    }

    let non_goals = fixture["non_goals"]
        .as_array()
        .expect("non-goal list is present");
    for non_goal in [
        "real Codex runner adapters",
        "real Pi runner adapters",
        "subprocess supervision",
        "advanced usage governance",
        "learning promotion",
        "Slice 8 advanced parity surfaces",
        "Rust self-hosting",
        "real stage-agent execution in daemon tests",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing non-goal {non_goal}"
        );
    }
}

#[test]
fn committed_slice7_runner_adapter_parity_evidence_covers_python_sources_contracts_and_axes() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/slice7_runner_adapter_parity_evidence.json")
            .expect("read Slice 7 runner adapter parity evidence fixture"),
    )
    .expect("parse Slice 7 runner adapter parity evidence fixture");
    assert_eq!(fixture["kind"], "slice7_runner_adapter_parity_evidence");

    let scenarios = fixture["scenarios"]
        .as_array()
        .expect("scenario entries are present");
    assert!(!scenarios.is_empty());

    let mut referenced_paths = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axis list is present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("required coverage axis"))
        .collect();
    let allowed_rust_tests = BTreeSet::from([
        "codex_adapter_maps_failure_modes_to_inspectable_raw_results_and_completion_notes",
        "codex_adapter_preserves_empty_final_text_for_normalization_failure",
        "codex_adapter_reconciles_timeout_after_final_terminal_marker",
        "codex_adapter_resolves_permission_precedence_and_mapping",
        "codex_adapter_uses_one_hour_fallback_timeout_when_request_timeout_is_zero",
        "codex_adapter_writes_prompt_invocation_completion_events_stdout_and_tokens",
        "codex_command_preserves_python_flag_order_and_environment_delta",
        "codex_live_smoke_gate_skips_without_env",
        "codex_real_adapter_live_smoke",
        "pi_adapter_maps_provider_empty_text_invalid_json_binary_and_timeout_failures",
        "pi_adapter_writes_prompt_artifacts_stdout_tokens_and_default_command",
        "pi_command_preserves_provider_model_thinking_defaults_env_and_rejects_reserved_args",
        "pi_event_log_policy_filters_message_updates_for_success_and_failure",
        "pi_jsonl_client_detects_provider_failure_invalid_json_timeout_abort_and_hard_kill",
        "pi_jsonl_client_runs_prompt_queries_final_text_and_session_stats",
        "pi_live_smoke_gate_skips_without_env",
        "pi_normalization_preserves_success_and_token_usage",
        "pi_real_adapter_live_smoke",
        "public_contract_exports_remain_importable",
        "runner_artifact_contracts_capture_invocation_completion_and_process_evidence",
        "runner_registry_and_dispatcher_resolve_in_python_compatible_order",
        "runner_registry_reports_duplicate_and_unknown_adapter_names",
        "runtime_config_loading_exposes_real_runner_adapter_settings",
        "runtime_config_loading_rejects_real_runner_config_failures_with_paths",
        "runtime_configured_dispatcher_registers_real_adapters_and_fake_test_adapter",
        "rust_config_validate_compiles_selected_or_explicit_config_modes",
        "rust_run_daemon_bounded_execution_uses_fake_runner_and_run_views",
        "rust_run_daemon_default_stdout_is_quiet_except_summary_lines",
        "rust_run_daemon_max_ticks_one_executes_one_fake_runner_tick_and_run_views_inspect_artifacts",
        "rust_status_config_and_modes_read_only_commands_render_without_mutation",
        "serial_tick_can_dispatch_through_registry_dispatcher_without_real_runner",
        "serial_tick_normalizes_dispatcher_unknown_runner_through_recovery_path",
    ]);
    for scenario in scenarios {
        let rust_tests = scenario["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "scenario is missing Rust coverage: {scenario:?}"
        );
        for rust_test in rust_tests {
            let rust_test = rust_test.as_str().expect("rust test name");
            assert!(
                allowed_rust_tests.contains(rust_test),
                "Slice 7 fixture references unknown Rust test {rust_test}"
            );
        }
        let python_sources = scenario["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "scenario is missing Python source coverage: {scenario:?}"
        );
        for source in python_sources {
            referenced_paths.insert(
                source["path"]
                    .as_str()
                    .expect("python source path")
                    .to_owned(),
            );
        }
        for axis in scenario["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "scenario references unknown Slice 7 coverage axis {axis}"
            );
            covered_axes.insert(axis.to_owned());
        }
    }

    for source_path in [
        "../millrace-py/docs/runtime/millrace-runner-architecture.md",
        "../millrace-py/src/millrace_ai/runners/requests.py",
        "../millrace-py/src/millrace_ai/runners/registry.py",
        "../millrace-py/src/millrace_ai/runners/dispatcher.py",
        "../millrace-py/src/millrace_ai/runners/process.py",
        "../millrace-py/src/millrace_ai/runners/contracts.py",
        "../millrace-py/src/millrace_ai/runners/adapters/_prompting.py",
        "../millrace-py/src/millrace_ai/runners/adapters/codex_cli.py",
        "../millrace-py/src/millrace_ai/runners/adapters/codex_cli_command.py",
        "../millrace-py/src/millrace_ai/runners/adapters/codex_cli_artifacts.py",
        "../millrace-py/src/millrace_ai/runners/adapters/codex_cli_tokens.py",
        "../millrace-py/src/millrace_ai/runners/adapters/pi_rpc.py",
        "../millrace-py/src/millrace_ai/runners/adapters/pi_rpc_client.py",
        "../millrace-py/src/millrace_ai/config/models.py",
        "../millrace-py/tests/runners/test_runner.py",
        "../millrace-py/tests/runners/test_runners_registry.py",
        "../millrace-py/tests/runners/test_runners_codex_adapter.py",
        "../millrace-py/tests/runners/test_runners_pi_rpc_adapter.py",
        "../millrace-py/tests/runtime/test_runtime.py",
        "../millrace-py/tests/cli/test_cli.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing Slice 7 parity evidence source {source_path}"
        );
    }

    for axis in required_axes {
        let axis = axis.as_str().expect("required coverage axis");
        assert!(
            covered_axes.contains(axis),
            "missing Slice 7 parity evidence coverage axis {axis}"
        );
    }

    let normalized_fields = fixture["normalized_fields"]
        .as_array()
        .expect("normalized field list is present");
    for field in [
        "request_id",
        "run_id",
        "timestamp",
        "absolute_workspace_path",
        "runner_prompt_path",
        "runner_invocation_path",
        "runner_stdout_path",
        "runner_stderr_path",
        "runner_events_path",
        "runner_completion_path",
        "compiled_plan_id",
        "live_smoke_gate_variable",
        "process_id",
        "token_usage",
    ] {
        assert!(
            normalized_fields
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing normalized volatile field {field}"
        );
    }

    assert_eq!(
        fixture["preserved_contracts"]["runner_request_result_contract"],
        "StageRunRequest -> RunnerRawResult"
    );
    assert_eq!(
        fixture["preserved_contracts"]["normalization_contract"],
        "RunnerRawResult -> StageResultEnvelope"
    );
    let artifact_filenames = fixture["preserved_contracts"]["artifact_filenames"]
        .as_array()
        .expect("artifact filenames are present");
    for artifact in [
        "runner_prompt.<request_id>.md",
        "runner_invocation.<request_id>.json",
        "runner_stdout.<request_id>.txt",
        "runner_stderr.<request_id>.txt",
        "runner_events.<request_id>.jsonl",
        "runner_completion.<request_id>.json",
    ] {
        assert!(
            artifact_filenames
                .iter()
                .any(|value| value.as_str() == Some(artifact)),
            "missing preserved artifact filename {artifact}"
        );
    }

    let runner_guarantees = fixture["runner_guarantees"]
        .as_array()
        .expect("runner guarantee list is present");
    for guarantee in [
        "mocked_processes_for_always_on_tests",
        "no_live_codex_cli_process",
        "no_live_pi_rpc_process",
        "live_smoke_opt_in_only",
        "live_smoke_gates_checked_before_real_adapter_construction",
        "live_smoke_credentials_inherited_from_operator_environment_only",
    ] {
        assert!(
            runner_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing runner guarantee {guarantee}"
        );
    }

    let live_smoke = &fixture["live_smoke"];
    assert_eq!(
        live_smoke["normal_no_live_command"],
        "cargo test --test runners_live_smoke"
    );
    assert_eq!(
        live_smoke["codex"]["gate_variable"],
        "MILLRACE_REAL_CODEX_SMOKE"
    );
    assert_eq!(live_smoke["pi"]["gate_variable"], "MILLRACE_REAL_PI_SMOKE");
    assert_eq!(
        live_smoke["codex"]["opt_in_command"],
        "MILLRACE_REAL_CODEX_SMOKE=1 cargo test --test runners_live_smoke codex_real_adapter_live_smoke -- --ignored --nocapture"
    );
    assert_eq!(
        live_smoke["pi"]["opt_in_command"],
        "MILLRACE_REAL_PI_SMOKE=1 cargo test --test runners_live_smoke pi_real_adapter_live_smoke -- --ignored --nocapture"
    );
    assert!(
        live_smoke["outside_ci_reason"]
            .as_str()
            .expect("outside_ci_reason")
            .contains("external binaries")
    );

    let non_goals = fixture["non_goals"]
        .as_array()
        .expect("non-goal list is present");
    for non_goal in [
        "live Codex smoke tests in always-on cargo test",
        "live Pi smoke tests in always-on cargo test",
        "broader compiled-plan semantics changes",
        "queue-state or stage-machine changes beyond existing runtime dispatch",
        "native filesystem watcher integration",
        "advanced usage governance",
        "subscription quota telemetry",
        "learning promotion",
        "Slice 8 advanced parity surfaces",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing non-goal {non_goal}"
        );
    }
}

#[test]
fn committed_slice8_e2e_handoff_parity_evidence_covers_python_sources_and_axes() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/slice8_e2e_handoff_parity_evidence.json")
            .expect("read Slice 8 E2E handoff parity evidence fixture"),
    )
    .expect("parse Slice 8 E2E handoff parity evidence fixture");
    assert_eq!(fixture["kind"], "slice8_e2e_handoff_parity_evidence");

    let scenarios = fixture["scenarios"]
        .as_array()
        .expect("scenario entries are present");
    assert!(!scenarios.is_empty());

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axis list is present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("required coverage axis"))
        .collect();
    let allowed_rust_tests = BTreeSet::from([
        "e2e_direct_task_handoff_happy_path_uses_runtime_queue_and_status_transitions",
        "e2e_repair_loop_fix_needed_cycle_preserves_fix_evidence_and_finishes",
        "e2e_recovery_malformed_result_routes_to_consultant_with_incident_evidence",
        "e2e_recovery_illegal_terminal_result_routes_to_consultant_with_incident_evidence",
        "e2e_needs_planning_incident_intake_reenters_execution_preserving_lineage",
        "e2e_lineage_drain_triggers_arbiter_complete_and_closes_target",
        "e2e_lineage_drain_triggers_arbiter_remediation_gap_and_blocks_repeat",
    ]);
    let mut referenced_paths = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();

    for scenario in scenarios {
        let rust_tests = scenario["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "scenario is missing Rust coverage: {scenario:?}"
        );
        for rust_test in rust_tests {
            let rust_test = rust_test.as_str().expect("rust test name");
            assert!(
                allowed_rust_tests.contains(rust_test),
                "Slice 8 fixture references unknown Rust test {rust_test}"
            );
        }

        let python_sources = scenario["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "scenario is missing Python source coverage: {scenario:?}"
        );
        for source in python_sources {
            referenced_paths.insert(
                source["path"]
                    .as_str()
                    .expect("python source path")
                    .to_owned(),
            );
        }

        for axis in scenario["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "scenario references unknown Slice 8 coverage axis {axis}"
            );
            covered_axes.insert(axis.to_owned());
        }
    }

    for source_path in [
        "../millrace-py/tests/integration/test_e2e_handoffs.py",
        "../millrace-py/src/millrace_ai/runtime/handoff_incidents.py",
        "../millrace-py/src/millrace_ai/runtime/completion_behavior.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing Slice 8 E2E parity evidence source {source_path}"
        );
    }

    for axis in required_axes {
        let axis = axis.as_str().expect("required coverage axis");
        assert!(
            covered_axes.contains(axis),
            "missing Slice 8 E2E parity evidence coverage axis {axis}"
        );
    }

    let normalized_fields = fixture["normalized_fields"]
        .as_array()
        .expect("normalized field list is present");
    for field in [
        "request_id",
        "run_id",
        "timestamp",
        "absolute_workspace_path",
        "fix_contract_path",
        "stage_result_path",
        "runtime_error_report_path",
        "runtime_error_context_path",
        "incident_id",
        "related_stage_result_path",
        "arbiter_verdict_path",
        "arbiter_report_path",
        "compiled_plan_id",
        "process_id",
    ] {
        assert!(
            normalized_fields
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing normalized volatile field {field}"
        );
    }

    let runner_guarantees = fixture["runner_guarantees"]
        .as_array()
        .expect("runner guarantee list is present");
    for guarantee in [
        "scripted_fake_runner_only",
        "no_codex_cli_process",
        "no_pi_rpc_transport",
        "no_network_or_credentials",
        "runtime_owned_queue_mutation_only",
    ] {
        assert!(
            runner_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing runner guarantee {guarantee}"
        );
    }

    let non_goals = fixture["non_goals"]
        .as_array()
        .expect("non-goal list is present");
    for non_goal in [
        "live Codex runner execution",
        "live Pi runner execution",
        "network access",
        "credentialed subscription services",
        "new stage names or terminal results",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing non-goal {non_goal}"
        );
    }
}

#[test]
fn committed_slice8_advanced_parity_evidence_covers_all_surfaces_and_live_tests() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/slice8_advanced_parity_evidence.json")
            .expect("read Slice 8 advanced parity evidence fixture"),
    )
    .expect("parse Slice 8 advanced parity evidence fixture");
    assert_eq!(fixture["kind"], "slice8_advanced_parity_evidence");

    let areas = fixture["advanced_coverage_areas"]
        .as_array()
        .expect("advanced coverage areas are present");
    assert!(!areas.is_empty());

    let required_areas = fixture["required_advanced_coverage_areas"]
        .as_array()
        .expect("required advanced coverage areas are present");
    let required_area_names: BTreeSet<_> = required_areas
        .iter()
        .map(|area| area.as_str().expect("required area name"))
        .collect();
    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("required coverage axis"))
        .collect();

    let test_files = [
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/workspace_runtime_control.rs",
        "tests/parity_cli.rs",
    ];
    let available_tests = rust_test_functions_by_file(&test_files);
    let allowed_rust_refs = BTreeSet::from([
        "tests/runtime_serial.rs::usage_governance_is_inert_by_default_for_contract_evaluation",
        "tests/runtime_serial.rs::usage_governance_ledger_records_once_and_reconciles_stage_results",
        "tests/runtime_serial.rs::malformed_usage_governance_state_and_ledger_fail_with_paths",
        "tests/runtime_serial.rs::serial_runtime_governance_pauses_after_token_stage_and_auto_resumes",
        "tests/workspace_runtime_control.rs::direct_resume_preserves_governance_pause_when_blocker_is_active",
        "tests/runtime_daemon.rs::basic_monitor_renders_reload_watcher_governance_and_fanout_lines",
        "tests/runtime_daemon.rs::daemon_supervisor_governance_pause_after_completion_blocks_new_claims",
        "tests/runtime_serial.rs::usage_governance_evaluates_token_windows_and_subscription_quota_contracts",
        "tests/runtime_serial.rs::serial_runtime_subscription_quota_degraded_fail_open_and_fail_closed",
        "tests/runtime_daemon.rs::runtime_config_loading_exposes_real_runner_adapter_settings",
        "tests/runtime_daemon.rs::runtime_config_loading_rejects_real_runner_config_failures_with_paths",
        "tests/parity_cli.rs::rust_status_config_and_modes_read_only_commands_render_without_mutation",
        "tests/runtime_serial.rs::serial_tick_learning_trigger_enqueues_analyst_first_request",
        "tests/runtime_serial.rs::serial_tick_curator_promotion_defers_until_foreground_drain",
        "tests/runtime_serial.rs::serial_tick_curator_rejected_decision_keeps_evidence_without_promotion_or_source_mutation",
        "tests/runtime_serial.rs::serial_tick_curator_blocked_decision_keeps_evidence_without_promotion_or_source_mutation",
        "tests/parity_cli.rs::rust_skills_install_export_and_promote_file_backed_packages",
        "tests/runtime_serial.rs::serial_tick_activates_learning_request_only_when_learning_graph_exists",
        "tests/parity_cli.rs::rust_runs_read_only_commands_surface_advanced_inspection_evidence",
        "tests/runtime_serial.rs::serial_tick_opens_closure_target_when_root_spec_claim_activates",
        "tests/runtime_serial.rs::serial_tick_backfills_closure_target_from_done_root_spec",
        "tests/runtime_serial.rs::serial_tick_activates_closure_target_request_without_active_work_item",
        "tests/runtime_serial.rs::serial_tick_suppresses_closure_target_when_queued_lineage_work_remains",
        "tests/runtime_serial.rs::serial_tick_suppresses_closure_target_when_blocked_lineage_work_remains",
        "tests/runtime_serial.rs::serial_tick_reports_active_spec_and_blocked_incident_lineage_ids_before_arbiter",
        "tests/runtime_serial.rs::serial_tick_blocks_closure_target_on_lineage_drift_diagnostic",
        "tests/runtime_serial.rs::serial_tick_closes_closure_target_on_arbiter_complete",
        "tests/runtime_serial.rs::serial_tick_enqueues_remediation_incident_for_arbiter_gap",
        "tests/runtime_serial.rs::serial_tick_blocks_repeated_arbiter_remediation_without_execution",
        "tests/parity_cli.rs::rust_queue_repair_lineage_preview_writes_report_skips_unsafe_findings_and_does_not_mutate",
        "tests/parity_cli.rs::rust_queue_repair_lineage_apply_repairs_safe_documents_refreshes_snapshot_and_emits_event",
        "tests/parity_cli.rs::rust_runs_read_only_commands_inspect_and_tail_artifacts_without_mutation",
        "tests/runtime_serial.rs::e2e_direct_task_handoff_happy_path_uses_runtime_queue_and_status_transitions",
        "tests/runtime_serial.rs::e2e_repair_loop_fix_needed_cycle_preserves_fix_evidence_and_finishes",
        "tests/runtime_serial.rs::e2e_recovery_malformed_result_routes_to_consultant_with_incident_evidence",
        "tests/runtime_serial.rs::e2e_recovery_illegal_terminal_result_routes_to_consultant_with_incident_evidence",
        "tests/runtime_serial.rs::e2e_needs_planning_incident_intake_reenters_execution_preserving_lineage",
        "tests/runtime_serial.rs::e2e_lineage_drain_triggers_arbiter_complete_and_closes_target",
        "tests/runtime_serial.rs::e2e_lineage_drain_triggers_arbiter_remediation_gap_and_blocks_repeat",
    ]);

    let mut covered_areas = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();

    for area in areas {
        let area_name = area["area"].as_str().expect("advanced coverage area");
        assert!(
            required_area_names.contains(area_name),
            "Slice 8 advanced fixture references unknown coverage area {area_name}"
        );
        covered_areas.insert(area_name.to_owned());

        for axis in area["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "Slice 8 advanced fixture references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis.to_owned());
        }

        let python_sources = area["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "Slice 8 advanced fixture area {area_name} is missing Python sources"
        );
        for source in python_sources {
            referenced_paths.insert(
                source["path"]
                    .as_str()
                    .expect("python source path")
                    .to_owned(),
            );
        }

        let rust_tests = area["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "Slice 8 advanced fixture area {area_name} is missing Rust tests"
        );
        for rust_test in rust_tests {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                is_snake_case_rust_test_name(test_name),
                "Slice 8 advanced fixture has malformed Rust test name {test_name}"
            );
            assert!(
                available_tests.contains_key(test_file),
                "Slice 8 advanced fixture references unsupported Rust test file {test_file}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                allowed_rust_refs.contains(rust_ref.as_str()),
                "Slice 8 advanced fixture references unknown Rust test {rust_ref}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "Slice 8 advanced fixture references stale Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }

    for area in required_area_names {
        assert!(
            covered_areas.contains(area),
            "missing Slice 8 advanced parity evidence area {area}"
        );
    }
    for axis in required_axis_names {
        assert!(
            covered_axes.contains(axis),
            "missing Slice 8 advanced parity evidence axis {axis}"
        );
    }
    for rust_ref in &allowed_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required Slice 8 advanced Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/tests/runtime/test_usage_governance.py",
        "../millrace-py/src/millrace_ai/runtime/usage_governance/",
        "../millrace-py/src/millrace_ai/runtime/usage_governance/subscription_quota.py",
        "../millrace-py/tests/runtime/test_learning_promotions.py",
        "../millrace-py/src/millrace_ai/runtime/learning_promotions.py",
        "../millrace-py/src/millrace_ai/runtime/skill_evidence.py",
        "../millrace-py/tests/runtime/test_runtime.py",
        "../millrace-py/tests/runners/test_runner.py",
        "../millrace-py/tests/runtime/test_completion_behavior.py",
        "../millrace-py/src/millrace_ai/runtime/completion_behavior.py",
        "../millrace-py/src/millrace_ai/runtime/closure_transitions.py",
        "../millrace-py/tests/runtime/test_run_inspection.py",
        "../millrace-py/src/millrace_ai/runtime/inspection.py",
        "../millrace-py/tests/integration/test_e2e_handoffs.py",
        "../millrace-py/src/millrace_ai/runtime/handoff_incidents.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing Slice 8 advanced parity evidence source {source_path}"
        );
    }

    let preserved_contracts = fixture["preserved_python_contracts"]
        .as_array()
        .expect("preserved Python contracts are present");
    for contract in [
        "on_disk_workspace_shape",
        "headed_work_documents",
        "typed_runtime_json",
        "runner_artifact_filenames",
        "queue_lineage",
        "closure_target_state",
        "operator_controlled_source_promotion",
        "legal_terminal_markers",
    ] {
        assert!(
            preserved_contracts
                .iter()
                .any(|value| value.as_str() == Some(contract)),
            "missing preserved Python contract {contract}"
        );
    }

    let guarantees = fixture["always_on_guarantees"]
        .as_array()
        .expect("always-on guarantee list is present");
    for guarantee in [
        "fixture_backed_subscription_quota_telemetry",
        "scripted_fake_runner_only_for_e2e_handoffs",
        "mocked_runner_adapters_for_runtime_dispatch",
        "no_live_codex_cli_process",
        "no_live_pi_rpc_process",
        "no_network_or_credentials",
        "no_external_quota_service",
        "runtime_owned_queue_mutation_only",
        "read_only_run_inspection_does_not_mutate",
    ] {
        assert!(
            guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing Slice 8 always-on guarantee {guarantee}"
        );
    }

    let preview_only = fixture["preview_only_surfaces"]
        .as_array()
        .expect("preview-only surface list is present");
    for surface in [
        "native filesystem watcher integration",
        "live subscription quota provider polling",
        "live Codex runner smoke",
        "live Pi runner smoke",
    ] {
        assert!(
            preview_only
                .iter()
                .any(|value| value.as_str() == Some(surface)),
            "missing preview-only surface {surface}"
        );
    }

    let validation = fixture["completed_slice_validation"]
        .as_array()
        .expect("completed slice validation list is present");
    for command in [
        "cargo fmt --check",
        "cargo test --test runtime_serial",
        "cargo test --test runtime_daemon",
        "cargo test --test workspace_runtime_control",
        "cargo test --test parity_cli",
        "cargo test",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing Slice 8 validation command {command}"
        );
    }

    let non_goals = fixture["non_goals"]
        .as_array()
        .expect("non-goal list is present");
    for non_goal in [
        "native filesystem watcher integration",
        "live subscription quota provider polling in normal CI",
        "live Codex runner execution in normal CI",
        "live Pi runner execution in normal CI",
        "network access",
        "credentialed subscription services",
        "external quota services",
        "new queue states",
        "new stage names",
        "new terminal results",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing non-goal {non_goal}"
        );
    }
}

#[test]
fn committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(fixture["kind"], "web_dashboard_parity_decision");
    assert_eq!(fixture["release_surface"], "python_packages_millrace_web");
    assert_eq!(
        fixture["rust_decision"]["status"],
        "deferred_unsupported_gap"
    );
    assert_eq!(fixture["rust_decision"]["arbiter_visible"], true);
    assert!(
        fixture["rust_decision"]["reason"]
            .as_str()
            .expect("decision reason")
            .contains("No Rust-owned web/server/static-dashboard package target"),
        "dashboard gap reason must name the absent Rust-owned web/server target"
    );

    let required_surfaces = fixture["required_python_surfaces"]
        .as_array()
        .expect("required Python surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "workspace_registry",
        "summary_dtos",
        "queue_reader",
        "run_reader",
        "snapshot_reader",
        "baseline_reader",
        "compiled_plan_reader",
        "arbiter_reader",
        "usage_governance_reader",
        "event_stream",
        "static_shell",
        "cli_server_boundary",
        "package_boundary_tests",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let surfaces = fixture["python_surfaces"]
        .as_array()
        .expect("Python surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "unknown dashboard parity surface {surface_name}"
        );
        assert!(
            surface["rust_status"]
                .as_str()
                .expect("Rust status")
                .starts_with("deferred")
                || surface["rust_status"]
                    .as_str()
                    .expect("Rust status")
                    .starts_with("existing_read_only_cli_shadow_only"),
            "dashboard surface {surface_name} has unclear Rust status"
        );
        let python_sources = surface["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "dashboard surface {surface_name} is missing Python source references"
        );
        for source in python_sources {
            referenced_paths.insert(source.as_str().expect("source path").to_owned());
        }
        covered_surfaces.insert(surface_name);
    }
    assert_eq!(covered_surfaces, expected_surfaces);

    for source_path in [
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/cli.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/server.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/models.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/workspace_registry.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/queue_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/run_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/snapshot_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/baseline_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/compiled_plan_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/arbiter_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/usage_governance_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/event_stream.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/index.html",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/styles.css",
        "../millrace-py/packages/millrace-web/tests/test_app.py",
        "../millrace-py/packages/millrace-web/tests/test_packaging_boundary.py",
        "../millrace-py/packages/millrace-web/tests/test_summary_services.py",
        "../millrace-py/packages/millrace-web/tests/test_workspace_registry.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing web dashboard Python source reference {source_path}"
        );
    }

    let graph_trace = &fixture["v0_18_0_graph_trace_evidence"];
    assert_eq!(graph_trace["python_previous_tag"], "v0.17.4");
    assert_eq!(graph_trace["python_target_tag"], "v0.18.0");
    assert_eq!(graph_trace["diff_range"], "v0.17.4..v0.18.0");
    let graph_trace_sources = graph_trace["changed_python_sources"]
        .as_array()
        .expect("v0.18.0 graph/trace changed sources are present");
    for source_path in [
        "../millrace-py/packages/millrace-web/src/millrace_web/services/compiled_plan_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/run_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/styles.css",
        "../millrace-py/packages/millrace-web/tests/test_app.py",
    ] {
        assert!(
            graph_trace_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.0 web graph/trace source reference {source_path}"
        );
    }
    let graph_trace_gap_surfaces = graph_trace["required_gap_surfaces"]
        .as_array()
        .expect("v0.18.0 graph/trace gap surfaces are present");
    for surface in [
        "compiled_plan_graph_api_summary",
        "run_trace_api_summary",
        "recent_trace_flow_overlay",
        "trace_outcome_labels",
        "package_version_dependency_sync",
        "read_only_no_lock_guarantee",
    ] {
        assert!(
            graph_trace_gap_surfaces
                .iter()
                .any(|value| value.as_str() == Some(surface)),
            "missing v0.18.0 web graph/trace gap surface {surface}"
        );
    }

    let existing_surfaces = fixture["existing_non_mutating_rust_surfaces"]
        .as_array()
        .expect("existing Rust surface list is present");
    for surface in [
        "queue ls/show",
        "status/status show/status watch",
        "runs ls/show/tail/trace",
        "modes list/show",
        "config show",
        "compile show/graph",
    ] {
        assert!(
            existing_surfaces
                .iter()
                .any(|value| value.as_str() == Some(surface)),
            "missing existing read-only Rust surface {surface}"
        );
    }

    let guarantees = fixture["safety_guarantees"]
        .as_array()
        .expect("safety guarantees are present");
    for guarantee in [
        "existing_read_only_cli_commands_remain_non_mutating",
        "existing_read_only_cli_commands_do_not_acquire_runtime_ownership_lock",
        "no_control_or_queue_mutation_routes_are_added",
        "future_web_surface_must_be_local_workspace_only_if_implemented",
    ] {
        assert!(
            guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing dashboard safety guarantee {guarantee}"
        );
    }

    let non_goals = fixture["clear_non_goals"]
        .as_array()
        .expect("clear non-goal list is present");
    for non_goal in [
        "Rust does not ship a millrace-web package in this parity slice.",
        "Rust does not expose dashboard HTTP API routes in this parity slice.",
        "Rust does not ship a static dashboard shell in this parity slice.",
        "Rust does not expose an SSE event stream in this parity slice.",
        "Rust does not add authenticated or unauthenticated web control routes in this parity slice.",
    ] {
        assert!(
            non_goals
                .iter()
                .any(|value| value.as_str() == Some(non_goal)),
            "missing dashboard non-goal wording {non_goal}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_2_0() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_17_3_release_parity_evidence.json")
            .expect("read historical v0.17.3 release evidence fixture"),
    )
    .expect("parse historical v0.17.3 release evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.2.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.2.0"
    );
    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("historical package include list");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "historical release fixture missing package include rule {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_2_1() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_17_4_release_parity_evidence.json")
            .expect("read historical v0.17.4 release evidence fixture"),
    )
    .expect("parse historical v0.17.4 release evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.2.1");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.2.1"
    );
    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("historical package include list");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "historical v0.17.4 release fixture missing package include rule {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_3_0() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_0_release_parity_evidence.json")
            .expect("read v0.18.0 release parity evidence fixture"),
    )
    .expect("parse v0.18.0 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.0"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("historical v0.18.0 package include surfaces");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "historical v0.18.0 release fixture missing package include surface {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_3_1() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_1_release_parity_evidence.json")
            .expect("read v0.18.1 release parity evidence fixture"),
    )
    .expect("parse v0.18.1 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.1");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.1"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("historical v0.18.1 package include surfaces");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "historical v0.18.1 release fixture missing package include surface {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_3_2() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_2_release_parity_evidence.json")
            .expect("read v0.18.2 release parity evidence fixture"),
    )
    .expect("parse v0.18.2 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.2");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.2"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("historical v0.18.2 package include surfaces");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "historical v0.18.2 release fixture missing package include surface {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_3_3() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_3_release_parity_evidence.json")
            .expect("read v0.18.3 release parity evidence fixture"),
    )
    .expect("parse v0.18.3 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.3");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.3"
    );
    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("v0.18.3 package include surfaces");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "v0.18.3 fixture missing package include rule {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_3_4() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_4_release_parity_evidence.json")
            .expect("read v0.18.4 release parity evidence fixture"),
    )
    .expect("parse v0.18.4 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.4");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.4"
    );
    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("v0.18.4 package include surfaces");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "v0.18.4 fixture missing package include rule {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_3_5() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_6_release_parity_evidence.json")
            .expect("read v0.18.6 release parity evidence fixture"),
    )
    .expect("parse v0.18.6 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.5");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.5"
    );
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_4_0() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_19_0_release_parity_evidence.json")
            .expect("read v0.19.0 release parity evidence fixture"),
    )
    .expect("parse v0.19.0 release parity evidence fixture");
    assert_eq!(fixture["rust_release"]["crate_version"], "0.4.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.4.0"
    );
    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("v0.19.0 package include surfaces");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "v0.19.0 fixture missing package include rule {expected}"
        );
    }
}

#[test]
fn rust_crate_release_metadata_and_package_include_rules_are_0_5_0() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.5.0");

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(repo_root.join("Cargo.toml")).expect("read Cargo.toml"))
            .expect("parse Cargo.toml");
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .expect("package table");
    assert_eq!(
        package.get("version").and_then(toml::Value::as_str),
        Some("0.5.0")
    );
    let include = package
        .get("include")
        .and_then(toml::Value::as_array)
        .expect("package include list");
    for expected in [
        "/Cargo.lock",
        "/CHANGELOG.md",
        "/README.md",
        "/ROADMAP.md",
        "/docs/**/*.md",
        "/src/assets/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
        "/src/**/*.rs",
        "/tests/**/*.rs",
        "/tests/fixtures/**/*",
        "/tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing package include rule {expected}"
        );
    }
    for forbidden in [
        "millrace-agents/**",
        "ideas/**",
        "target/**",
        "/packages/millrace-web/**/*",
        "packages/millrace-web/**/*",
        "README.md",
        "ROADMAP.md",
    ] {
        assert!(
            !include
                .iter()
                .any(|value| value.as_str() == Some(forbidden)),
            "package include rule must not allow private or unanchored path {forbidden}"
        );
    }

    let lockfile = fs::read_to_string(repo_root.join("Cargo.lock")).expect("read Cargo.lock");
    assert!(
        lockfile.contains("[[package]]\nname = \"millrace-ai\"\nversion = \"0.5.0\"\n"),
        "Cargo.lock package metadata must track the 0.5.0 crate version"
    );
}

#[test]
fn committed_auto_port_v0_17_3_release_parity_evidence_covers_python_range_docs_assets_and_tests() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_17_3_release_parity_evidence.json")
            .expect("read final v0.17.3 auto-port parity evidence fixture"),
    )
    .expect("parse final v0.17.3 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_17_3_release_parity_evidence");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.16.1");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.17.3");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "a0d6b1bd5b71284eab7e9a5dcc9f76cee6580aaf"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.16.1..v0.17.3"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.2.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.2.0"
    );

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "thinking_level_contracts_compiler",
        "runner_thinking_artifacts",
        "daemon_monitor_idle_throttle",
        "closure_target_actionability",
        "task_lifecycle_integrity",
        "web_dashboard_unsupported_gap",
        "assets_docs_release_package",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let test_files = [
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
        "tests/runners_codex_cli.rs",
        "tests/runners_pi_rpc.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
        "tests/workspace_assets_baseline.rs",
        "tests/workspace_doctor.rs",
        "tests/workspace_queue_state_stores.rs",
    ];
    let available_tests = rust_test_functions_by_file(&test_files);
    let required_rust_refs = BTreeSet::from([
        "tests/compiler_contracts.rs::mode_and_graph_contracts_accept_runner_neutral_thinking_bindings",
        "tests/compiler_contracts.rs::stale_thinking_contract_shapes_are_rejected",
        "tests/compiler_materialization.rs::thinking_level_precedence_matches_python_materialization_contract",
        "tests/compiler_materialization.rs::codex_model_reasoning_effort_is_derived_from_effective_thinking_level",
        "tests/compiler_materialization.rs::conflicting_stage_thinking_aliases_are_rejected",
        "tests/compiler_parity.rs::rust_compiler_matches_python_normalized_plan_and_cli_fixtures",
        "tests/parity_cli.rs::rust_compile_show_renders_representative_inspection_fields",
        "tests/contracts_runtime_json.rs::python_produced_runtime_json_fixtures_round_trip_against_rust_contracts",
        "tests/runtime_serial.rs::shared_prompt_renderer_persists_stage_request_context",
        "tests/runtime_serial.rs::runner_artifact_contracts_capture_invocation_completion_and_process_evidence",
        "tests/runtime_serial.rs::serial_tick_dispatches_fake_runner_persists_artifacts_and_routes_from_graph",
        "tests/runners_codex_cli.rs::codex_adapter_writes_prompt_invocation_completion_events_stdout_and_tokens",
        "tests/runners_codex_cli.rs::codex_command_preserves_python_flag_order_and_environment_delta",
        "tests/runners_pi_rpc.rs::pi_adapter_prefers_request_thinking_level_over_global_default",
        "tests/runners_pi_rpc.rs::pi_command_preserves_provider_model_thinking_defaults_env_and_rejects_reserved_args",
        "tests/runtime_daemon.rs::basic_monitor_suppresses_repeated_idle_and_resets_after_activity",
        "tests/parity_cli.rs::rust_run_daemon_basic_monitor_prints_live_lines_to_stdout",
        "tests/runtime_serial.rs::serial_tick_blocked_closure_target_allows_unrelated_root_spec_to_activate",
        "tests/runtime_serial.rs::serial_tick_suppresses_closure_target_when_queued_lineage_work_remains",
        "tests/runtime_serial.rs::serial_tick_suppresses_closure_target_when_blocked_lineage_work_remains",
        "tests/parity_cli.rs::rust_status_prefers_actionable_closure_target_and_counts_deferred_roots",
        "tests/parity_cli.rs::rust_status_reports_multiple_actionable_closure_targets_as_invalid",
        "tests/workspace_queue_state_stores.rs::task_lifecycle_duplicate_scan_uses_parsed_ids_and_parse_error_filename_fallback",
        "tests/workspace_queue_state_stores.rs::mark_task_done_retires_same_root_blocked_duplicate_and_records_audit_evidence",
        "tests/workspace_queue_state_stores.rs::mark_task_done_keeps_different_root_or_unparseable_blocked_duplicate_in_place",
        "tests/workspace_doctor.rs::doctor_flags_duplicate_task_lifecycle_state_with_workspace_relative_paths",
        "tests/parity_cli.rs::rust_doctor_reports_duplicate_task_lifecycle_state_paths",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_2_0",
        "tests/workspace_assets_baseline.rs::packaged_baseline_manifest_is_sorted_hashed_and_deterministic",
        "tests/workspace_assets_baseline.rs::initialize_workspace_deploys_managed_assets_and_manifest_io",
        "tests/parity_cli.rs::committed_auto_port_v0_17_3_release_parity_evidence_covers_python_range_docs_assets_and_tests",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "final auto-port fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);

        let python_sources = surface["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "surface {surface_name} is missing Python source references"
        );
        for source in python_sources {
            referenced_paths.insert(source.as_str().expect("Python source path").to_owned());
        }

        let rust_tests = surface["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "surface {surface_name} is missing Rust test references"
        );
        for rust_test in rust_tests {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                is_snake_case_rust_test_name(test_name),
                "final auto-port fixture has malformed Rust test name {test_name}"
            );
            assert!(
                available_tests.contains_key(test_file),
                "final auto-port fixture references unsupported Rust test file {test_file}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "final auto-port fixture references unknown Rust test {rust_ref}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "final auto-port fixture references stale Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required final auto-port Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/.github/workflows/publish-to-pypi.yml",
        "../millrace-py/.github/workflows/repo-guardrails.yml",
        "../millrace-py/docs/source-package-map.md",
        "../millrace-py/src/millrace_ai/config/models.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/src/millrace_ai/runners/adapters/codex_cli.py",
        "../millrace-py/src/millrace_ai/runners/adapters/pi_rpc.py",
        "../millrace-py/src/millrace_ai/cli/monitoring.py",
        "../millrace-py/src/millrace_ai/runtime/completion_behavior.py",
        "../millrace-py/src/millrace_ai/workspace/task_lifecycle_integrity.py",
        "../millrace-py/src/millrace_ai/doctor.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/baseline_reader.py",
        "../millrace-py/packages/millrace-web/tests/test_packaging_boundary.py",
        "../millrace-py/tests/integration/test_compiler.py",
        "../millrace-py/tests/runtime/test_runtime.py",
        "../millrace-py/tests/workspace/test_queue_store.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing final auto-port Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing final auto-port local doc reference {doc_path}"
        );
    }

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing final auto-port validation command {command}"
        );
    }

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));
}

#[test]
fn committed_auto_port_v0_17_4_parity_fixture_covers_noop_trigger_runtime_and_cli_surfaces() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_17_4_parity_evidence.json")
            .expect("read v0.17.4 auto-port parity evidence fixture"),
    )
    .expect("parse v0.17.4 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_17_4_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.17.3");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.17.4");
    assert_ne!(
        fixture["python_reference"]["target_tag"], fixture["python_reference"]["previous_tag"],
        "v0.17.4 parity fixture target is stale"
    );
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "304e537964ff772c815689b87e4c1e3b805c656c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.17.3..v0.17.4"
    );

    let runtime_noop: Value = serde_json::from_str(
        &read_fixture("runtime_json/stage_result_learning_noop.json")
            .expect("read runtime JSON no-op fixture"),
    )
    .expect("parse runtime JSON no-op fixture");
    assert_eq!(runtime_noop["terminal_result"], "ANALYST_NOOP");
    assert_eq!(runtime_noop["result_class"], "no_op");
    assert_eq!(runtime_noop["success"], false);
    assert_eq!(runtime_noop["work_item_kind"], "learning_request");

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "noop_contracts_stage_metadata",
        "learning_assets_compiler_triggers",
        "runtime_learning_noop_lifecycle",
        "cli_runtime_json_inspection",
        "source_reference_guardrails",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();

    let test_files = [
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
        "tests/workspace_assets_baseline.rs",
    ];
    let available_tests = rust_test_functions_by_file(&test_files);
    let required_rust_refs = BTreeSet::from([
        "tests/compiler_contracts.rs::baseline_mode_graph_and_stage_kind_assets_parse_through_contracts",
        "tests/compiler_contracts.rs::learning_trigger_destination_metadata_normalizes_and_serializes",
        "tests/compiler_materialization.rs::learning_modes_materialize_learning_graph_triggers_and_concurrency_policy",
        "tests/compiler_materialization.rs::direct_curator_learning_trigger_requires_safe_destination",
        "tests/compiler_materialization.rs::direct_curator_learning_trigger_accepts_targeted_destination",
        "tests/compiler_parity.rs::rust_compiler_matches_python_normalized_plan_and_cli_fixtures",
        "tests/compiler_parity.rs::compiler_parity_fixture_documents_regeneration_surface",
        "tests/contracts_runtime_json.rs::python_v0_17_4_stage_result_no_op_runtime_json_fixture_round_trips_as_non_success",
        "tests/contracts_runtime_json.rs::python_v0_17_4_stage_result_no_op_runtime_json_round_trips_as_non_success",
        "tests/contracts_runtime_json.rs::python_v0_17_4_request_driven_no_op_terminal_identity_round_trips",
        "tests/runtime_serial.rs::python_v0_17_4_stage_run_request_preserves_learning_no_op_allowed_policy",
        "tests/runtime_serial.rs::python_v0_17_4_learning_noop_terminal_normalizes_to_non_success_noop_result",
        "tests/runtime_serial.rs::python_v0_17_4_learning_noop_rejects_mismatched_terminal_result_class_pairs",
        "tests/runtime_serial.rs::serial_tick_learning_trigger_enqueues_analyst_first_request",
        "tests/runtime_serial.rs::python_v0_17_4_runtime_generated_learning_request_copies_trigger_destination_metadata",
        "tests/runtime_serial.rs::python_v0_17_4_learning_noop_terminal_marks_request_done_with_noop_evidence",
        "tests/runtime_daemon.rs::daemon_supervisor_learning_mode_dispatches_learning_beside_execution",
        "tests/runtime_daemon.rs::daemon_supervisor_learning_mode_keeps_planning_and_execution_mutually_exclusive",
        "tests/workspace_assets_baseline.rs::initialized_workspace_learning_assets_match_packaged_noop_trigger_baseline",
        "tests/parity_cli.rs::rust_runs_show_displays_learning_noop_result_class_without_mutation",
        "tests/parity_cli.rs::committed_auto_port_v0_17_4_parity_fixture_covers_noop_trigger_runtime_and_cli_surfaces",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.17.4 fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);

        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.17.4 fixture references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }

        let python_sources = surface["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "surface {surface_name} is missing Python source references"
        );
        for source in python_sources {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }

        let rust_tests = surface["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "surface {surface_name} is missing Rust test references"
        );
        for rust_test in rust_tests {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                is_snake_case_rust_test_name(test_name),
                "v0.17.4 fixture has malformed Rust test name {test_name}"
            );
            assert!(
                available_tests.contains_key(test_file),
                "v0.17.4 fixture references unsupported Rust test file {test_file}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.17.4 fixture references unknown Rust test {rust_ref}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.17.4 fixture references stale Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for axis in required_axis_names {
        assert!(
            covered_axes.contains(axis),
            "missing v0.17.4 parity coverage axis {axis}"
        );
    }
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.17.4 Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/enums.py",
        "../millrace-py/src/millrace_ai/contracts/modes.py",
        "../millrace-py/src/millrace_ai/contracts/stage_metadata.py",
        "../millrace-py/src/millrace_ai/architecture/loop_graphs.py",
        "../millrace-py/src/millrace_ai/assets/graphs/learning/standard.json",
        "../millrace-py/src/millrace_ai/assets/loops/learning/default.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_pi.json",
        "../millrace-py/src/millrace_ai/assets/entrypoints/learning/analyst.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/learning/professor.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/learning/curator.md",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/learning/analyst.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/learning/professor.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/learning/curator.json",
        "../millrace-py/src/millrace_ai/assets/skills/stage/learning/analyst-core/SKILL.md",
        "../millrace-py/src/millrace_ai/assets/skills/stage/learning/professor-core/SKILL.md",
        "../millrace-py/src/millrace_ai/assets/skills/stage/learning/curator-core/SKILL.md",
        "../millrace-py/src/millrace_ai/compilation/learning_triggers.py",
        "../millrace-py/src/millrace_ai/runtime/learning_triggers.py",
        "../millrace-py/src/millrace_ai/workspace/state_reconciliation.py",
        "../millrace-py/tests/assets/test_entrypoints.py",
        "../millrace-py/tests/assets/test_loop_graphs.py",
        "../millrace-py/tests/assets/test_modes.py",
        "../millrace-py/tests/assets/test_stage_kinds.py",
        "../millrace-py/tests/integration/test_compiler.py",
        "../millrace-py/tests/runtime/test_contracts.py",
        "../millrace-py/tests/runtime/test_runtime.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.17.4 Python source reference {source_path}"
        );
    }

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no release upload",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.17.4 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_18_0_parity_fixture_covers_graph_trace_and_web_gap_scout() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_0_parity_evidence.json")
            .expect("read v0.18.0 auto-port parity evidence fixture"),
    )
    .expect("parse v0.18.0 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_0_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.17.4");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "304e537964ff772c815689b87e4c1e3b805c656c"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.0");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.17.4..v0.18.0"
    );
    assert_ne!(
        fixture["python_reference"]["target_tag"], fixture["python_reference"]["previous_tag"],
        "v0.18.0 parity fixture still treats the previous Python baseline as target"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.2.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_until_release_slice"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.0");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.0 parity fixture still treats Rust 0.2.1 as the planned target",
    );

    let compiler_fixture: Value = serde_json::from_str(
        &read_fixture("compiler_parity/python_compiler_parity.json")
            .expect("read compiler parity fixture"),
    )
    .expect("parse compiler parity fixture");
    match compiler_fixture["source"]["target_tag"]
        .as_str()
        .expect("compiler fixture target tag")
    {
        "v0.18.0" => {
            assert_eq!(compiler_fixture["source"]["previous_tag"], "v0.17.4");
            assert_eq!(
                compiler_fixture["source"]["previous_commit"],
                "304e537964ff772c815689b87e4c1e3b805c656c"
            );
            assert_eq!(
                compiler_fixture["source"]["target_commit"],
                "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
            );
            assert_eq!(
                compiler_fixture["source"]["diff_range"],
                fixture["python_reference"]["diff_range"]
            );
        }
        "v0.18.1" => {
            assert_eq!(compiler_fixture["source"]["previous_tag"], "v0.18.0");
            assert_eq!(
                compiler_fixture["source"]["previous_commit"],
                "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
            );
            assert_eq!(
                compiler_fixture["source"]["target_commit"],
                "0396c7852793b212d31345862b38a7d6f3f02854"
            );
            assert_eq!(compiler_fixture["source"]["diff_range"], "v0.18.0..v0.18.1");
        }
        "v0.20.0" => {
            assert_eq!(compiler_fixture["source"]["previous_tag"], "v0.19.0");
            assert_eq!(
                compiler_fixture["source"]["previous_commit"],
                "efb9c5881f524d23dcb78aecfc96fdf7cda9d26f"
            );
            assert_eq!(
                compiler_fixture["source"]["target_commit"],
                "c432786242e9e7cf9f7262ec0ec4f906f4bb7bf7"
            );
            assert_eq!(compiler_fixture["source"]["diff_range"], "v0.19.0..v0.20.0");
        }
        target_tag => panic!("unexpected compiler parity target tag {target_tag}"),
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "compiled_graph_export_contracts",
        "compiled_graph_export_projection",
        "run_trace_contracts",
        "runtime_trace_persistence_and_fallback",
        "cli_graph_trace_commands",
        "operator_docs_and_source_map",
        "web_dashboard_graph_trace_gap",
        "version_release_guardrails",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "generated_scout_changed_paths",
        "graph_export_contract_source_refs",
        "graph_export_test_source_refs",
        "run_trace_contract_source_refs",
        "run_trace_test_source_refs",
        "cli_graph_trace_source_refs",
        "runtime_trace_persistence_source_refs",
        "operator_doc_source_refs",
        "web_graph_trace_gap_evidence",
        "expected_rust_target_mapping",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.18.0 parity coverage axis {axis}"
        );
    }

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.0 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.18.0 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            referenced_targets.insert(target.as_str().expect("Rust target path").to_owned());
        }
    }
    for axis in [
        "graph_export_contract_source_refs",
        "run_trace_contract_source_refs",
        "cli_graph_trace_source_refs",
        "runtime_trace_persistence_source_refs",
        "operator_doc_source_refs",
        "web_graph_trace_gap_evidence",
        "expected_rust_target_mapping",
    ] {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.18.0 axis {axis}"
        );
    }

    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/graph_exports.py",
        "../millrace-py/src/millrace_ai/compilation/graph_exports.py",
        "../millrace-py/tests/integration/test_graph_exports.py",
        "../millrace-py/src/millrace_ai/contracts/run_trace.py",
        "../millrace-py/src/millrace_ai/runtime/run_traces.py",
        "../millrace-py/tests/runtime/test_run_traces.py",
        "../millrace-py/src/millrace_ai/cli/commands/compile.py",
        "../millrace-py/src/millrace_ai/cli/commands/runs.py",
        "../millrace-py/src/millrace_ai/cli/formatting.py",
        "../millrace-py/tests/cli/test_graph_trace_cli.py",
        "../millrace-py/src/millrace_ai/runtime/stage_result_persistence.py",
        "../millrace-py/src/millrace_ai/runtime/supervisor.py",
        "../millrace-py/src/millrace_ai/runtime/tick_cycle.py",
        "../millrace-py/docs/runtime/millrace-compiled-stage-graphs-and-run-traces.md",
        "../millrace-py/docs/skills/millrace-autonomous-delegation/SKILL.md",
        "../millrace-py/docs/skills/millrace-ops-agent-manual/SKILL.md",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/compiled_plan_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/run_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
        "../millrace-py/packages/millrace-web/tests/test_app.py",
    ] {
        assert!(
            referenced_sources.contains(source_path),
            "missing v0.18.0 Python source reference {source_path}"
        );
    }

    for target_path in [
        "src/contracts/graph_exports.rs",
        "src/contracts/run_trace.rs",
        "src/compiler/graph_exports.rs",
        "src/runtime/run_traces.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/parity_cli.rs",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.18.0 expected Rust target {target_path}"
        );
    }

    let expected_changed_paths = BTreeSet::from([
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/assets/images/millrace-icon-signal-transparent-glow.png",
        "docs/assets/images/millrace-icon-signal-transparent.png",
        "docs/millrace-technical-overview.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiled-stage-graphs-and-run-traces.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-runtime-lifecycle-diagram.md",
        "docs/skills/README.md",
        "docs/skills/millrace-autonomous-delegation/SKILL.md",
        "docs/skills/millrace-ops-agent-manual/SKILL.md",
        "docs/source-package-map.md",
        "packages/millrace-web/CHANGELOG.md",
        "packages/millrace-web/README.md",
        "packages/millrace-web/docs/README.md",
        "packages/millrace-web/pyproject.toml",
        "packages/millrace-web/src/millrace_web/__init__.py",
        "packages/millrace-web/src/millrace_web/app.py",
        "packages/millrace-web/src/millrace_web/models.py",
        "packages/millrace-web/src/millrace_web/services/compiled_plan_reader.py",
        "packages/millrace-web/src/millrace_web/services/run_reader.py",
        "packages/millrace-web/src/millrace_web/services/snapshot_reader.py",
        "packages/millrace-web/src/millrace_web/static/assets/app.js",
        "packages/millrace-web/src/millrace_web/static/assets/styles.css",
        "packages/millrace-web/tests/test_app.py",
        "src/millrace_ai/__init__.py",
        "src/millrace_ai/cli/__init__.py",
        "src/millrace_ai/cli/commands/compile.py",
        "src/millrace_ai/cli/commands/runs.py",
        "src/millrace_ai/cli/formatting.py",
        "src/millrace_ai/compilation/__init__.py",
        "src/millrace_ai/compilation/graph_exports.py",
        "src/millrace_ai/contracts/__init__.py",
        "src/millrace_ai/contracts/graph_exports.py",
        "src/millrace_ai/contracts/run_trace.py",
        "src/millrace_ai/run_inspection.py",
        "src/millrace_ai/runtime/engine.py",
        "src/millrace_ai/runtime/inspection.py",
        "src/millrace_ai/runtime/result_application.py",
        "src/millrace_ai/runtime/run_traces.py",
        "src/millrace_ai/runtime/stage_result_persistence.py",
        "src/millrace_ai/runtime/supervisor.py",
        "src/millrace_ai/runtime/tick_cycle.py",
        "src/millrace_ai/runtime/work_item_transitions.py",
        "tests/cli/test_graph_trace_cli.py",
        "tests/integration/test_graph_exports.py",
        "tests/runtime/test_run_traces.py",
    ]);
    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths, expected_changed_paths,
        "v0.18.0 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.0 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.18.0 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_unsupported_gap = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.18.0 path mapping uses unknown Rust target kind {kind}"
            );
            assert!(
                target["path"].as_str().is_some_and(|path| !path.is_empty()),
                "v0.18.0 path mapping has empty Rust target path"
            );
            has_unsupported_gap |= kind == "unsupported_gap_evidence";
        }
        if surface_name == "web_dashboard_graph_trace_gap" {
            assert!(
                has_unsupported_gap,
                "web dashboard mapping must remain explicit unsupported-gap evidence"
            );
        }
    }
    assert_eq!(covered_mapping_surfaces, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.18.0 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.18.0 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_0_graph_trace_evidence"]["python_target_tag"],
        "v0.18.0"
    );
    assert_eq!(
        dashboard_fixture["v0_18_0_graph_trace_evidence"]["diff_range"],
        "v0.17.4..v0.18.0"
    );
    let graph_trace_gap_surfaces =
        dashboard_fixture["v0_18_0_graph_trace_evidence"]["required_gap_surfaces"]
            .as_array()
            .expect("v0.18.0 web gap surface list");
    for surface in [
        "compiled_plan_graph_api_summary",
        "run_trace_api_summary",
        "recent_trace_flow_overlay",
        "trace_outcome_labels",
        "package_version_dependency_sync",
        "read_only_no_lock_guarantee",
    ] {
        assert!(
            graph_trace_gap_surfaces
                .iter()
                .any(|value| value.as_str() == Some(surface)),
            "missing v0.18.0 web gap surface {surface}"
        );
    }

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.0 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_18_1_parity_fixture_covers_probe_recon_release_guardrails() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_1_parity_evidence.json")
            .expect("read v0.18.1 auto-port parity evidence fixture"),
    )
    .expect("parse v0.18.1 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_1_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.0");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.1");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.0..v0.18.1"
    );
    assert_ne!(
        fixture["python_reference"]["target_tag"], fixture["python_reference"]["previous_tag"],
        "v0.18.1 parity fixture still treats Python v0.18.0 as the target"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "released_target_for_python_v0.18.1"
    );
    assert_eq!(
        fixture["rust_reference"]["previous_repo_crate_version"],
        "0.3.0"
    );
    assert_eq!(
        fixture["rust_reference"]["previous_repo_version_role"],
        "previous_baseline_for_python_v0.18.0"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.1");
    assert_eq!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.1 parity fixture must treat Rust 0.3.1 as the current target"
    );

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "probe_work_documents_and_recon_packets",
        "recon_managed_assets_compiler_graph",
        "probe_workspace_queue_lifecycle",
        "probe_cli_mailbox_readonly",
        "runtime_recon_routing_result_application",
        "docs_version_web_package_evidence",
        "source_reference_guardrails",
        "release_validation_guardrails",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "generated_scout_changed_paths",
        "probe_work_document_source_refs",
        "recon_packet_source_refs",
        "recon_asset_source_refs",
        "compiler_graph_source_refs",
        "queue_lifecycle_source_refs",
        "cli_mailbox_source_refs",
        "runtime_recon_source_refs",
        "docs_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.18.1 parity coverage axis {axis}"
        );
    }

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.1 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.18.1 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            referenced_targets.insert(target.as_str().expect("Rust target path").to_owned());
        }
    }

    for axis in [
        "probe_work_document_source_refs",
        "recon_packet_source_refs",
        "recon_asset_source_refs",
        "compiler_graph_source_refs",
        "queue_lifecycle_source_refs",
        "cli_mailbox_source_refs",
        "runtime_recon_source_refs",
        "docs_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
    ] {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.18.1 axis {axis}"
        );
    }

    for source_path in [
        "../millrace-py/src/millrace_ai/contracts/work_documents.py",
        "../millrace-py/src/millrace_ai/contracts/recon.py",
        "../millrace-py/src/millrace_ai/recon_packets.py",
        "../millrace-py/src/millrace_ai/assets/entrypoints/planning/recon.md",
        "../millrace-py/src/millrace_ai/assets/graphs/planning/standard.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/planning/recon.json",
        "../millrace-py/src/millrace_ai/assets/skills/stage/planning/recon-core/SKILL.md",
        "../millrace-py/src/millrace_ai/workspace/queue_store.py",
        "../millrace-py/src/millrace_ai/workspace/queue_selection.py",
        "../millrace-py/src/millrace_ai/cli/commands/queue.py",
        "../millrace-py/src/millrace_ai/contracts/mailbox.py",
        "../millrace-py/src/millrace_ai/runtime/mailbox_intake.py",
        "../millrace-py/src/millrace_ai/runtime/activation.py",
        "../millrace-py/src/millrace_ai/runtime/recon_transitions.py",
        "../millrace-py/src/millrace_ai/runtime/result_application.py",
        "../millrace-py/docs/runtime/millrace-cli-reference.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            referenced_sources.contains(source_path),
            "missing v0.18.1 Python source reference {source_path}"
        );
    }

    for target_path in [
        "src/contracts/work_documents.rs",
        "src/work_documents.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/runtime_control.rs",
        "src/cli/intake.rs",
        "src/cli/read_only.rs",
        "src/runtime/startup.rs",
        "src/runtime/tick.rs",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_1_runtime_contract_scout.json",
        "tests/fixtures/compiler_parity/auto_port_v0_18_1_compiler_contract_scout.json",
        "tests/parity_cli.rs",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.18.1 expected Rust target {target_path}"
        );
    }

    let expected_changed_paths = BTreeSet::from([
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/millrace-technical-overview.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-entrypoint-mapping.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/skills/millrace-ops-agent-manual/SKILL.md",
        "docs/source-package-map.md",
        "packages/millrace-web/CHANGELOG.md",
        "packages/millrace-web/pyproject.toml",
        "packages/millrace-web/src/millrace_web/__init__.py",
        "packages/millrace-web/src/millrace_web/app.py",
        "src/millrace_ai/__init__.py",
        "src/millrace_ai/architecture/loop_graphs.py",
        "src/millrace_ai/assets/entrypoints/planning/recon.md",
        "src/millrace_ai/assets/graphs/planning/standard.json",
        "src/millrace_ai/assets/modes/default_codex.json",
        "src/millrace_ai/assets/modes/default_pi.json",
        "src/millrace_ai/assets/modes/learning_codex.json",
        "src/millrace_ai/assets/modes/learning_pi.json",
        "src/millrace_ai/assets/registry/stage_kinds/planning/recon.json",
        "src/millrace_ai/assets/skills/skills_index.md",
        "src/millrace_ai/assets/skills/stage/planning/recon-core/SKILL.md",
        "src/millrace_ai/cli/app.py",
        "src/millrace_ai/cli/commands/queue.py",
        "src/millrace_ai/cli/shared.py",
        "src/millrace_ai/compilation/node_materialization.py",
        "src/millrace_ai/contracts/__init__.py",
        "src/millrace_ai/contracts/enums.py",
        "src/millrace_ai/contracts/mailbox.py",
        "src/millrace_ai/contracts/recon.py",
        "src/millrace_ai/contracts/stage_metadata.py",
        "src/millrace_ai/contracts/work_documents.py",
        "src/millrace_ai/recon_packets.py",
        "src/millrace_ai/router.py",
        "src/millrace_ai/runtime/activation.py",
        "src/millrace_ai/runtime/control.py",
        "src/millrace_ai/runtime/control_mutations.py",
        "src/millrace_ai/runtime/engine.py",
        "src/millrace_ai/runtime/graph_authority/planning.py",
        "src/millrace_ai/runtime/graph_authority/stage_mapping.py",
        "src/millrace_ai/runtime/mailbox_intake.py",
        "src/millrace_ai/runtime/recon_transitions.py",
        "src/millrace_ai/runtime/result_application.py",
        "src/millrace_ai/runtime/stage_requests.py",
        "src/millrace_ai/runtime/work_item_transitions.py",
        "src/millrace_ai/workspace/initialization.py",
        "src/millrace_ai/workspace/paths.py",
        "src/millrace_ai/workspace/queue_reconciliation.py",
        "src/millrace_ai/workspace/queue_selection.py",
        "src/millrace_ai/workspace/queue_store.py",
        "src/millrace_ai/workspace/queue_transitions.py",
        "src/millrace_ai/workspace/state_reconciliation.py",
        "src/millrace_ai/workspace/work_documents.py",
        "tests/assets/test_entrypoints.py",
        "tests/assets/test_loop_graphs.py",
        "tests/assets/test_stage_kinds.py",
        "tests/cli/test_cli.py",
        "tests/integration/test_compiler.py",
        "tests/integration/test_single_compiled_plan.py",
        "tests/runtime/test_graph_authority.py",
        "tests/runtime/test_result_application.py",
        "tests/workspace/test_paths.py",
        "tests/workspace/test_queue_store.py",
    ]);
    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths, expected_changed_paths,
        "v0.18.1 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let allowed_rust_targets = BTreeSet::from([
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/runtime/",
        "millrace-agents/outline.md",
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.0-to-v0.18.1-rust-0.3.1.md",
        "millrace-agents/entrypoints/planning/recon.md",
        "millrace-agents/graphs/planning/standard.json",
        "millrace-agents/modes/default_codex.json",
        "millrace-agents/modes/default_pi.json",
        "millrace-agents/modes/learning_codex.json",
        "millrace-agents/modes/learning_codex_auto_port.json",
        "millrace-agents/modes/learning_pi.json",
        "millrace-agents/registry/stage_kinds/planning/recon.json",
        "millrace-agents/skills/skills_index.md",
        "millrace-agents/skills/stage/planning/recon-core/SKILL.md",
        "src/assets/baseline/entrypoints/planning/recon.md",
        "src/assets/baseline/graphs/planning/standard.json",
        "src/assets/baseline/modes/default_codex.json",
        "src/assets/baseline/modes/default_pi.json",
        "src/assets/baseline/modes/learning_codex.json",
        "src/assets/baseline/modes/learning_pi.json",
        "src/assets/baseline/registry/stage_kinds/planning/recon.json",
        "src/assets/baseline/skills/skills_index.md",
        "src/assets/baseline/skills/stage/planning/recon-core/SKILL.md",
        "src/cli/intake.rs",
        "src/cli/mod.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "src/compiler/contracts.rs",
        "src/compiler/graph_exports.rs",
        "src/compiler/materialization.rs",
        "src/contracts/enums.rs",
        "src/contracts/mod.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/stage_metadata.rs",
        "src/contracts/work_documents.rs",
        "src/runtime/mod.rs",
        "src/runtime/run_traces.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/runners/contracts.rs",
        "src/work_documents.rs",
        "src/workspace.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/runtime_control.rs",
        "src/workspace/state_store.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/fixtures/cli_parity/auto_port_v0_18_1_parity_evidence.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/compiler_parity/auto_port_v0_18_1_compiler_contract_scout.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_1_runtime_contract_scout.json",
        "tests/fixtures/work_documents/",
        "tests/parity_cli.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        assert!(!python_path.is_empty(), "mapping has empty Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.1 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.18.1 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.18.1 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_unsupported_gap = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.18.1 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(path),
                "v0.18.1 path mapping uses unknown Rust target path {path}"
            );
            has_unsupported_gap |= kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_unsupported_gap
                    || rust_targets.iter().any(|target| {
                        target["kind"].as_str() == Some("package_evidence")
                            && target["path"].as_str()
                                == Some(
                                    "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
                                )
                    }),
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    assert_eq!(covered_mapping_surfaces, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.18.1 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.18.1 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.18.1 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "cargo fmt --check",
        "cargo test --test parity_cli",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.1 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_1_version_package_evidence"]["python_target_tag"],
        "v0.18.1"
    );
    assert_eq!(
        dashboard_fixture["v0_18_1_version_package_evidence"]["diff_range"],
        "v0.18.0..v0.18.1"
    );
    let web_sources =
        dashboard_fixture["v0_18_1_version_package_evidence"]["changed_python_sources"]
            .as_array()
            .expect("v0.18.1 web package source list");
    for source_path in [
        "../millrace-py/packages/millrace-web/CHANGELOG.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            web_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.1 web package evidence source {source_path}"
        );
    }

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.1 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_18_2_parity_fixture_covers_integrator_status_recon_ownership_guardrails()
{
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_2_parity_evidence.json")
            .expect("read v0.18.2 auto-port parity evidence fixture"),
    )
    .expect("parse v0.18.2 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_2_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.1");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.2");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.1..v0.18.2"
    );
    assert_ne!(
        fixture["python_reference"]["target_tag"], fixture["python_reference"]["previous_tag"],
        "v0.18.2 parity fixture still treats Python v0.18.1 as the target"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.1"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.1"
    );
    assert_eq!(
        fixture["rust_reference"]["previous_repo_version_role"],
        "released_target_for_python_v0.18.1"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.2");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.2 parity fixture still treats Rust 0.3.1 as the planned target"
    );

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "integrator_contracts_assets_compiler",
        "integrated_modes_runtime_routing",
        "status_json_diagnostics",
        "recon_handoff_graph_hardening",
        "stage_work_item_ownership",
        "docs_version_web_package_evidence",
        "source_reference_guardrails",
        "release_validation_guardrails",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "generated_scout_changed_paths",
        "integrator_asset_source_refs",
        "integrated_mode_source_refs",
        "status_json_source_refs",
        "recon_hardening_source_refs",
        "stage_work_item_ownership_source_refs",
        "docs_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "package_dry_run_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.18.2 parity coverage axis {axis}"
        );
    }

    let allowed_rust_targets = BTreeSet::from([
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/runtime/",
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.1-to-v0.18.2-rust-0.3.2.md",
        "millrace-agents/entrypoints/execution/checker.md",
        "millrace-agents/entrypoints/execution/integrator.md",
        "millrace-agents/graphs/execution/with_integrator.json",
        "millrace-agents/loops/execution/with_integrator.json",
        "millrace-agents/modes/default_codex_integrated.json",
        "millrace-agents/modes/learning_codex_integrated.json",
        "millrace-agents/registry/stage_kinds/execution/integrator.json",
        "millrace-agents/skills/skills_index.md",
        "millrace-agents/skills/stage/execution/checker-core/SKILL.md",
        "millrace-agents/skills/stage/execution/integrator-core/SKILL.md",
        "src/assets/baseline/entrypoints/execution/checker.md",
        "src/assets/baseline/entrypoints/execution/integrator.md",
        "src/assets/baseline/graphs/execution/with_integrator.json",
        "src/assets/baseline/loops/execution/with_integrator.json",
        "src/assets/baseline/modes/default_codex_integrated.json",
        "src/assets/baseline/modes/learning_codex_integrated.json",
        "src/assets/baseline/registry/stage_kinds/execution/integrator.json",
        "src/assets/baseline/skills/README.md",
        "src/assets/baseline/skills/skills_index.md",
        "src/assets/baseline/skills/stage/execution/checker-core/SKILL.md",
        "src/assets/baseline/skills/stage/execution/integrator-core/SKILL.md",
        "src/cli/parser.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "src/compiler/assets.rs",
        "src/compiler/contracts.rs",
        "src/compiler/graph_exports.rs",
        "src/compiler/materialization.rs",
        "src/contracts/enums.rs",
        "src/contracts/recon.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/stage_metadata.rs",
        "src/recon_packets.rs",
        "src/runtime/mod.rs",
        "src/runtime/run_traces.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/state_store.rs",
        "tests/compiler_assets.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/fixtures/cli_parity/auto_port_v0_18_2_parity_evidence.json",
        "tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/compiler_parity/auto_port_v0_18_2_compiler_contract_scout.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_2_runtime_contract_scout.json",
        "tests/parity_cli.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
        "tests/workspace_assets_baseline.rs",
        "tests/workspace_queue_state_stores.rs",
    ]);

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.2 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.18.2 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            let target_path = target.as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(target_path),
                "v0.18.2 source surface references unknown Rust target path {target_path}"
            );
            referenced_targets.insert(target_path.to_owned());
        }
    }

    for axis in [
        "integrator_asset_source_refs",
        "integrated_mode_source_refs",
        "status_json_source_refs",
        "recon_hardening_source_refs",
        "stage_work_item_ownership_source_refs",
        "docs_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "package_dry_run_evidence",
    ] {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.18.2 axis {axis}"
        );
    }

    for source_path in [
        "../millrace-py/src/millrace_ai/assets/entrypoints/execution/integrator.md",
        "../millrace-py/src/millrace_ai/assets/graphs/execution/with_integrator.json",
        "../millrace-py/src/millrace_ai/assets/loops/execution/with_integrator.json",
        "../millrace-py/src/millrace_ai/assets/modes/default_codex_integrated.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex_integrated.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/execution/integrator.json",
        "../millrace-py/src/millrace_ai/assets/skills/stage/execution/integrator-core/SKILL.md",
        "../millrace-py/src/millrace_ai/cli/commands/status.py",
        "../millrace-py/src/millrace_ai/cli/status_view.py",
        "../millrace-py/src/millrace_ai/contracts/recon.py",
        "../millrace-py/src/millrace_ai/runtime/error_recovery.py",
        "../millrace-py/src/millrace_ai/runtime/recon_transitions.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/tests/runtime/test_recon_packets.py",
        "../millrace-py/tests/runtime/test_runtime.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
    ] {
        assert!(
            referenced_sources.contains(source_path),
            "missing v0.18.2 Python source reference {source_path}"
        );
    }

    for target_path in [
        "millrace-agents/entrypoints/execution/integrator.md",
        "millrace-agents/graphs/execution/with_integrator.json",
        "millrace-agents/modes/default_codex_integrated.json",
        "src/contracts/enums.rs",
        "src/contracts/recon.rs",
        "src/contracts/runtime_json.rs",
        "src/runtime/tick.rs",
        "src/runtime/supervisor.rs",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/compiler_parity/auto_port_v0_18_2_compiler_contract_scout.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_2_runtime_contract_scout.json",
        "tests/parity_cli.rs",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.18.2 expected Rust target {target_path}"
        );
    }

    let expected_changed_paths = BTreeSet::from([
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/millrace-technical-overview.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-arbiter-and-completion-behavior.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-entrypoint-mapping.md",
        "docs/runtime/millrace-loop-authoring.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runner-architecture.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-runtime-error-codes.md",
        "docs/runtime/millrace-runtime-lifecycle-diagram.md",
        "docs/runtime/millrace-workspace-baselines-and-upgrades.md",
        "docs/skills/millrace-ops-agent-manual/SKILL.md",
        "docs/source-package-map.md",
        "packages/millrace-web/CHANGELOG.md",
        "packages/millrace-web/pyproject.toml",
        "packages/millrace-web/src/millrace_web/__init__.py",
        "packages/millrace-web/src/millrace_web/app.py",
        "src/millrace_ai/__init__.py",
        "src/millrace_ai/assets/entrypoints/execution/checker.md",
        "src/millrace_ai/assets/entrypoints/execution/integrator.md",
        "src/millrace_ai/assets/graphs/execution/with_integrator.json",
        "src/millrace_ai/assets/loop_graphs.py",
        "src/millrace_ai/assets/loops/execution/with_integrator.json",
        "src/millrace_ai/assets/modes.py",
        "src/millrace_ai/assets/modes/default_codex_integrated.json",
        "src/millrace_ai/assets/modes/learning_codex_integrated.json",
        "src/millrace_ai/assets/registry/stage_kinds/execution/integrator.json",
        "src/millrace_ai/assets/skills/README.md",
        "src/millrace_ai/assets/skills/skills_index.md",
        "src/millrace_ai/assets/skills/stage/execution/checker-core/SKILL.md",
        "src/millrace_ai/assets/skills/stage/execution/integrator-core/SKILL.md",
        "src/millrace_ai/cli/commands/status.py",
        "src/millrace_ai/cli/status_view.py",
        "src/millrace_ai/contracts/enums.py",
        "src/millrace_ai/contracts/recon.py",
        "src/millrace_ai/contracts/stage_metadata.py",
        "src/millrace_ai/errors.py",
        "src/millrace_ai/runtime/engine.py",
        "src/millrace_ai/runtime/error_recovery.py",
        "src/millrace_ai/runtime/recon_transitions.py",
        "src/millrace_ai/runtime/result_application.py",
        "src/millrace_ai/runtime/stage_requests.py",
        "src/millrace_ai/runtime/supervisor.py",
        "src/millrace_ai/runtime/tick_cycle.py",
        "tests/assets/test_entrypoints.py",
        "tests/assets/test_loop_graphs.py",
        "tests/assets/test_modes.py",
        "tests/assets/test_packaging_runtime_assets.py",
        "tests/assets/test_stage_kinds.py",
        "tests/cli/test_cli.py",
        "tests/runtime/test_graph_authority.py",
        "tests/runtime/test_recon_packets.py",
        "tests/runtime/test_runtime.py",
    ]);

    let generated_scout_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        fixture["python_reference"]["generated_scout"]
            .as_str()
            .expect("generated scout path"),
    );
    let generated_scout =
        fs::read_to_string(generated_scout_path).expect("read v0.18.2 generated scout");
    let mut generated_changed_paths = BTreeSet::new();
    let mut in_changed_paths = false;
    for line in generated_scout.lines() {
        if line == "Changed Python paths:" {
            in_changed_paths = true;
            continue;
        }
        if !in_changed_paths {
            continue;
        }
        if line == "Diff stat:" {
            break;
        }
        if let Some(path) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            generated_changed_paths.insert(path);
        }
    }
    assert_eq!(
        generated_changed_paths, expected_changed_paths,
        "v0.18.2 generated scout changed paths drifted from guardrail expectation"
    );

    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths.len(),
        changed_mappings.len(),
        "v0.18.2 path mappings must not contain duplicate Python paths"
    );
    assert_eq!(
        mapped_changed_paths, generated_changed_paths,
        "v0.18.2 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        assert!(!python_path.is_empty(), "mapping has empty Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.2 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.18.2 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.18.2 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_package_or_gap_evidence = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.18.2 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(path),
                "v0.18.2 path mapping uses unknown Rust target path {path}"
            );
            has_package_or_gap_evidence |=
                kind == "package_evidence" || kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_package_or_gap_evidence,
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    let expected_mapping_surfaces = BTreeSet::from([
        "integrator_contracts_assets_compiler",
        "integrated_modes_runtime_routing",
        "status_json_diagnostics",
        "recon_handoff_graph_hardening",
        "stage_work_item_ownership",
        "docs_version_web_package_evidence",
        "release_validation_guardrails",
    ]);
    assert_eq!(covered_mapping_surfaces, expected_mapping_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.18.2 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.18.2 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.18.2 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo publish --dry-run",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.2 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_2_version_package_evidence"]["python_target_tag"],
        "v0.18.2"
    );
    assert_eq!(
        dashboard_fixture["v0_18_2_version_package_evidence"]["diff_range"],
        "v0.18.1..v0.18.2"
    );
    let web_sources =
        dashboard_fixture["v0_18_2_version_package_evidence"]["changed_python_sources"]
            .as_array()
            .expect("v0.18.2 web package source list");
    for source_path in [
        "../millrace-py/packages/millrace-web/CHANGELOG.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            web_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.2 web package evidence source {source_path}"
        );
    }

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.2 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_18_3_parity_fixture_covers_librarian_learning_trigger_metadata_skill_lint_and_web_guardrails()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_3_parity_evidence.json")
            .expect("read v0.18.3 auto-port parity evidence fixture"),
    )
    .expect("parse v0.18.3 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_3_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.2");
    assert_eq!(
        fixture["python_reference"]["previous_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.3");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "6556e55c8463ce9256716bc425a49059b4c5981c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.2..v0.18.3"
    );
    assert_ne!(
        fixture["python_reference"]["target_tag"], fixture["python_reference"]["previous_tag"],
        "v0.18.3 parity fixture still treats Python v0.18.2 as the target"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.2"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.2"
    );
    assert_eq!(
        fixture["rust_reference"]["previous_repo_version_role"],
        "released_target_for_python_v0.18.2"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.3");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.3 parity fixture still treats Rust 0.3.2 as the planned target"
    );

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "librarian_contracts_stage_metadata",
        "librarian_assets_compiler_modes",
        "planner_librarian_learning_triggers",
        "learning_request_artifact_metadata",
        "runner_normalization_metadata",
        "shipped_skill_lint_guidance_handoffs",
        "docs_version_web_package_evidence",
        "source_reference_guardrails",
        "release_validation_guardrails",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "auto_port_state_currentness",
        "generated_scout_changed_paths",
        "librarian_contract_source_refs",
        "librarian_asset_source_refs",
        "librarian_graph_mode_source_refs",
        "planner_librarian_trigger_source_refs",
        "learning_request_artifact_metadata_source_refs",
        "runner_normalization_metadata_source_refs",
        "shipped_skill_lint_source_refs",
        "guidance_handoff_source_refs",
        "docs_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "package_dry_run_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.18.3 parity coverage axis {axis}"
        );
    }

    let allowed_rust_targets = BTreeSet::from([
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/millrace-technical-overview.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/runtime/",
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.2-to-v0.18.3-rust-0.3.3.md",
        "millrace-agents/auto-port/state.json",
        "millrace-agents/entrypoints/learning/curator.md",
        "millrace-agents/entrypoints/learning/librarian.md",
        "millrace-agents/entrypoints/planning/planner.md",
        "millrace-agents/entrypoints/planning/recon.md",
        "millrace-agents/graphs/learning/standard.json",
        "millrace-agents/loops/learning/default.json",
        "millrace-agents/modes/learning_codex.json",
        "millrace-agents/modes/learning_codex_auto_port.json",
        "millrace-agents/modes/learning_codex_integrated.json",
        "millrace-agents/modes/learning_pi.json",
        "millrace-agents/registry/stage_kinds/learning/librarian.json",
        "millrace-agents/skills/README.md",
        "millrace-agents/skills/skills_index.md",
        "millrace-agents/skills/shared/marathon-qa-audit/SKILL.md",
        "millrace-agents/skills/stage/learning/curator-core/SKILL.md",
        "millrace-agents/skills/stage/learning/librarian-core/SKILL.md",
        "millrace-agents/skills/stage/planning/recon-core/SKILL.md",
        "src/assets/baseline/entrypoints/learning/curator.md",
        "src/assets/baseline/entrypoints/learning/librarian.md",
        "src/assets/baseline/entrypoints/planning/planner.md",
        "src/assets/baseline/entrypoints/planning/recon.md",
        "src/assets/baseline/graphs/learning/standard.json",
        "src/assets/baseline/loops/learning/default.json",
        "src/assets/baseline/modes/learning_codex.json",
        "src/assets/baseline/modes/learning_codex_integrated.json",
        "src/assets/baseline/modes/learning_pi.json",
        "src/assets/baseline/registry/stage_kinds/learning/librarian.json",
        "src/assets/baseline/skills/README.md",
        "src/assets/baseline/skills/skills_index.md",
        "src/assets/baseline/skills/shared/marathon-qa-audit/SKILL.md",
        "src/assets/baseline/skills/stage/learning/curator-core/SKILL.md",
        "src/assets/baseline/skills/stage/learning/librarian-core/SKILL.md",
        "src/assets/baseline/skills/stage/planning/recon-core/SKILL.md",
        "src/compiler/assets.rs",
        "src/compiler/contracts.rs",
        "src/compiler/graph_exports.rs",
        "src/compiler/materialization.rs",
        "src/contracts/enums.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/stage_metadata.rs",
        "src/runtime/mod.rs",
        "src/runtime/run_traces.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/runners/normalization.rs",
        "tests/compiler_assets.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/fixtures/cli_parity/auto_port_v0_18_3_parity_evidence.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/compiler_parity/auto_port_v0_18_3_compiler_contract_scout.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_3_runtime_contract_scout.json",
        "tests/parity_cli.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
        "tests/runners_normalization.rs",
        "tests/shipped_skill_lint.rs",
        "tests/workspace_assets_baseline.rs",
    ]);

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.3 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.18.3 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            let target_path = target.as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(target_path),
                "v0.18.3 source surface references unknown Rust target path {target_path}"
            );
            referenced_targets.insert(target_path.to_owned());
        }
    }

    for axis in [
        "librarian_contract_source_refs",
        "librarian_asset_source_refs",
        "librarian_graph_mode_source_refs",
        "planner_librarian_trigger_source_refs",
        "learning_request_artifact_metadata_source_refs",
        "runner_normalization_metadata_source_refs",
        "shipped_skill_lint_source_refs",
        "guidance_handoff_source_refs",
        "docs_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "package_dry_run_evidence",
    ] {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.18.3 axis {axis}"
        );
    }

    for source_path in [
        "../millrace-py/src/millrace_ai/assets/entrypoints/learning/librarian.md",
        "../millrace-py/src/millrace_ai/assets/graphs/learning/standard.json",
        "../millrace-py/src/millrace_ai/assets/loops/learning/default.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_codex_integrated.json",
        "../millrace-py/src/millrace_ai/assets/modes/learning_pi.json",
        "../millrace-py/src/millrace_ai/assets/registry/stage_kinds/learning/librarian.json",
        "../millrace-py/src/millrace_ai/assets/skills/shared/marathon-qa-audit/SKILL.md",
        "../millrace-py/src/millrace_ai/assets/skills/stage/learning/librarian-core/SKILL.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/planning/planner.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/planning/recon.md",
        "../millrace-py/src/millrace_ai/compilation/node_materialization.py",
        "../millrace-py/src/millrace_ai/runners/normalization.py",
        "../millrace-py/src/millrace_ai/runtime/learning_triggers.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/tests/assets/test_shipped_skill_lint.py",
        "../millrace-py/tests/runtime/test_runtime.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
    ] {
        assert!(
            referenced_sources.contains(source_path),
            "missing v0.18.3 Python source reference {source_path}"
        );
    }

    for target_path in [
        "millrace-agents/entrypoints/learning/librarian.md",
        "millrace-agents/graphs/learning/standard.json",
        "millrace-agents/modes/learning_codex_auto_port.json",
        "millrace-agents/registry/stage_kinds/learning/librarian.json",
        "src/contracts/enums.rs",
        "src/contracts/stage_metadata.rs",
        "src/runners/normalization.rs",
        "src/runtime/tick.rs",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/compiler_parity/auto_port_v0_18_3_compiler_contract_scout.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_3_runtime_contract_scout.json",
        "tests/parity_cli.rs",
        "tests/shipped_skill_lint.rs",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.18.3 expected Rust target {target_path}"
        );
    }

    let expected_changed_paths = BTreeSet::from([
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/millrace-technical-overview.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-entrypoint-mapping.md",
        "docs/runtime/millrace-loop-authoring.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-runtime-lifecycle-diagram.md",
        "docs/runtime/millrace-workspace-baselines-and-upgrades.md",
        "docs/skills/millrace-ops-agent-manual/SKILL.md",
        "docs/source-package-map.md",
        "packages/millrace-web/CHANGELOG.md",
        "packages/millrace-web/pyproject.toml",
        "packages/millrace-web/src/millrace_web/__init__.py",
        "packages/millrace-web/src/millrace_web/app.py",
        "src/millrace_ai/__init__.py",
        "src/millrace_ai/assets/entrypoints/learning/curator.md",
        "src/millrace_ai/assets/entrypoints/learning/librarian.md",
        "src/millrace_ai/assets/entrypoints/planning/planner.md",
        "src/millrace_ai/assets/entrypoints/planning/recon.md",
        "src/millrace_ai/assets/graphs/learning/standard.json",
        "src/millrace_ai/assets/loops/learning/default.json",
        "src/millrace_ai/assets/modes/learning_codex.json",
        "src/millrace_ai/assets/modes/learning_codex_integrated.json",
        "src/millrace_ai/assets/modes/learning_pi.json",
        "src/millrace_ai/assets/registry/stage_kinds/learning/librarian.json",
        "src/millrace_ai/assets/skills/README.md",
        "src/millrace_ai/assets/skills/shared/marathon-qa-audit/SKILL.md",
        "src/millrace_ai/assets/skills/skills_index.md",
        "src/millrace_ai/assets/skills/stage/learning/curator-core/SKILL.md",
        "src/millrace_ai/assets/skills/stage/learning/librarian-core/SKILL.md",
        "src/millrace_ai/assets/skills/stage/planning/recon-core/SKILL.md",
        "src/millrace_ai/compilation/node_materialization.py",
        "src/millrace_ai/contracts/enums.py",
        "src/millrace_ai/contracts/stage_metadata.py",
        "src/millrace_ai/runners/normalization.py",
        "src/millrace_ai/runtime/learning_triggers.py",
        "src/millrace_ai/runtime/stage_requests.py",
        "tests/assets/test_entrypoints.py",
        "tests/assets/test_loop_graphs.py",
        "tests/assets/test_modes.py",
        "tests/assets/test_packaging_runtime_assets.py",
        "tests/assets/test_shipped_skill_lint.py",
        "tests/assets/test_stage_kinds.py",
        "tests/integration/test_compiler.py",
        "tests/runtime/test_runtime.py",
    ]);

    let generated_scout_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        fixture["python_reference"]["generated_scout"]
            .as_str()
            .expect("generated scout path"),
    );
    let generated_scout =
        fs::read_to_string(generated_scout_path).expect("read v0.18.3 generated scout");
    let mut generated_changed_paths = BTreeSet::new();
    let mut in_changed_paths = false;
    for line in generated_scout.lines() {
        if line == "Changed Python paths:" {
            in_changed_paths = true;
            continue;
        }
        if !in_changed_paths {
            continue;
        }
        if line == "Diff stat:" {
            break;
        }
        if let Some(path) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            generated_changed_paths.insert(path);
        }
    }
    assert_eq!(
        generated_changed_paths, expected_changed_paths,
        "v0.18.3 generated scout changed paths drifted from guardrail expectation"
    );

    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths.len(),
        changed_mappings.len(),
        "v0.18.3 path mappings must not contain duplicate Python paths"
    );
    assert_eq!(
        mapped_changed_paths, generated_changed_paths,
        "v0.18.3 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        assert!(!python_path.is_empty(), "mapping has empty Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.3 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.18.3 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.18.3 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_package_or_gap_evidence = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.18.3 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(path),
                "v0.18.3 path mapping uses unknown Rust target path {path}"
            );
            if python_path.starts_with("packages/millrace-web/") {
                assert_ne!(
                    kind, "implementation",
                    "web package mapping must not authorize Rust web implementation expansion"
                );
            }
            has_package_or_gap_evidence |=
                kind == "package_evidence" || kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_package_or_gap_evidence,
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    let expected_mapping_surfaces = BTreeSet::from([
        "librarian_contracts_stage_metadata",
        "librarian_assets_compiler_modes",
        "planner_librarian_learning_triggers",
        "learning_request_artifact_metadata",
        "runner_normalization_metadata",
        "shipped_skill_lint_guidance_handoffs",
        "docs_version_web_package_evidence",
        "release_validation_guardrails",
    ]);
    assert_eq!(covered_mapping_surfaces, expected_mapping_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.18.3 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.18.3 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.18.3 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.18.2..v0.18.3",
        "cargo fmt --check",
        "cargo test --test compiler_parity",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo test --test compiler_contracts",
        "cargo test --test compiler_materialization",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo publish --dry-run",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.3 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_3_version_package_evidence"]["python_target_tag"],
        "v0.18.3"
    );
    assert_eq!(
        dashboard_fixture["v0_18_3_version_package_evidence"]["diff_range"],
        "v0.18.2..v0.18.3"
    );
    let web_sources =
        dashboard_fixture["v0_18_3_version_package_evidence"]["changed_python_sources"]
            .as_array()
            .expect("v0.18.3 web package source list");
    for source_path in [
        "../millrace-py/packages/millrace-web/CHANGELOG.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            web_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.3 web package evidence source {source_path}"
        );
    }
    assert_eq!(
        dashboard_fixture["v0_18_3_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.3 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no remote skill installation",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.3 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_18_4_parity_fixture_covers_blocked_recovery_retry_autorecovery_and_web_guardrails()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_4_parity_evidence.json")
            .expect("read v0.18.4 auto-port parity evidence fixture"),
    )
    .expect("parse v0.18.4 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_4_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.3");
    assert_eq!(
        fixture["python_reference"]["previous_tag_commit"],
        "6fbb3c7b9d23e4c61b178e0a8d129c3fa540060e"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.4");
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "acf4f637c4e983793011c3bc5977d8a72e79e7cd"
    );
    assert_eq!(
        fixture["python_reference"]["release_commit"],
        "516e947e90155b6436dbc9efcf932254f34bc39c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.3..v0.18.4"
    );
    assert_ne!(
        fixture["python_reference"]["target_tag"], fixture["python_reference"]["previous_tag"],
        "v0.18.4 parity fixture still treats Python v0.18.3 as the target"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.3"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_version_role"],
        "previous_baseline_for_python_v0.18.3"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.4");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.4 parity fixture still treats Rust 0.3.3 as the planned target"
    );

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "runner_failure_metadata_contracts",
        "blocked_metadata_persistence",
        "runtime_router_blocked_recovery_paths",
        "queue_retry_blocked_cli",
        "workspace_queue_blocked_requeue",
        "auto_recovery_config_status",
        "daemon_stranded_dependency_auto_recovery",
        "docs_operator_skill_and_release_evidence",
        "web_version_package_evidence",
        "source_reference_guardrails",
        "release_validation_guardrails",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let required_axes = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present");
    let required_axis_names: BTreeSet<_> = required_axes
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "auto_port_state_currentness",
        "generated_scout_changed_paths",
        "blocked_recovery_source_refs",
        "runner_failure_classifier_metadata",
        "blocked_metadata_schema_fields",
        "blocked_metadata_diagnostics_paths",
        "queue_retry_blocked_cli_behavior",
        "queue_requeue_audit_jsonl",
        "auto_recovery_config_defaults",
        "auto_recovery_change_boundaries",
        "status_config_auto_recovery_surface",
        "daemon_idle_auto_recovery_decisions",
        "runtime_event_monitor_evidence",
        "docs_source_refs",
        "operator_skill_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.18.4 parity coverage axis {axis}"
        );
    }

    let allowed_rust_targets = BTreeSet::from([
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-arbiter-and-completion-behavior.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.3-to-v0.18.4-rust-0.3.4.md",
        "millrace-agents/auto-port/state.json",
        "src/lib.rs",
        "src/contracts/runtime_json.rs",
        "src/runners/normalization.rs",
        "src/runners/contracts.rs",
        "src/runtime/mod.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/runtime/monitor.rs",
        "src/workspace.rs",
        "src/workspace/queue_store.rs",
        "src/cli/mod.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "src/cli/parser.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runners_normalization.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
        "tests/parity_cli.rs",
        "tests/support/parity.rs",
        "tests/fixtures/cli_parity/auto_port_v0_18_4_parity_evidence.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/runtime_json/auto_port_v0_18_4_runtime_contract_scout.json",
    ]);

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.4 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.18.4 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            let target_path = target.as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(target_path),
                "v0.18.4 source surface references unknown Rust target path {target_path}"
            );
            referenced_targets.insert(target_path.to_owned());
        }
    }
    for axis in [
        "blocked_recovery_source_refs",
        "runner_failure_classifier_metadata",
        "blocked_metadata_schema_fields",
        "blocked_metadata_diagnostics_paths",
        "queue_retry_blocked_cli_behavior",
        "queue_requeue_audit_jsonl",
        "auto_recovery_config_defaults",
        "auto_recovery_change_boundaries",
        "status_config_auto_recovery_surface",
        "daemon_idle_auto_recovery_decisions",
        "runtime_event_monitor_evidence",
        "docs_source_refs",
        "operator_skill_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.18.4 axis {axis}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/docs/runtime/millrace-cli-reference.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/docs/skills/millrace-ops-agent-manual/SKILL.md",
        "../millrace-py/packages/millrace-web/CHANGELOG.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
        "../millrace-py/src/millrace_ai/__init__.py",
        "../millrace-py/src/millrace_ai/cli/commands/queue.py",
        "../millrace-py/src/millrace_ai/cli/config_view.py",
        "../millrace-py/src/millrace_ai/config/__init__.py",
        "../millrace-py/src/millrace_ai/config/boundaries.py",
        "../millrace-py/src/millrace_ai/config/models.py",
        "../millrace-py/src/millrace_ai/runners/normalization.py",
        "../millrace-py/src/millrace_ai/runtime/blocked_recovery.py",
        "../millrace-py/src/millrace_ai/runtime/recon_transitions.py",
        "../millrace-py/src/millrace_ai/runtime/result_application.py",
        "../millrace-py/src/millrace_ai/runtime/supervisor.py",
        "../millrace-py/src/millrace_ai/runtime/work_item_transitions.py",
        "../millrace-py/src/millrace_ai/workspace/queue_store.py",
        "../millrace-py/src/millrace_ai/workspace/queue_transitions.py",
        "../millrace-py/tests/cli/test_cli.py",
        "../millrace-py/tests/config/test_config.py",
        "../millrace-py/tests/runners/test_runner.py",
        "../millrace-py/tests/runtime/test_supervisor.py",
        "../millrace-py/tests/workspace/test_queue_store.py",
    ] {
        assert!(
            referenced_sources.contains(source_path),
            "missing v0.18.4 Python source reference {source_path}"
        );
    }
    for target_path in [
        "src/runners/normalization.rs",
        "src/contracts/runtime_json.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/workspace/queue_store.rs",
        "src/cli/mod.rs",
        "src/cli/read_only.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runners_normalization.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
        "tests/fixtures/runtime_json/auto_port_v0_18_4_runtime_contract_scout.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.18.4 expected Rust target {target_path}"
        );
    }

    let expected_changed_paths = BTreeSet::from([
        "CHANGELOG.md",
        "README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/skills/millrace-ops-agent-manual/SKILL.md",
        "packages/millrace-web/CHANGELOG.md",
        "packages/millrace-web/pyproject.toml",
        "packages/millrace-web/src/millrace_web/__init__.py",
        "packages/millrace-web/src/millrace_web/app.py",
        "src/millrace_ai/__init__.py",
        "src/millrace_ai/cli/commands/queue.py",
        "src/millrace_ai/cli/config_view.py",
        "src/millrace_ai/config/__init__.py",
        "src/millrace_ai/config/boundaries.py",
        "src/millrace_ai/config/models.py",
        "src/millrace_ai/runners/normalization.py",
        "src/millrace_ai/runtime/blocked_recovery.py",
        "src/millrace_ai/runtime/recon_transitions.py",
        "src/millrace_ai/runtime/result_application.py",
        "src/millrace_ai/runtime/supervisor.py",
        "src/millrace_ai/runtime/work_item_transitions.py",
        "src/millrace_ai/workspace/queue_store.py",
        "src/millrace_ai/workspace/queue_transitions.py",
        "tests/cli/test_cli.py",
        "tests/config/test_config.py",
        "tests/runners/test_runner.py",
        "tests/runtime/test_supervisor.py",
        "tests/workspace/test_queue_store.py",
    ]);

    let generated_scout_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        fixture["python_reference"]["generated_scout"]
            .as_str()
            .expect("generated scout path"),
    );
    let generated_scout =
        fs::read_to_string(generated_scout_path).expect("read v0.18.4 generated scout");
    let mut generated_changed_paths = BTreeSet::new();
    let mut in_changed_paths = false;
    for line in generated_scout.lines() {
        if line == "Changed Python paths:" {
            in_changed_paths = true;
            continue;
        }
        if !in_changed_paths {
            continue;
        }
        if line == "Diff stat:" {
            break;
        }
        if let Some(path) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            generated_changed_paths.insert(path);
        }
    }
    assert_eq!(
        generated_changed_paths, expected_changed_paths,
        "v0.18.4 generated scout changed paths drifted from guardrail expectation"
    );

    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths.len(),
        changed_mappings.len(),
        "v0.18.4 path mappings must not contain duplicate Python paths"
    );
    assert_eq!(
        mapped_changed_paths, generated_changed_paths,
        "v0.18.4 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        assert!(!python_path.is_empty(), "mapping has empty Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.4 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.18.4 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.18.4 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_package_or_gap_evidence = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.18.4 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(path),
                "v0.18.4 path mapping uses unknown Rust target path {path}"
            );
            if python_path.starts_with("packages/millrace-web/") {
                assert_ne!(
                    kind, "implementation",
                    "web package mapping must not authorize Rust web implementation expansion"
                );
            }
            has_package_or_gap_evidence |=
                kind == "package_evidence" || kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_package_or_gap_evidence,
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    let expected_mapping_surfaces = BTreeSet::from([
        "runner_failure_metadata_contracts",
        "blocked_metadata_persistence",
        "runtime_router_blocked_recovery_paths",
        "queue_retry_blocked_cli",
        "workspace_queue_blocked_requeue",
        "auto_recovery_config_status",
        "daemon_stranded_dependency_auto_recovery",
        "docs_operator_skill_and_release_evidence",
        "web_version_package_evidence",
        "release_validation_guardrails",
    ]);
    assert_eq!(covered_mapping_surfaces, expected_mapping_surfaces);

    let classifier = &fixture["failure_classifier_metadata"];
    for failure_class in [
        "network_unavailable",
        "provider_unavailable",
        "provider_rate_limited",
        "runner_timeout",
    ] {
        assert!(
            classifier["retryable_failure_classes"]
                .as_array()
                .expect("retryable classes are present")
                .iter()
                .any(|value| value.as_str() == Some(failure_class)),
            "missing v0.18.4 retryable failure class {failure_class}"
        );
    }
    for metadata_key in [
        "blocked_origin",
        "failure_scope",
        "auto_requeue_candidate",
        "failure_classifier_code",
    ] {
        assert!(
            classifier["metadata_keys"]
                .as_array()
                .expect("metadata keys are present")
                .iter()
                .any(|value| value.as_str() == Some(metadata_key)),
            "missing v0.18.4 runner failure metadata key {metadata_key}"
        );
    }

    let blocked = &fixture["blocked_metadata_contract"];
    for required_field in [
        "work_item_kind",
        "work_item_id",
        "root_spec_id",
        "root_idea_id",
        "blocked_at",
        "blocked_origin",
        "failure_class",
        "failure_scope",
        "auto_requeue_candidate",
        "source_run_id",
        "source_plane",
        "source_stage",
        "terminal_result",
        "stage_result_path",
        "stdout_path",
        "stderr_path",
    ] {
        assert!(
            blocked["required_fields"]
                .as_array()
                .expect("blocked metadata fields are present")
                .iter()
                .any(|value| value.as_str() == Some(required_field)),
            "missing v0.18.4 blocked metadata field {required_field}"
        );
    }
    assert_eq!(
        blocked["diagnostic_path_template"],
        "millrace-agents/diagnostics/blocked/task-<TASK_ID>.json"
    );

    let queue_retry = &fixture["queue_retry_behavior"];
    assert_eq!(
        queue_retry["command"],
        "millrace queue retry-blocked <TASK_ID>"
    );
    for required in [
        "safe work item id parsing",
        "live daemon lock refusal",
        "root spec guard",
        "retryability check",
        "retry budget check",
        "force override",
        "queue depth snapshot refresh",
    ] {
        assert!(
            queue_retry["required_guards"]
                .as_array()
                .expect("queue retry guards are present")
                .iter()
                .any(|value| value.as_str() == Some(required)),
            "missing v0.18.4 queue retry guard {required}"
        );
    }
    for audit_field in [
        "actor",
        "attempt_number",
        "auto",
        "destination_state",
        "failure_class",
        "reason",
        "source_state",
    ] {
        assert!(
            queue_retry["audit_fields"]
                .as_array()
                .expect("queue retry audit fields are present")
                .iter()
                .any(|value| value.as_str() == Some(audit_field)),
            "missing v0.18.4 queue retry audit field {audit_field}"
        );
    }

    let auto_recovery = &fixture["auto_recovery_contract"];
    assert_eq!(
        auto_recovery["default_policy"]["enabled"],
        Value::Bool(true)
    );
    assert_eq!(
        auto_recovery["default_policy"]["blocked_dependency_retry_enabled"],
        Value::Bool(true)
    );
    assert_eq!(
        auto_recovery["default_policy"]["max_auto_requeues_per_work_item"],
        Value::from(3)
    );
    assert_eq!(
        auto_recovery["default_policy"]["cooldown_seconds"],
        serde_json::json!([300, 900, 3600])
    );
    for skip_reason in [
        "disabled",
        "blocked_dependency_retry_disabled",
        "paused",
        "stop_requested",
        "active_runs_present",
        "no_queued_execution_dependents",
        "blocked_dependency_not_retryable",
        "retry_budget_exhausted",
        "cooldown_active",
        "missing_or_invalid_metadata",
        "root_spec_mismatch",
    ] {
        assert!(
            auto_recovery["skip_reasons"]
                .as_array()
                .expect("auto recovery skip reasons are present")
                .iter()
                .any(|value| value.as_str() == Some(skip_reason)),
            "missing v0.18.4 auto recovery skip reason {skip_reason}"
        );
    }

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runners_normalization.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.18.4 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.18.4 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.18.4 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.18.3..v0.18.4",
        "cargo fmt --check",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo test --test runners_normalization",
        "cargo test --test workspace_queue_state_stores",
        "cargo test --test runtime_daemon",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.4 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_4_version_package_evidence"]["python_target_tag"],
        "v0.18.4"
    );
    assert_eq!(
        dashboard_fixture["v0_18_4_version_package_evidence"]["diff_range"],
        "v0.18.3..v0.18.4"
    );
    let web_sources =
        dashboard_fixture["v0_18_4_version_package_evidence"]["changed_python_sources"]
            .as_array()
            .expect("v0.18.4 web package source list");
    for source_path in [
        "../millrace-py/packages/millrace-web/CHANGELOG.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            web_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.4 web package evidence source {source_path}"
        );
    }
    assert_eq!(
        dashboard_fixture["v0_18_4_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.4 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no remote skill installation",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.4 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_18_6_parity_fixture_covers_operator_intervention_durable_idea_source_and_web_guardrails()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_6_parity_evidence.json")
            .expect("read v0.18.6 auto-port parity evidence fixture"),
    )
    .expect("parse v0.18.6 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_6_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.4");
    assert_eq!(
        fixture["python_reference"]["previous_peeled_commit"],
        "516e947e90155b6436dbc9efcf932254f34bc39c"
    );
    assert_eq!(fixture["python_reference"]["intermediate_tag"], "v0.18.5");
    assert_eq!(
        fixture["python_reference"]["intermediate_peeled_commit"],
        "51374def7e9ea8225f52d95d25abc2fd43f85c9a"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.6");
    assert_eq!(
        fixture["python_reference"]["target_tag_object"],
        "85d91683f3be3dfa6f2983d3e397ed373f12edba"
    );
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "63e623bc6fcfcf74ae0cc2ce5605a12ae4179873"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.4..v0.18.6"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.4"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.3.5");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.18.6 parity fixture still treats Rust 0.3.4 as the planned target"
    );

    let state_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("millrace-agents/auto-port/state.json");
    let state: Value = serde_json::from_str(
        &fs::read_to_string(state_path).expect("read v0.18.6 auto-port state"),
    )
    .expect("parse v0.18.6 auto-port state");
    assert_ne!(
        state["active_python_version"], fixture["python_reference"]["previous_tag"],
        "auto-port state still treats Python v0.18.4 as current"
    );
    assert_ne!(
        state["active_rust_version"], fixture["rust_reference"]["current_repo_crate_version"],
        "auto-port state still treats Rust 0.3.4 as the target"
    );

    let required_surface_names: BTreeSet<_> = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present")
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    assert_eq!(
        required_surface_names,
        BTreeSet::from([
            "mailbox_intervention_contracts",
            "operator_intervention_archive_audit",
            "cli_runtime_control_interventions",
            "daemon_readonly_intervention_surfaces",
            "durable_idea_source_closure_recovery",
            "docs_operator_skill_release_evidence",
            "web_version_package_evidence",
            "source_reference_guardrails",
            "release_validation_guardrails",
        ])
    );

    let required_axis_names: BTreeSet<_> = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present")
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "python_intermediate_release_pin",
        "rust_version_transition_pin",
        "auto_port_state_currentness",
        "generated_scout_changed_paths",
        "mailbox_command_enum_values",
        "mailbox_payload_validation",
        "intervention_payload_exports",
        "direct_runtime_control_interventions",
        "daemon_mailbox_intervention_routing",
        "safe_active_mutation_deferral",
        "intervention_archive_paths",
        "intervention_audit_jsonl_schema",
        "intervention_runtime_events",
        "queue_depth_snapshot_refresh",
        "queue_cancel_supersede_retarget_cli",
        "incident_intervention_cli",
        "queue_archive_inspection",
        "status_latest_intervention",
        "durable_idea_source_artifacts",
        "watcher_source_reference_order",
        "closure_target_source_preference",
        "missing_root_idea_source_block",
        "docs_source_refs",
        "operator_skill_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.18.6 parity coverage axis {axis}"
        );
    }

    let allowed_rust_targets = BTreeSet::from([
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/millrace-arbiter-and-completion-behavior.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-workspace-baselines-and-upgrades.md",
        "millrace-agents/outline.md",
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.4-to-v0.18.6-rust-0.3.5.md",
        "millrace-agents/auto-port/state.json",
        "src/lib.rs",
        "src/contracts/enums.rs",
        "src/contracts/mod.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/work_documents.rs",
        "src/workspace.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/runtime_control.rs",
        "src/workspace/state_store.rs",
        "src/workspace/task_lifecycle_integrity.rs",
        "src/runtime/mod.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/runtime/monitor.rs",
        "src/cli/mod.rs",
        "src/cli/intake.rs",
        "src/cli/parser.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "tests/contracts_public_exports.rs",
        "tests/contracts_runtime_json.rs",
        "tests/workspace_runtime_control.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
        "tests/workspace_init_parity.rs",
        "tests/parity_cli.rs",
        "tests/support/parity.rs",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/cli_parity/auto_port_v0_18_6_parity_evidence.json",
        "tests/fixtures/cli_parity/auto_port_v0_18_6_release_parity_evidence.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/runtime_json/README.md",
        "tests/fixtures/runtime_json/auto_port_v0_18_6_runtime_contract_scout.json",
    ]);

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.6 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.18.6 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            let target_path = target.as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(target_path),
                "v0.18.6 source surface references unknown Rust target path {target_path}"
            );
            assert_repo_relative_target_exists(target_path, "v0.18.6 source-reference surface");
            referenced_targets.insert(target_path.to_owned());
        }
    }
    for axis in [
        "mailbox_command_enum_values",
        "mailbox_payload_validation",
        "intervention_payload_exports",
        "direct_runtime_control_interventions",
        "daemon_mailbox_intervention_routing",
        "safe_active_mutation_deferral",
        "intervention_archive_paths",
        "intervention_audit_jsonl_schema",
        "intervention_runtime_events",
        "queue_depth_snapshot_refresh",
        "queue_cancel_supersede_retarget_cli",
        "incident_intervention_cli",
        "queue_archive_inspection",
        "status_latest_intervention",
        "durable_idea_source_artifacts",
        "watcher_source_reference_order",
        "closure_target_source_preference",
        "missing_root_idea_source_block",
        "docs_source_refs",
        "operator_skill_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.18.6 axis {axis}"
        );
    }

    let expected_changed_paths = BTreeSet::from([
        "CHANGELOG.md",
        "README.md",
        "docs/runtime/millrace-arbiter-and-completion-behavior.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/skills/millrace-ops-agent-manual/SKILL.md",
        "docs/source-package-map.md",
        "packages/millrace-web/CHANGELOG.md",
        "packages/millrace-web/pyproject.toml",
        "packages/millrace-web/src/millrace_web/__init__.py",
        "packages/millrace-web/src/millrace_web/app.py",
        "src/millrace_ai/__init__.py",
        "src/millrace_ai/cli/app.py",
        "src/millrace_ai/cli/commands/incident.py",
        "src/millrace_ai/cli/commands/queue.py",
        "src/millrace_ai/cli/shared.py",
        "src/millrace_ai/cli/status_view.py",
        "src/millrace_ai/contracts/__init__.py",
        "src/millrace_ai/contracts/enums.py",
        "src/millrace_ai/contracts/mailbox.py",
        "src/millrace_ai/runtime/completion_behavior.py",
        "src/millrace_ai/runtime/control.py",
        "src/millrace_ai/runtime/control_mutations.py",
        "src/millrace_ai/runtime/mailbox_intake.py",
        "src/millrace_ai/runtime/watcher_intake.py",
        "src/millrace_ai/workspace/idea_sources.py",
        "src/millrace_ai/workspace/operator_interventions.py",
        "src/millrace_ai/workspace/paths.py",
        "src/millrace_ai/workspace/queue_store.py",
        "tests/cli/test_cli.py",
        "tests/integration/test_operator_intervention_cleanup.py",
        "tests/runtime/test_completion_behavior.py",
        "tests/runtime/test_control.py",
        "tests/runtime/test_runtime.py",
        "tests/workspace/test_operator_interventions.py",
    ]);

    let generated_scout_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        fixture["python_reference"]["generated_scout"]
            .as_str()
            .expect("generated scout path"),
    );
    let generated_scout =
        fs::read_to_string(generated_scout_path).expect("read v0.18.6 generated scout");
    let mut generated_changed_paths = BTreeSet::new();
    let mut in_changed_paths = false;
    for line in generated_scout.lines() {
        if line == "Changed Python paths:" {
            in_changed_paths = true;
            continue;
        }
        if !in_changed_paths {
            continue;
        }
        if line == "Diff stat:" {
            break;
        }
        if let Some(path) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            generated_changed_paths.insert(path);
        }
    }
    assert_eq!(
        generated_changed_paths, expected_changed_paths,
        "v0.18.6 generated scout changed paths drifted from guardrail expectation"
    );

    for source_path in expected_changed_paths
        .iter()
        .map(|path| format!("../millrace-py/{path}"))
    {
        assert!(
            referenced_sources.contains(&source_path),
            "missing v0.18.6 Python source reference {source_path}"
        );
    }
    for target_path in [
        "src/contracts/enums.rs",
        "src/contracts/runtime_json.rs",
        "src/workspace/queue_store.rs",
        "src/workspace/runtime_control.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/cli/mod.rs",
        "src/cli/parser.rs",
        "src/cli/read_only.rs",
        "tests/contracts_runtime_json.rs",
        "tests/workspace_runtime_control.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
        "tests/fixtures/runtime_json/auto_port_v0_18_6_runtime_contract_scout.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.18.6 expected Rust target {target_path}"
        );
    }

    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths.len(),
        changed_mappings.len(),
        "v0.18.6 path mappings must not contain duplicate Python paths"
    );
    assert_eq!(
        mapped_changed_paths, generated_changed_paths,
        "v0.18.6 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.6 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.18.6 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.18.6 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_package_or_gap_evidence = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.18.6 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(path),
                "v0.18.6 path mapping uses unknown Rust target path {path}"
            );
            if !is_evidence_only_target_kind(kind) {
                assert_repo_relative_target_exists(path, "v0.18.6 changed-path mapping");
            }
            if python_path.starts_with("packages/millrace-web/") {
                assert_ne!(
                    kind, "implementation",
                    "web package mapping must not authorize Rust web implementation expansion"
                );
            }
            has_package_or_gap_evidence |=
                kind == "package_evidence" || kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_package_or_gap_evidence,
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    assert_eq!(
        covered_mapping_surfaces,
        BTreeSet::from([
            "mailbox_intervention_contracts",
            "operator_intervention_archive_audit",
            "cli_runtime_control_interventions",
            "daemon_readonly_intervention_surfaces",
            "durable_idea_source_closure_recovery",
            "docs_operator_skill_release_evidence",
            "web_version_package_evidence",
            "release_validation_guardrails",
        ])
    );

    let mailbox = &fixture["mailbox_intervention_contract"];
    for command in [
        "cancel_work_item",
        "archive_blocked_task",
        "supersede_task",
        "retarget_task_dependency",
        "resolve_incident",
        "cancel_incident",
        "archive_invalid_incident",
    ] {
        assert!(
            mailbox["command_values"]
                .as_array()
                .expect("mailbox command values are present")
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.6 mailbox command {command}"
        );
    }

    let intervention = &fixture["operator_intervention_contract"];
    for field in [
        "action",
        "actor",
        "reason",
        "issued_at",
        "applied_at",
        "work_item_kind",
        "work_item_id",
        "source_state",
        "destination_state",
        "source_path",
        "destination_path",
        "replacement_work_item_id",
        "affected_dependents",
    ] {
        assert!(
            intervention["record_fields"]
                .as_array()
                .expect("operator intervention record fields are present")
                .iter()
                .any(|value| value.as_str() == Some(field)),
            "missing v0.18.6 operator intervention record field {field}"
        );
    }
    for event_type in [
        "work_item_cancelled",
        "blocked_task_archived",
        "task_superseded",
        "task_dependency_retargeted",
        "incident_resolved_by_operator",
        "incident_cancelled",
        "invalid_incident_artifact_archived",
        "mailbox_operator_intervention_applied",
        "operator_intervention_deferred",
    ] {
        assert!(
            intervention["event_types"]
                .as_array()
                .expect("operator intervention event types are present")
                .iter()
                .any(|value| value.as_str() == Some(event_type)),
            "missing v0.18.6 operator intervention event {event_type}"
        );
    }

    let cli_intervention = &fixture["cli_intervention_contract"];
    for command in [
        "queue cancel",
        "queue archive-blocked",
        "queue supersede",
        "queue retarget-dependency",
        "incident resolve",
        "incident cancel",
        "incident archive-invalid",
    ] {
        assert!(
            cli_intervention["commands"]
                .as_array()
                .expect("CLI intervention commands are present")
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.6 CLI intervention command {command}"
        );
    }
    for key in [
        "action",
        "mode",
        "applied",
        "detail",
        "command_id",
        "mailbox_path",
        "artifact_path",
    ] {
        assert!(
            cli_intervention["control_result_output_keys"]
                .as_array()
                .expect("CLI control result output keys are present")
                .iter()
                .any(|value| value.as_str() == Some(key)),
            "missing v0.18.6 CLI control result key {key}"
        );
    }
    for guard in [
        "direct success",
        "mailbox-routed daemon ownership",
        "active-stage direct deferral",
        "empty reason rejection",
        "unsafe id rejection",
        "ambiguous cancellation requires --kind",
        "replacement existence",
        "cascade retarget",
        "cascade cancel",
        "absent dependency rejection",
        "invalid incident filename rejection",
    ] {
        assert!(
            cli_intervention["validation_guards"]
                .as_array()
                .expect("CLI validation guards are present")
                .iter()
                .any(|value| value.as_str() == Some(guard)),
            "missing v0.18.6 CLI intervention validation guard {guard}"
        );
    }

    let durable = &fixture["durable_idea_source_contract"];
    assert_eq!(
        durable["durable_source_path_template"],
        "millrace-agents/intake/ideas/<root_idea_id>.md"
    );
    assert_eq!(
        durable["missing_source_failure_class"],
        "missing_root_idea_source"
    );
    assert_eq!(durable["missing_source_event"], "root_idea_source_missing");

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/workspace_runtime_control.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.18.6 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.18.6 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.18.6 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.18.4..v0.18.6",
        "cargo fmt --check",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo test --test workspace_runtime_control",
        "cargo test --test workspace_queue_state_stores",
        "cargo test --test runtime_daemon",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.6 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_5_version_package_evidence"]["python_target_tag"],
        "v0.18.5"
    );
    assert_eq!(
        dashboard_fixture["v0_18_6_version_package_evidence"]["python_target_tag"],
        "v0.18.6"
    );
    assert_eq!(
        dashboard_fixture["v0_18_6_version_package_evidence"]["diff_range"],
        "v0.18.4..v0.18.6"
    );
    let web_sources =
        dashboard_fixture["v0_18_6_version_package_evidence"]["changed_python_sources"]
            .as_array()
            .expect("v0.18.6 web package source list");
    for source_path in [
        "../millrace-py/packages/millrace-web/CHANGELOG.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            web_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.18.6 web package evidence source {source_path}"
        );
    }
    assert_eq!(
        dashboard_fixture["v0_18_6_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.6 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Python execution beyond checked-out ../millrace-py diff inspection",
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no remote skill installation",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.18.6 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_19_0_parity_fixture_covers_capability_governance_release_guardrails() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_19_0_parity_evidence.json")
            .expect("read v0.19.0 auto-port parity evidence fixture"),
    )
    .expect("parse v0.19.0 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_19_0_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.6");
    assert_eq!(
        fixture["python_reference"]["previous_tag_object"],
        "85d91683f3be3dfa6f2983d3e397ed373f12edba"
    );
    assert_eq!(
        fixture["python_reference"]["previous_peeled_commit"],
        "63e623bc6fcfcf74ae0cc2ce5605a12ae4179873"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.19.0");
    assert_eq!(
        fixture["python_reference"]["target_tag_object"],
        "11c45b03428226f04f56fe078e083bea2464e6b0"
    );
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "efb9c5881f524d23dcb78aecfc96fdf7cda9d26f"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.6..v0.19.0"
    );
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.3.5"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.4.0");
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.19.0 parity fixture still treats Rust 0.3.5 as the planned target"
    );

    let state_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("millrace-agents/auto-port/state.json");
    let state: Value = serde_json::from_str(
        &fs::read_to_string(state_path).expect("read v0.19.0 auto-port state"),
    )
    .expect("parse v0.19.0 auto-port state");
    assert_ne!(
        state["active_python_version"], fixture["python_reference"]["previous_tag"],
        "auto-port state still treats Python v0.18.6 as current"
    );
    assert_ne!(
        state["active_rust_version"], fixture["rust_reference"]["current_repo_crate_version"],
        "auto-port state still treats Rust 0.3.5 as the target"
    );

    let required_surface_names: BTreeSet<_> = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present")
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    assert_eq!(
        required_surface_names,
        BTreeSet::from([
            "execution_capability_contracts",
            "execution_capability_config",
            "compiled_capability_grants",
            "runtime_capability_gates",
            "approval_cli_runtime_control",
            "runner_support_and_evidence",
            "inspection_prompt_artifacts",
            "docs_operator_skill_release_evidence",
            "web_version_package_evidence",
            "source_reference_guardrails",
            "release_validation_guardrails",
        ])
    );

    let required_axis_names: BTreeSet<_> = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present")
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "auto_port_state_currentness",
        "generated_scout_changed_paths",
        "capability_contract_models",
        "capability_id_aliases",
        "capability_scope_validation",
        "capability_grant_fingerprints",
        "mailbox_approval_command_values",
        "approval_payload_validation",
        "execution_capability_config_keys",
        "recompile_boundary_config_keys",
        "default_grants",
        "policy_override_resolution",
        "grant_summaries",
        "strict_required_advisory_failure",
        "compiled_plan_grants",
        "stage_request_capability_context",
        "prompt_context_capability_context",
        "capability_gate_artifacts",
        "capability_gate_events",
        "gate_failure_classes",
        "approval_storage_pending_resolved",
        "approval_cli_commands",
        "runtime_control_approval_routing",
        "daemon_mailbox_approval_processing",
        "runner_support_decisions",
        "runner_artifact_capability_metadata",
        "missing_capability_evidence_normalization",
        "runs_show_capability_output",
        "config_show_execution_capabilities",
        "compile_show_execution_capabilities",
        "docs_source_refs",
        "operator_skill_source_refs",
        "web_version_package_evidence",
        "expected_rust_target_mapping",
        "stale_test_reference_guardrails",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.19.0 parity coverage axis {axis}"
        );
    }

    let allowed_rust_targets = BTreeSet::from([
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-execution-capabilities.md",
        "docs/runtime/millrace-runner-architecture.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.6-to-v0.19.0-rust-0.4.0.md",
        "millrace-agents/auto-port/state.json",
        "src/lib.rs",
        "src/contracts/mod.rs",
        "src/contracts/enums.rs",
        "src/contracts/runtime_json.rs",
        "src/contracts/capabilities.rs",
        "src/compiler/contracts.rs",
        "src/compiler/materialization.rs",
        "src/compiler/graph_exports.rs",
        "src/compiler/persistence.rs",
        "src/runtime/startup.rs",
        "src/runtime/supervisor.rs",
        "src/runtime/tick.rs",
        "src/runtime/approvals.rs",
        "src/runtime/capability_gates.rs",
        "src/workspace/runtime_control.rs",
        "src/cli/mod.rs",
        "src/cli/parser.rs",
        "src/cli/read_only.rs",
        "src/cli/render.rs",
        "src/runners/contracts.rs",
        "src/runners/dispatcher.rs",
        "src/runners/normalization.rs",
        "src/runners/prompting.rs",
        "src/runners/artifacts.rs",
        "src/runners/codex_cli.rs",
        "src/runners/pi_rpc.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/contracts_capabilities.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_persistence.rs",
        "tests/compiler_capability_grants.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_capability_gates.rs",
        "tests/runners_normalization.rs",
        "tests/runners_codex_cli.rs",
        "tests/runners_pi_rpc.rs",
        "tests/runners_capability_support.rs",
        "tests/parity_cli.rs",
        "tests/workspace_runtime_control.rs",
        "tests/fixtures/cli_parity/auto_port_v0_19_0_parity_evidence.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "tests/fixtures/runtime_json/auto_port_v0_19_0_runtime_contract_scout.json",
    ]);
    let planned_new_rust_targets = BTreeSet::from([
        "docs/runtime/millrace-execution-capabilities.md",
        "docs/runtime/millrace-runner-architecture.md",
        "src/contracts/capabilities.rs",
        "src/runtime/approvals.rs",
        "src/runtime/capability_gates.rs",
        "tests/contracts_capabilities.rs",
        "tests/compiler_capability_grants.rs",
        "tests/runtime_capability_gates.rs",
        "tests/runners_capability_support.rs",
    ]);

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.19.0 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.19.0 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            let target_path = target.as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(target_path),
                "v0.19.0 source surface references unknown Rust target path {target_path}"
            );
            if !planned_new_rust_targets.contains(target_path) {
                assert_repo_relative_target_exists(target_path, "v0.19.0 source-reference surface");
            }
            referenced_targets.insert(target_path.to_owned());
        }
    }
    for axis in &required_axis_names {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.19.0 axis {axis}"
        );
    }

    let generated_scout_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        fixture["python_reference"]["generated_scout"]
            .as_str()
            .expect("generated scout path"),
    );
    let generated_scout =
        fs::read_to_string(generated_scout_path).expect("read v0.19.0 generated scout");
    let mut generated_changed_paths = BTreeSet::new();
    let mut in_changed_paths = false;
    for line in generated_scout.lines() {
        if line == "Changed Python paths:" {
            in_changed_paths = true;
            continue;
        }
        if !in_changed_paths {
            continue;
        }
        if line == "Diff stat:" {
            break;
        }
        if let Some(path) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            generated_changed_paths.insert(path);
        }
    }
    assert_eq!(
        generated_changed_paths.len(),
        61,
        "v0.19.0 generated scout should pin the 61-path Python diff"
    );

    for source_path in generated_changed_paths
        .iter()
        .map(|path| format!("../millrace-py/{path}"))
    {
        assert!(
            referenced_sources.contains(&source_path),
            "missing v0.19.0 Python source reference {source_path}"
        );
    }
    for target_path in [
        "src/contracts/capabilities.rs",
        "src/contracts/runtime_json.rs",
        "src/compiler/materialization.rs",
        "src/runtime/capability_gates.rs",
        "src/runtime/approvals.rs",
        "src/workspace/runtime_control.rs",
        "src/runners/normalization.rs",
        "src/runners/artifacts.rs",
        "src/cli/read_only.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/runners_normalization.rs",
        "tests/fixtures/runtime_json/auto_port_v0_19_0_runtime_contract_scout.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.19.0 expected Rust target {target_path}"
        );
    }

    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| mapping["python_path"].as_str().expect("Python path"))
        .collect();
    assert_eq!(
        mapped_changed_paths.len(),
        changed_mappings.len(),
        "v0.19.0 path mappings must not contain duplicate Python paths"
    );
    assert_eq!(
        mapped_changed_paths, generated_changed_paths,
        "v0.19.0 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.19.0 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.19.0 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.19.0 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_package_or_gap_evidence = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.19.0 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                allowed_rust_targets.contains(path),
                "v0.19.0 path mapping uses unknown Rust target path {path}"
            );
            if !is_evidence_only_target_kind(kind) && !planned_new_rust_targets.contains(path) {
                assert_repo_relative_target_exists(path, "v0.19.0 changed-path mapping");
            }
            if python_path.starts_with("packages/millrace-web/") {
                assert_ne!(
                    kind, "implementation",
                    "web package mapping must not authorize Rust web implementation expansion"
                );
            }
            has_package_or_gap_evidence |=
                kind == "package_evidence" || kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_package_or_gap_evidence,
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    for surface in [
        "execution_capability_contracts",
        "execution_capability_config",
        "compiled_capability_grants",
        "runtime_capability_gates",
        "approval_cli_runtime_control",
        "runner_support_and_evidence",
        "inspection_prompt_artifacts",
        "docs_operator_skill_release_evidence",
        "web_version_package_evidence",
        "release_validation_guardrails",
    ] {
        assert!(
            covered_mapping_surfaces.contains(surface),
            "changed-path mappings do not cover v0.19.0 surface {surface}"
        );
    }

    let capability_contract = &fixture["execution_capability_contract"];
    for capability_id in [
        "runner.invoke",
        "workspace.read",
        "workspace.write",
        "artifact.write",
        "shell.run",
        "git.mutate",
        "package.install",
        "network.access",
        "approval.request",
        "evidence.emit",
        "runtime.control",
    ] {
        assert!(
            capability_contract["base_capability_ids"]
                .as_array()
                .expect("base capability ids are present")
                .iter()
                .any(|value| value.as_str() == Some(capability_id)),
            "missing v0.19.0 base capability id {capability_id}"
        );
    }
    assert_eq!(
        capability_contract["capability_key_aliases"]["package_install"],
        "package.install"
    );
    assert_eq!(
        fixture["execution_capability_config"]["default_decisions"]["network.access"],
        "deny"
    );
    assert_eq!(
        fixture["execution_capability_config"]["default_decisions"]["package.install"],
        "approval_required"
    );
    assert_eq!(
        fixture["approval_contract"]["mailbox_commands"],
        json!(["approve_execution_capability", "deny_execution_capability"])
    );
    assert_eq!(
        fixture["capability_gate_contract"]["event_type"],
        "capability_gate_evaluated"
    );
    assert!(
        fixture["capability_gate_contract"]["failure_classes"]
            .as_array()
            .expect("capability gate failure classes are present")
            .iter()
            .any(|value| value.as_str() == Some("capability_evidence_missing")),
        "missing v0.19.0 capability evidence failure class"
    );
    assert!(
        fixture["runner_support_contract"]["artifact_fields"]
            .as_array()
            .expect("runner artifact fields are present")
            .iter()
            .any(|value| value.as_str() == Some("missing_capability_evidence_refs")),
        "missing v0.19.0 runner missing evidence metadata field"
    );

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/runners_normalization.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.19.0 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.19.0 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.19.0 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.18.6..v0.19.0",
        "cargo fmt --check",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo test --test contracts_public_exports",
        "cargo test --test compiler_contracts",
        "cargo test --test compiler_materialization",
        "cargo test --test runtime_serial",
        "cargo test --test runtime_daemon",
        "cargo test --test runners_normalization",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.19.0 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_19_0_version_package_evidence"]["python_target_tag"],
        "v0.19.0"
    );
    assert_eq!(
        dashboard_fixture["v0_19_0_version_package_evidence"]["diff_range"],
        "v0.18.6..v0.19.0"
    );
    assert_eq!(
        dashboard_fixture["v0_19_0_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.19.0 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, dashboard HTTP API, or separate dashboard package is added."
    );

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Python execution beyond checked-out ../millrace-py diff inspection",
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no remote skill installation",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.19.0 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_20_0_parity_fixture_covers_workflow_blueprint_release_guardrails() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_20_0_parity_evidence.json")
            .expect("read v0.20.0 auto-port parity evidence fixture"),
    )
    .expect("parse v0.20.0 auto-port parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_20_0_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.19.0");
    assert_eq!(
        fixture["python_reference"]["previous_tag_object"],
        "11c45b03428226f04f56fe078e083bea2464e6b0"
    );
    assert_eq!(
        fixture["python_reference"]["previous_peeled_commit"],
        "efb9c5881f524d23dcb78aecfc96fdf7cda9d26f"
    );
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.20.0");
    assert_eq!(
        fixture["python_reference"]["target_tag_object"],
        "25d86f0c560d60d66039611e34df2737a64bebe3"
    );
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "c432786242e9e7cf9f7262ec0ec4f906f4bb7bf7"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.19.0..v0.20.0"
    );
    assert_eq!(fixture["python_reference"]["changed_path_count"], 249);
    assert_eq!(
        fixture["rust_reference"]["current_repo_crate_version"],
        "0.4.0"
    );
    assert_eq!(fixture["rust_reference"]["planned_crate_version"], "0.5.0");
    assert_ne!(
        fixture["python_reference"]["target_tag"], "v0.19.0",
        "v0.20.0 guardrail fixture still treats Python v0.19.0 as current"
    );
    assert_ne!(
        fixture["rust_reference"]["planned_crate_version"],
        fixture["rust_reference"]["current_repo_crate_version"],
        "v0.20.0 guardrail fixture still treats Rust 0.4.0 as the planned target"
    );

    let state_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("millrace-agents/auto-port/state.json");
    let state: Value = serde_json::from_str(
        &fs::read_to_string(state_path).expect("read v0.20.0 auto-port state"),
    )
    .expect("parse v0.20.0 auto-port state");
    assert_eq!(state["last_seen_python_version"], "v0.20.0");
    assert_eq!(state["active_python_version"], "v0.20.0");
    assert_eq!(state["last_ported_python_version"], "v0.19.0");
    assert_eq!(state["last_released_rust_version"], "0.4.0");
    assert_eq!(state["active_rust_version"], "0.5.0");

    let required_surface_names: BTreeSet<_> = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present")
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    assert_eq!(
        required_surface_names,
        BTreeSet::from([
            "workflow_primitive_contracts_assets",
            "compiler_authority_validation",
            "workspace_schema_lifecycle",
            "lanes_request_context_inspection",
            "runtime_effects_failure_policy",
            "blueprint_planning_runtime",
            "cli_status_run_once",
            "docs_operator_skill_release_evidence",
            "web_version_package_evidence",
            "test_guardrail_evidence",
            "source_reference_guardrails",
            "release_validation_guardrails",
        ])
    );

    let required_axis_names: BTreeSet<_> = fixture["required_coverage_axes"]
        .as_array()
        .expect("required coverage axes are present")
        .iter()
        .map(|axis| axis.as_str().expect("coverage axis"))
        .collect();
    for axis in [
        "python_diff_pin",
        "rust_version_transition_pin",
        "auto_port_state_currentness",
        "generated_scout_changed_paths",
        "live_python_diff_match",
        "source_reference_coverage",
        "expected_rust_target_mapping",
        "stale_test_reference_guardrails",
        "changed_path_metadata_validation",
        "workflow_primitive_contract_models",
        "workflow_primitive_asset_registry",
        "compiler_authority_fields",
        "primitive_fingerprint_persistence",
        "schema_epoch_reset_archive",
        "generic_work_item_lifecycle",
        "scheduler_lane_dispatch",
        "lane_conflict_policy",
        "request_context_bundle_artifacts",
        "runtime_effect_dispatch",
        "runtime_failure_policy_routing",
        "blueprint_graph_modes_assets",
        "blueprint_contracts_state",
        "blueprint_runtime_effects",
        "planner_disposition_semantics",
        "cli_run_once_removed",
        "daemon_max_ticks_one_replacement",
        "web_version_package_evidence",
        "release_check_evidence",
        "no_live_external_dependencies",
    ] {
        assert!(
            required_axis_names.contains(axis),
            "missing v0.20.0 parity coverage axis {axis}"
        );
    }

    let planned_new_rust_targets = BTreeSet::from([
        "src/assets/baseline/entrypoints/planning/contractor_blueprint.md",
        "src/assets/baseline/entrypoints/planning/evaluator_blueprint.md",
        "src/assets/baseline/entrypoints/planning/manager_blueprint.md",
        "src/assets/baseline/entrypoints/planning/mechanic_blueprint.md",
        "src/assets/baseline/graphs/planning/blueprint.json",
        "src/assets/baseline/modes/blueprint_codex.json",
        "src/assets/baseline/modes/blueprint_learning_codex.json",
        "src/assets/baseline/registry/artifact_contracts/default_artifact_contracts.json",
        "src/assets/baseline/registry/document_adapters/blueprint_draft_markdown_v1.json",
        "src/assets/baseline/registry/document_adapters/builtin_markdown_v1.json",
        "src/assets/baseline/registry/lifecycle_mutation_plans/default_lifecycle_mutations.json",
        "src/assets/baseline/registry/queue_claim_policies/default_queue_claim_policies.json",
        "src/assets/baseline/registry/recovery_policies/default_recovery_policies.json",
        "src/assets/baseline/registry/request_context_profiles/default_request_context_profiles.json",
        "src/assets/baseline/registry/runtime_effect_handlers/default_effect_handlers.json",
        "src/assets/baseline/registry/runtime_effect_rules/blueprint_effect_rules.json",
        "src/assets/baseline/registry/runtime_effect_rules/planner_effect_rules.json",
        "src/assets/baseline/registry/runtime_failure_policies/default_runtime_failure_policies.json",
        "src/assets/baseline/registry/stage_kinds/planning/contractor_blueprint.json",
        "src/assets/baseline/registry/stage_kinds/planning/evaluator_blueprint.json",
        "src/assets/baseline/registry/stage_kinds/planning/manager_blueprint.json",
        "src/assets/baseline/registry/stage_kinds/planning/mechanic_blueprint.json",
        "src/assets/baseline/registry/terminal_actions/default_terminal_actions.json",
        "src/assets/baseline/registry/work_item_families/blueprint_draft.json",
        "src/assets/baseline/registry/work_item_families/incident.json",
        "src/assets/baseline/registry/work_item_families/learning_request.json",
        "src/assets/baseline/registry/work_item_families/probe.json",
        "src/assets/baseline/registry/work_item_families/spec.json",
        "src/assets/baseline/registry/work_item_families/task.json",
        "src/assets/baseline/registry/workspace_schema_epochs/current.json",
        "src/assets/baseline/skills/stage/planning/contractor-blueprint-core/SKILL.md",
        "src/assets/baseline/skills/stage/planning/evaluator-blueprint-core/SKILL.md",
        "src/assets/baseline/skills/stage/planning/manager-blueprint-core/SKILL.md",
        "src/assets/baseline/skills/stage/planning/mechanic-blueprint-core/SKILL.md",
        "src/compiler/workflow_primitives.rs",
        "src/contracts/blueprint.rs",
        "src/contracts/workflow_primitives.rs",
        "src/runtime/blueprint_effects.rs",
        "src/runtime/effects.rs",
        "src/runtime/failure_policy.rs",
        "src/runtime/lanes.rs",
        "src/runtime/lifecycle.rs",
        "src/runtime/request_context.rs",
        "src/runtime/stage_requests.rs",
        "src/workspace/blueprint_state.rs",
        "src/workspace/queue_lifecycle.rs",
        "src/workspace/schema_epoch.rs",
        "src/workspace/work_item_adapters.rs",
        "tests/blueprint_contracts.rs",
        "tests/blueprint_effects.rs",
        "tests/blueprint_planning_loop.rs",
        "tests/compiler_workflow_primitives.rs",
        "tests/contracts_workflow_primitives.rs",
        "tests/runtime_effects.rs",
        "tests/runtime_failure_policy.rs",
        "tests/runtime_lanes.rs",
        "tests/runtime_request_context.rs",
        "tests/runtime_run_inspection.rs",
        "tests/workspace_schema_epoch.rs",
        "tests/workspace_work_item_adapters.rs",
    ]);

    let mut referenced_sources = BTreeSet::new();
    let mut referenced_targets = BTreeSet::new();
    let mut covered_axes = BTreeSet::new();
    for surface in fixture["source_reference_surfaces"]
        .as_array()
        .expect("source reference surfaces are present")
    {
        let surface_name = surface["surface"].as_str().expect("source surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.20.0 source surface references unknown surface {surface_name}"
        );
        for axis in surface["coverage"].as_array().expect("coverage array") {
            let axis = axis.as_str().expect("coverage axis");
            assert!(
                required_axis_names.contains(axis),
                "v0.20.0 source surface references unknown coverage axis {axis}"
            );
            covered_axes.insert(axis);
        }
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_sources.insert(source.as_str().expect("Python source path").to_owned());
        }
        for target in surface["expected_rust_targets"]
            .as_array()
            .expect("expected_rust_targets array")
        {
            let target_path = target.as_str().expect("Rust target path");
            assert!(
                !target_path.is_empty()
                    && !target_path.contains('\\')
                    && !Path::new(target_path).is_absolute()
                    && !target_path.split('/').any(|segment| segment == ".."),
                "v0.20.0 source-reference surface references invalid Rust target path {target_path}"
            );
            if !planned_new_rust_targets.contains(target_path) {
                assert_repo_relative_target_exists(target_path, "v0.20.0 source-reference surface");
            }
            referenced_targets.insert(target_path.to_owned());
        }
    }
    for axis in &required_axis_names {
        assert!(
            covered_axes.contains(axis),
            "source-reference surfaces do not cover v0.20.0 axis {axis}"
        );
    }

    for source_path in [
        "../millrace-py/src/millrace_ai/architecture/workflow_primitives.py",
        "../millrace-py/src/millrace_ai/assets/registry/artifact_contracts/default_artifact_contracts.json",
        "../millrace-py/src/millrace_ai/assets/registry/document_adapters/blueprint_draft_markdown_v1.json",
        "../millrace-py/src/millrace_ai/assets/graphs/planning/blueprint.json",
        "../millrace-py/src/millrace_ai/assets/modes/blueprint_codex.json",
        "../millrace-py/src/millrace_ai/runtime/request_context.py",
        "../millrace-py/src/millrace_ai/runtime/lanes.py",
        "../millrace-py/src/millrace_ai/runtime/effects.py",
        "../millrace-py/src/millrace_ai/runtime/blueprint_effects.py",
        "../millrace-py/src/millrace_ai/workspace/schema_epoch.py",
        "../millrace-py/src/millrace_ai/cli/commands/run.py",
        "../millrace-py/docs/runtime/millrace-blueprint-planning.md",
        "../millrace-py/docs/skills/millrace-ops-agent-manual/SKILL.md",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
        "../millrace-py/tests/architecture/test_workflow_primitives.py",
        "../millrace-py/tests/blueprint/test_effects.py",
    ] {
        assert!(
            referenced_sources.contains(source_path),
            "missing v0.20.0 source reference {source_path}"
        );
    }

    for target_path in [
        "src/contracts/workflow_primitives.rs",
        "src/contracts/blueprint.rs",
        "src/compiler/workflow_primitives.rs",
        "src/runtime/lanes.rs",
        "src/runtime/request_context.rs",
        "src/runtime/effects.rs",
        "src/runtime/blueprint_effects.rs",
        "src/workspace/schema_epoch.rs",
        "src/cli/parser.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_persistence.rs",
        "tests/fixtures/runtime_json/auto_port_v0_20_0_runtime_contract_scout.json",
        "tests/fixtures/compiler_parity/auto_port_v0_20_0_compiler_contract_scout.json",
        "tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
    ] {
        assert!(
            referenced_targets.contains(target_path),
            "missing v0.20.0 expected Rust target {target_path}"
        );
    }

    let generated_scout_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        fixture["python_reference"]["generated_scout"]
            .as_str()
            .expect("generated scout path"),
    );
    let generated_scout =
        fs::read_to_string(generated_scout_path).expect("read v0.20.0 generated scout");
    let generated_changed_paths = changed_paths_from_generated_scout(&generated_scout);
    assert_eq!(
        generated_changed_paths.len(),
        249,
        "v0.20.0 generated scout should pin the 249-path Python diff"
    );
    let live_changed_paths = live_python_diff_changed_paths("v0.19.0..v0.20.0");
    assert_eq!(
        live_changed_paths, generated_changed_paths,
        "live Python v0.19.0..v0.20.0 diff must match generated scout"
    );

    for source_path in generated_changed_paths
        .iter()
        .map(|path| format!("../millrace-py/{path}"))
    {
        assert!(
            referenced_sources.contains(&source_path),
            "missing v0.20.0 Python source reference {source_path}"
        );
    }

    let changed_mappings = fixture["changed_path_mappings"]
        .as_array()
        .expect("changed path mappings are present");
    let mapped_changed_paths: BTreeSet<_> = changed_mappings
        .iter()
        .map(|mapping| {
            mapping["python_path"]
                .as_str()
                .expect("Python path")
                .to_owned()
        })
        .collect();
    assert_eq!(
        mapped_changed_paths.len(),
        changed_mappings.len(),
        "v0.20.0 path mappings must not contain duplicate Python paths"
    );
    assert_eq!(
        mapped_changed_paths, generated_changed_paths,
        "v0.20.0 parity fixture must map every generated scout path exactly"
    );

    let allowed_target_kinds = BTreeSet::from([
        "implementation",
        "test",
        "documentation",
        "fixture",
        "package_evidence",
        "unsupported_gap_evidence",
        "reference_evidence",
    ]);
    let mut covered_mapping_surfaces = BTreeSet::new();
    for mapping in changed_mappings {
        let python_path = mapping["python_path"].as_str().expect("Python path");
        let surface_name = mapping["surface"].as_str().expect("mapping surface");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.20.0 path mapping uses unknown surface {surface_name}"
        );
        covered_mapping_surfaces.insert(surface_name);
        assert!(
            mapping["change_role"]
                .as_str()
                .is_some_and(is_snake_case_rust_test_name),
            "v0.20.0 path mapping has malformed change_role: {mapping:?}"
        );
        let rust_targets = mapping["rust_targets"]
            .as_array()
            .expect("rust target mappings are present");
        assert!(
            !rust_targets.is_empty(),
            "v0.20.0 path mapping has no Rust target: {mapping:?}"
        );
        let mut has_package_or_gap_evidence = false;
        for target in rust_targets {
            let kind = target["kind"].as_str().expect("Rust target kind");
            assert!(
                allowed_target_kinds.contains(kind),
                "v0.20.0 path mapping uses unknown Rust target kind {kind}"
            );
            let path = target["path"].as_str().expect("Rust target path");
            assert!(
                !path.is_empty()
                    && !path.contains('\\')
                    && !Path::new(path).is_absolute()
                    && !path.split('/').any(|segment| segment == ".."),
                "v0.20.0 path mapping references invalid Rust target path {path}"
            );
            if !is_evidence_only_target_kind(kind) && !planned_new_rust_targets.contains(path) {
                assert_repo_relative_target_exists(path, "v0.20.0 changed-path mapping");
            }
            if python_path.starts_with("packages/millrace-web/") {
                assert_ne!(
                    kind, "implementation",
                    "web package mapping must not authorize Rust web implementation expansion"
                );
            }
            has_package_or_gap_evidence |=
                kind == "package_evidence" || kind == "unsupported_gap_evidence";
        }
        if python_path.starts_with("packages/millrace-web/") {
            assert!(
                has_package_or_gap_evidence,
                "web package mapping must remain explicit package or unsupported-gap evidence"
            );
        }
    }
    for surface in [
        "workflow_primitive_contracts_assets",
        "compiler_authority_validation",
        "workspace_schema_lifecycle",
        "lanes_request_context_inspection",
        "runtime_effects_failure_policy",
        "blueprint_planning_runtime",
        "cli_status_run_once",
        "docs_operator_skill_release_evidence",
        "web_version_package_evidence",
        "test_guardrail_evidence",
        "release_validation_guardrails",
    ] {
        assert!(
            covered_mapping_surfaces.contains(surface),
            "changed-path mappings do not cover v0.20.0 surface {surface}"
        );
    }

    assert_eq!(
        fixture["workflow_primitive_contract"]["work_item_families"],
        json!([
            "task",
            "spec",
            "probe",
            "incident",
            "learning_request",
            "blueprint_draft"
        ])
    );
    assert!(
        fixture["workflow_primitive_contract"]["registry_collections"]
            .as_array()
            .expect("registry collections are present")
            .iter()
            .any(|value| value.as_str() == Some("runtime_effect_rules")),
        "missing v0.20.0 runtime effect rule registry evidence"
    );
    assert_eq!(
        fixture["blueprint_contract"]["mode_ids"],
        json!(["blueprint_codex", "blueprint_learning_codex"])
    );
    assert!(
        fixture["compiler_authority_contract"]["selected_authority_fields"]
            .as_array()
            .expect("compiler authority fields are present")
            .iter()
            .any(|value| value.as_str() == Some("workflow_primitive_fingerprints")),
        "missing v0.20.0 primitive fingerprint persistence evidence"
    );
    assert_eq!(
        fixture["cli_status_contract"]["removed_public_commands"],
        json!(["millrace run once"])
    );
    assert_eq!(
        fixture["cli_status_contract"]["bounded_one_tick_replacement"],
        "millrace run daemon --max-ticks 1"
    );

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/compiler_persistence.rs",
    ]);
    for guardrail in fixture["active_guardrail_tests"]
        .as_array()
        .expect("active guardrail tests are present")
    {
        let test_file = guardrail["file"].as_str().expect("guardrail file");
        let test_name = guardrail["name"].as_str().expect("guardrail name");
        assert!(
            available_tests.contains_key(test_file),
            "v0.20.0 fixture references unsupported guardrail test file {test_file}"
        );
        assert!(
            is_snake_case_rust_test_name(test_name),
            "v0.20.0 fixture references malformed guardrail test name {test_name}"
        );
        assert!(
            available_tests[test_file].contains(test_name),
            "v0.20.0 fixture references stale guardrail test {test_file}::{test_name}"
        );
    }

    let required_release_checks = fixture["required_release_checks"]
        .as_array()
        .expect("required release checks are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.19.0..v0.20.0",
        "cargo fmt --check",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo test --test contracts_public_exports",
        "cargo test --test compiler_contracts",
        "cargo test --test compiler_materialization",
        "cargo test --test compiler_persistence",
    ] {
        assert!(
            required_release_checks
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.20.0 required release check {command}"
        );
    }

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_20_0_version_package_evidence"]["python_target_tag"],
        "v0.20.0"
    );
    assert_eq!(
        dashboard_fixture["v0_20_0_version_package_evidence"]["diff_range"],
        "v0.19.0..v0.20.0"
    );
    let web_sources =
        dashboard_fixture["v0_20_0_version_package_evidence"]["changed_python_sources"]
            .as_array()
            .expect("v0.20.0 web package source list");
    for source_path in [
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/models.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/queue_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
        "../millrace-py/packages/millrace-web/tests/test_summary_services.py",
    ] {
        assert!(
            web_sources
                .iter()
                .any(|value| value.as_str() == Some(source_path)),
            "missing v0.20.0 web package evidence source {source_path}"
        );
    }
    assert_eq!(
        dashboard_fixture["v0_20_0_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.20.0 package/version and dashboard summary evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, dashboard HTTP API, or separate dashboard package is added."
    );

    let non_live_guarantees = fixture["non_live_guarantees"]
        .as_array()
        .expect("non-live guarantees are present");
    for guarantee in [
        "no live Python execution beyond checked-out ../millrace-py diff inspection",
        "no live Codex runner",
        "no live Pi runner",
        "no network",
        "no credentials",
        "no web server",
        "no remote skill installation",
        "no release upload",
        "no publishing",
    ] {
        assert!(
            non_live_guarantees
                .iter()
                .any(|value| value.as_str() == Some(guarantee)),
            "missing v0.20.0 non-live guarantee {guarantee}"
        );
    }
}

#[test]
fn committed_auto_port_v0_20_0_release_parity_evidence_covers_version_docs_package_workflow_blueprint_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_20_0_release_parity_evidence.json")
            .expect("read v0.20.0 release parity evidence fixture"),
    )
    .expect("parse v0.20.0 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_20_0_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.19.0");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.20.0");
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "c432786242e9e7cf9f7262ec0ec4f906f4bb7bf7"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.19.0..v0.20.0"
    );
    assert_eq!(fixture["python_reference"]["changed_path_count"], 249);
    assert_eq!(fixture["rust_release"]["crate_version"], "0.5.0");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.4.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.5.0"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.20.0 release package include surface {expected}"
        );
    }
    for forbidden in [
        "packages/millrace-web/**/*",
        "millrace-agents/**",
        "target/**",
    ] {
        assert!(
            !include
                .iter()
                .any(|value| value.as_str() == Some(forbidden)),
            "v0.20.0 package include surfaces must not include {forbidden}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "v0.20.0 parity fixture included under tests/fixtures/cli_parity/auto_port_v0_20_0_parity_evidence.json",
        "v0.20.0 release fixture included under tests/fixtures/cli_parity/auto_port_v0_20_0_release_parity_evidence.json",
        "workflow primitive registry assets included under src/assets/baseline/registry/",
        "Blueprint graph, mode, stage-kind, entrypoint, and skill assets included under src/assets/baseline/",
        "runtime Blueprint docs included under docs/runtime/millrace-blueprint-planning.md",
        "runtime error-code docs included under docs/runtime/millrace-runtime-error-codes.md",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "Cargo metadata checked by cargo metadata --no-deps --format-version 1",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.20.0 package readiness evidence {expected}"
        );
    }

    let required_surface_names: BTreeSet<_> = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present")
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "runtime_docs_release_evidence",
        "package_release_evidence",
        "workflow_blueprint_release_evidence",
        "web_dashboard_v0_20_0_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_public_exports.rs",
        "tests/contracts_workflow_primitives.rs",
        "tests/contracts_blueprint.rs",
        "tests/compiler_workflow_primitives.rs",
        "tests/runtime_effects.rs",
        "tests/runtime_failure_policy.rs",
        "tests/runtime_lanes.rs",
        "tests/runtime_request_context.rs",
        "tests/runtime_run_inspection.rs",
        "tests/blueprint_contracts.rs",
        "tests/blueprint_effects.rs",
        "tests/blueprint_planning_loop.rs",
    ]);
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    for surface in fixture["surfaces"]
        .as_array()
        .expect("surface entries are present")
    {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.20.0 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.20.0 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.20.0 release fixture references stale Rust test {test_file}::{test_name}"
            );
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for source_path in [
        "../millrace-py/docs/runtime/millrace-blueprint-planning.md",
        "../millrace-py/docs/runtime/millrace-compiler-and-frozen-plans.md",
        "../millrace-py/src/millrace_ai/runtime/effects.py",
        "../millrace-py/src/millrace_ai/runtime/request_context.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.20.0 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-arbiter-and-completion-behavior.md",
        "docs/runtime/millrace-blueprint-planning.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-entrypoint-mapping.md",
        "docs/runtime/millrace-loop-authoring.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runner-architecture.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-runtime-error-codes.md",
        "docs/runtime/millrace-workspace-baselines-and-upgrades.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.20.0 release local doc reference {doc_path}"
        );
        assert_repo_relative_target_exists(doc_path, "v0.20.0 release local docs");
    }

    let mapping_evidence = &fixture["changed_path_mapping_evidence"];
    assert_eq!(
        mapping_evidence["generated_scout"],
        "millrace-agents/auto-port/generated/auto-port-python-v0.19.0-to-v0.20.0-rust-0.5.0.md"
    );
    assert_eq!(
        mapping_evidence["evidence_fixture"],
        "tests/fixtures/cli_parity/auto_port_v0_20_0_parity_evidence.json"
    );
    assert_eq!(mapping_evidence["mapped_python_paths"], 249);

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo test --test parity_cli",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo package --allow-dirty --offline",
        "git diff --check",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.20.0 release validation command {command}"
        );
    }

    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }

    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.20.0 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.5.0"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.5.0"
    );

    let source_package = &fixture["source_package_evidence"];
    assert_eq!(
        source_package["rust_package_includes_web_implementation"],
        false
    );
    assert!(
        source_package["excluded_surfaces"]
            .as_array()
            .expect("excluded surfaces")
            .iter()
            .any(|value| value.as_str() == Some("packages/millrace-web/**/*")),
        "v0.20.0 source-package evidence must keep Python web package excluded"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.5.0");
    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_20_0_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));
}

#[test]
fn committed_auto_port_v0_19_0_release_parity_evidence_covers_version_docs_package_capabilities_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_19_0_release_parity_evidence.json")
            .expect("read v0.19.0 release parity evidence fixture"),
    )
    .expect("parse v0.19.0 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_19_0_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.6");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.19.0");
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "efb9c5881f524d23dcb78aecfc96fdf7cda9d26f"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.6..v0.19.0"
    );
    assert_eq!(fixture["python_reference"]["changed_path_count"], 61);
    assert_eq!(fixture["rust_release"]["crate_version"], "0.4.0");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.3.5");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.4.0"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.19.0 release package include surface {expected}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "v0.19.0 parity fixture included under tests/fixtures/cli_parity/auto_port_v0_19_0_parity_evidence.json",
        "v0.19.0 runtime scout included under tests/fixtures/runtime_json/auto_port_v0_19_0_runtime_contract_scout.json",
        "runtime execution capability docs included under docs/runtime/millrace-execution-capabilities.md",
        "runner architecture docs included under docs/runtime/millrace-runner-architecture.md",
        "run-inspection capability output included under src/cli/read_only.rs and tests/parity_cli.rs",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "full clippy checked by cargo clippy --all-targets --all-features -- -D warnings",
        "full suite checked by cargo test --all",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "Cargo metadata checked by cargo metadata --no-deps --format-version 1",
        "dry-run publish verification checked by cargo publish --dry-run --allow-dirty",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.19.0 package readiness evidence {expected}"
        );
    }

    let required_surface_names: BTreeSet<_> = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present")
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "capability_governance_release_docs",
        "capability_runtime_release_evidence",
        "package_release_evidence",
        "web_dashboard_v0_19_0_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/contracts_capabilities.rs",
        "tests/contracts_public_exports.rs",
        "tests/compiler_capability_grants.rs",
        "tests/runtime_capability_gates.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/runners_capability_support.rs",
    ]);
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    for surface in fixture["surfaces"]
        .as_array()
        .expect("surface entries are present")
    {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.19.0 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.19.0 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.19.0 release fixture references stale Rust test {test_file}::{test_name}"
            );
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for source_path in [
        "../millrace-py/docs/runtime/millrace-execution-capabilities.md",
        "../millrace-py/docs/runtime/millrace-runner-architecture.md",
        "../millrace-py/src/millrace_ai/runtime/capability_gates.py",
        "../millrace-py/src/millrace_ai/runtime/inspection.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.19.0 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-execution-capabilities.md",
        "docs/runtime/millrace-runner-architecture.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/millrace-technical-overview.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/runtime_json/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.19.0 release local doc reference {doc_path}"
        );
        assert_repo_relative_target_exists(doc_path, "v0.19.0 release local docs");
    }

    let mapping_evidence = &fixture["changed_path_mapping_evidence"];
    assert_eq!(
        mapping_evidence["generated_scout"],
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.6-to-v0.19.0-rust-0.4.0.md"
    );
    assert_eq!(
        mapping_evidence["evidence_fixture"],
        "tests/fixtures/cli_parity/auto_port_v0_19_0_parity_evidence.json"
    );
    assert_eq!(mapping_evidence["mapped_python_paths"], 61);

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.18.6..v0.19.0",
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo test --test runtime_daemon",
        "cargo test --test runtime_serial",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
        "cargo package --allow-dirty --offline --list | rg \"__pycache__|\\.pyc|\\.pyo\"",
        "git diff --check",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.19.0 release validation command {command}"
        );
    }

    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }

    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.19.0 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.4.0"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.4.0"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.4.0");
    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_19_0_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));
}

#[test]
fn committed_auto_port_v0_18_6_release_parity_evidence_covers_version_docs_package_interventions_durable_sources_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_6_release_parity_evidence.json")
            .expect("read v0.18.6 release parity evidence fixture"),
    )
    .expect("parse v0.18.6 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_6_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.4");
    assert_eq!(fixture["python_reference"]["intermediate_tag"], "v0.18.5");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.6");
    assert_eq!(
        fixture["python_reference"]["target_peeled_commit"],
        "63e623bc6fcfcf74ae0cc2ce5605a12ae4179873"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.4..v0.18.6"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.5");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.3.4");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.5"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
        "!**/__pycache__/**",
        "!**/*.pyc",
        "!**/*.pyo",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.18.6 release package include surface {expected}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "v0.18.6 parity fixture included under tests/fixtures/cli_parity/auto_port_v0_18_6_parity_evidence.json",
        "v0.18.6 runtime scout included under tests/fixtures/runtime_json/auto_port_v0_18_6_runtime_contract_scout.json",
        "v0.18.6 mailbox intervention payload fixture included under tests/fixtures/runtime_json/mailbox_intervention_payloads.json",
        "operator intervention CLI coverage included under tests/parity_cli.rs",
        "operator intervention archive/audit coverage included under tests/workspace_queue_state_stores.rs",
        "daemon mailbox/read-only intervention coverage included under tests/runtime_daemon.rs",
        "durable idea-source and closure recovery coverage included under tests/runtime_serial.rs and tests/runtime_daemon.rs",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "generated Python cache artifacts excluded by Cargo package rules for __pycache__, .pyc, and .pyo paths",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "Cargo metadata checked by cargo metadata --no-deps --format-version 1",
        "cargo clippy --all-targets --all-features -- -D warnings checked the full target set",
        "cargo test --all checked the full local test suite",
        "plain cargo publish --dry-run checked and blocked only by the dirty Builder worktree",
        "cargo publish --dry-run --allow-dirty checked the release candidate without uploading",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.18.6 package readiness evidence {expected}"
        );
    }

    let required_surface_names: BTreeSet<_> = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present")
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "operator_intervention_release_docs",
        "durable_idea_source_release_docs",
        "package_release_evidence",
        "web_dashboard_v0_18_5_v0_18_6_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/workspace_runtime_control.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
        "tests/runtime_serial.rs",
    ]);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_version_subcommand_matches_version_flag",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_3_5",
        "tests/parity_cli.rs::committed_auto_port_v0_18_6_parity_fixture_covers_operator_intervention_durable_idea_source_and_web_guardrails",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::committed_auto_port_v0_18_6_release_parity_evidence_covers_version_docs_package_interventions_durable_sources_and_web_gap",
        "tests/parity_cli.rs::rust_cli_queue_operator_interventions_render_control_results_and_mutate_workspace",
        "tests/parity_cli.rs::rust_cli_incident_interventions_render_control_results_and_validate_filenames",
        "tests/parity_cli.rs::rust_cli_intervention_commands_route_to_mailbox_when_daemon_owns_workspace",
        "tests/parity_cli.rs::rust_cli_direct_intervention_refuses_active_runtime_stage_without_mailbox",
        "tests/contracts_runtime_json.rs::auto_port_v0_18_6_runtime_contract_scout_pins_operator_intervention_and_durable_idea_sources",
        "tests/workspace_runtime_control.rs::workspace_runtime_control_v0_18_6_guardrail_fixture_requires_intervention_mailbox_payloads",
        "tests/workspace_queue_state_stores.rs::workspace_queue_v0_18_6_guardrail_fixture_requires_operator_intervention_archive_audit_surface",
        "tests/runtime_daemon.rs::runtime_daemon_v0_18_6_guardrail_fixture_requires_intervention_mailbox_and_durable_idea_source_surfaces",
        "tests/runtime_daemon.rs::daemon_mailbox_applies_operator_intervention_when_idle_and_records_audit",
        "tests/runtime_daemon.rs::daemon_loop_continues_after_missing_root_idea_source_blocks_planning",
        "tests/runtime_serial.rs::serial_tick_closure_target_prefers_durable_idea_source_over_legacy_references",
        "tests/runtime_serial.rs::serial_tick_blocks_planning_when_backfill_root_idea_source_is_missing",
    ]);
    let mut seen_rust_refs = BTreeSet::new();
    for surface in fixture["surfaces"]
        .as_array()
        .expect("surface entries are present")
    {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.6 release fixture references unknown surface {surface_name}"
        );
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.18.6 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.18.6 release fixture references stale Rust test {test_file}::{test_name}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.18.6 release fixture references unknown Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.18.6 release Rust test {rust_ref}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/millrace-arbiter-and-completion-behavior.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-workspace-baselines-and-upgrades.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/runtime_json/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.18.6 release local doc reference {doc_path}"
        );
    }

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "git -C ../millrace-py diff --name-only v0.18.4..v0.18.6",
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo test --test parity_cli",
        "cargo test --test contracts_runtime_json",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
        "cargo package --allow-dirty --offline --list | rg \"__pycache__|\\.pyc|\\.pyo\"",
        "git diff --check",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.6 release validation command {command}"
        );
    }
    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.18.6 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.3.5"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.3.5"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["result_class"],
        "arbiter_visible_remediation_evidence"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["observed_terminal_warning"],
        "aborting upload due to dry run"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline --list | rg \"__pycache__|\\.pyc|\\.pyo\""]
            ["status"],
        "passed_no_matches"
    );
    assert_eq!(results_by_command["git diff --check"]["status"], "passed");
    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.3.5");
    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_18_5_version_package_gap"].as_bool() == Some(true)
            && gap["v0_18_6_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));
}

#[test]
fn committed_auto_port_v0_18_4_release_parity_evidence_covers_version_docs_package_blocked_recovery_retry_autorecovery_daemon_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_4_release_parity_evidence.json")
            .expect("read v0.18.4 release parity evidence fixture"),
    )
    .expect("parse v0.18.4 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_4_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.3");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.4");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "516e947e90155b6436dbc9efcf932254f34bc39c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.3..v0.18.4"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.4");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.3.3");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.4"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.18.4 release package include surface {expected}"
        );
    }
    let cache_excludes = fixture["rust_release"]["generated_cache_excludes"]
        .as_array()
        .expect("generated cache excludes are present");
    for forbidden in ["!**/__pycache__/**", "!**/*.pyc", "!**/*.pyo"] {
        assert!(
            cache_excludes
                .iter()
                .any(|value| value.as_str() == Some(forbidden)),
            "missing v0.18.4 generated cache exclude {forbidden}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "v0.18.4 parity fixture included under tests/fixtures/cli_parity/auto_port_v0_18_4_parity_evidence.json",
        "v0.18.4 runtime scout included under tests/fixtures/runtime_json/auto_port_v0_18_4_runtime_contract_scout.json",
        "blocked recovery metadata contracts included under src/contracts/runtime_json.rs and src/runtime/blocked_recovery.rs",
        "manual blocked retry CLI coverage included under tests/parity_cli.rs",
        "auto-recovery config/status coverage included under tests/contracts_runtime_json.rs and tests/runtime_daemon.rs",
        "daemon stranded-dependency recovery coverage included under tests/runtime_daemon.rs",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "generated Python cache artifacts excluded by Cargo package rules for __pycache__, .pyc, and .pyo paths",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "Cargo metadata checked by cargo metadata --no-deps --format-version 1",
        "full crate lint checked by cargo clippy --all-targets --all-features -- -D warnings",
        "full crate test suite checked by cargo test --all",
        "plain cargo publish --dry-run checked and blocked only by the dirty Builder worktree",
        "cargo publish --dry-run --allow-dirty checked the release candidate without uploading",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.18.4 package readiness evidence {expected}"
        );
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "blocked_recovery_release_docs",
        "package_release_evidence",
        "web_dashboard_v0_18_4_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runners_normalization.rs",
        "tests/workspace_queue_state_stores.rs",
        "tests/runtime_daemon.rs",
    ]);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_version_subcommand_matches_version_flag",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_3_4",
        "tests/parity_cli.rs::committed_auto_port_v0_18_4_parity_fixture_covers_blocked_recovery_retry_autorecovery_and_web_guardrails",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::committed_auto_port_v0_18_4_release_parity_evidence_covers_version_docs_package_blocked_recovery_retry_autorecovery_daemon_and_web_gap",
        "tests/parity_cli.rs::rust_queue_retry_blocked_requeues_retryable_task_refreshes_snapshot_and_audits",
        "tests/parity_cli.rs::rust_queue_retry_blocked_refuses_guards_and_force_overrides_retryability",
        "tests/parity_cli.rs::rust_config_validate_compiles_selected_or_explicit_config_modes",
        "tests/parity_cli.rs::rust_status_config_and_modes_read_only_commands_render_without_mutation",
        "tests/contracts_runtime_json.rs::auto_port_v0_18_4_runtime_contract_scout_pins_blocked_recovery_config_and_status_sources",
        "tests/runners_normalization.rs::runner_normalization_v0_18_4_guardrail_fixture_requires_failure_classifier_metadata",
        "tests/workspace_queue_state_stores.rs::workspace_queue_v0_18_4_guardrail_fixture_requires_blocked_retry_audit_surface",
        "tests/runtime_daemon.rs::runtime_daemon_v0_18_4_guardrail_fixture_requires_auto_recovery_idle_cycle_surface",
        "tests/runtime_daemon.rs::daemon_idle_cycle_auto_requeues_one_retryable_stranded_blocked_dependency",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.4 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.18.4 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.18.4 release fixture references stale Rust test {test_file}::{test_name}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.18.4 release fixture references unknown Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.18.4 release Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/docs/runtime/millrace-cli-reference.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/docs/skills/millrace-ops-agent-manual/SKILL.md",
        "../millrace-py/src/millrace_ai/__init__.py",
        "../millrace-py/src/millrace_ai/runtime/blocked_recovery.py",
        "../millrace-py/src/millrace_ai/cli/commands/queue.py",
        "../millrace-py/src/millrace_ai/config/models.py",
        "../millrace-py/src/millrace_ai/runtime/supervisor.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.18.4 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/runtime_json/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.18.4 release local doc reference {doc_path}"
        );
    }

    let mapping_evidence = &fixture["changed_path_mapping_evidence"];
    assert_eq!(
        mapping_evidence["generated_scout"],
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.3-to-v0.18.4-rust-0.3.4.md"
    );
    assert_eq!(
        mapping_evidence["evidence_fixture"],
        "tests/fixtures/cli_parity/auto_port_v0_18_4_parity_evidence.json"
    );
    assert_eq!(mapping_evidence["mapped_python_paths"], 28);

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo package --allow-dirty --offline --list | rg \"__pycache__|\\.pyc|\\.pyo\"",
        "git diff --check",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.4 release validation command {command}"
        );
    }
    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }

    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.18.4 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.3.4"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.3.4"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["result_class"],
        "arbiter_visible_remediation_evidence"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["observed_terminal_warning"],
        "aborting upload due to dry run"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["compressed_size"],
        "677.4KiB"
    );
    assert_eq!(results_by_command["git diff --check"]["status"], "passed");
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline --list | rg \"__pycache__|\\.pyc|\\.pyo\""]
            ["status"],
        "passed_no_matches"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.3.4");
    let completion_withheld_until = closure_guard["completion_withheld_until"]
        .as_array()
        .expect("completion guard reasons are present");
    assert!(completion_withheld_until.iter().any(|reason| {
        reason.as_str().is_some_and(|reason| {
            reason.contains(
                "same-lineage tasks auto-port-0-18-4-01 through auto-port-0-18-4-07 are done",
            )
        })
    }));

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_18_4_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_4_version_package_evidence"]["python_target_tag"],
        "v0.18.4"
    );
    assert_eq!(
        dashboard_fixture["v0_18_4_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.4 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );
}

#[test]
fn committed_auto_port_v0_18_3_release_parity_evidence_covers_version_docs_package_librarian_learning_skill_lint_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_3_release_parity_evidence.json")
            .expect("read v0.18.3 release parity evidence fixture"),
    )
    .expect("parse v0.18.3 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_3_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.2");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.3");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "6556e55c8463ce9256716bc425a49059b4c5981c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.2..v0.18.3"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.3");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.3.2");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.3"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.18.3 release package include surface {expected}"
        );
    }
    let excluded = fixture["rust_release"]["excluded_runtime_workspace_artifacts"]
        .as_array()
        .expect("excluded runtime workspace artifacts are present");
    for forbidden in ["millrace-agents/**", "ideas/**", "target/**"] {
        assert!(
            excluded
                .iter()
                .any(|value| value.as_str() == Some(forbidden)),
            "missing v0.18.3 excluded runtime workspace artifact {forbidden}"
        );
    }
    let cache_excludes = fixture["rust_release"]["generated_cache_excludes"]
        .as_array()
        .expect("generated cache excludes are present");
    for forbidden in ["!**/__pycache__/**", "!**/*.pyc", "!**/*.pyo"] {
        assert!(
            cache_excludes
                .iter()
                .any(|value| value.as_str() == Some(forbidden)),
            "missing v0.18.3 generated cache exclude {forbidden}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "Librarian managed entrypoint included under src/assets/baseline/entrypoints/learning/librarian.md",
        "Librarian learning graph and loop assets included under src/assets/baseline/graphs/learning/standard.json and src/assets/baseline/loops/learning/default.json",
        "Learning mode trigger assets included under src/assets/baseline/modes/learning_codex.json, src/assets/baseline/modes/learning_codex_integrated.json, and src/assets/baseline/modes/learning_pi.json",
        "Librarian stage-kind registry and core skill included under src/assets/baseline/registry/stage_kinds/learning/librarian.json and src/assets/baseline/skills/stage/learning/librarian-core/SKILL.md",
        "Shipped skill lint assets included under src/assets/baseline/skills/README.md, src/assets/baseline/skills/skills_index.md, src/assets/baseline/skills/shared/marathon-qa-audit/SKILL.md, and stage skill directories",
        "runtime docs included under docs/runtime/",
        "source package map included under docs/source-package-map.md",
        "v0.18.3 parity fixtures included under tests/fixtures/cli_parity/auto_port_v0_18_3_parity_evidence.json, tests/fixtures/compiler_parity/auto_port_v0_18_3_compiler_contract_scout.json, and tests/fixtures/runtime_json/auto_port_v0_18_3_runtime_contract_scout.json",
        "release fixture included under tests/fixtures/cli_parity/auto_port_v0_18_3_release_parity_evidence.json",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "generated Python cache artifacts excluded by Cargo package rules for __pycache__, .pyc, and .pyo paths",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "Cargo metadata checked by cargo metadata --no-deps --format-version 1",
        "full crate lint checked by cargo clippy --all-targets --all-features -- -D warnings",
        "full crate test suite checked by cargo test --all",
        "plain cargo publish --dry-run checked and blocked only by the dirty Builder worktree",
        "cargo publish --dry-run --allow-dirty checked the release candidate without uploading",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.18.3 package readiness evidence {expected}"
        );
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "librarian_learning_release_docs",
        "package_release_evidence",
        "web_dashboard_v0_18_3_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/compiler_contracts.rs",
        "tests/compiler_materialization.rs",
        "tests/workspace_assets_baseline.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/runners_normalization.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
        "tests/shipped_skill_lint.rs",
    ]);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_version_subcommand_matches_version_flag",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_3_3",
        "tests/parity_cli.rs::committed_auto_port_v0_18_3_parity_fixture_covers_librarian_learning_trigger_metadata_skill_lint_and_web_guardrails",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::committed_auto_port_v0_18_3_release_parity_evidence_covers_version_docs_package_librarian_learning_skill_lint_and_web_gap",
        "tests/compiler_parity.rs::compiler_parity_scout_pins_python_v0_18_3_librarian_assets_graph_modes_and_skill_lint_sources",
        "tests/contracts_runtime_json.rs::auto_port_v0_18_3_runtime_contract_scout_pins_librarian_trigger_runner_metadata_sources",
        "tests/compiler_contracts.rs::baseline_mode_graph_and_stage_kind_assets_parse_through_contracts",
        "tests/compiler_materialization.rs::learning_modes_materialize_learning_graph_triggers_and_concurrency_policy",
        "tests/compiler_materialization.rs::learning_mode_graph_exports_use_stable_plane_order_and_learning_edges",
        "tests/workspace_assets_baseline.rs::live_guidance_assets_match_packaged_baseline",
        "tests/contracts_stage_metadata.rs::every_stage_has_metadata_and_plane",
        "tests/contracts_runtime_json.rs::python_v0_18_3_librarian_stage_result_runtime_json_round_trips",
        "tests/runners_normalization.rs::raw_result_normalization_preserves_active_work_item_source_metadata",
        "tests/runtime_serial.rs::serial_tick_planner_complete_triggers_librarian_request_with_planner_artifacts",
        "tests/runtime_serial.rs::serial_tick_targeted_librarian_learning_request_uses_librarian_metadata_and_marks_done",
        "tests/runtime_serial.rs::serial_tick_librarian_blocked_preserves_recoverable_failure_evidence",
        "tests/runtime_daemon.rs::daemon_supervisor_planner_trigger_dispatches_librarian_and_traces_spawned_request",
        "tests/shipped_skill_lint.rs::every_shipped_skill_package_passes_packaged_skill_lint",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.3 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.18.3 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.18.3 release fixture references stale Rust test {test_file}::{test_name}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.18.3 release fixture references unknown Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.18.3 release Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/ROADMAP.md",
        "../millrace-py/docs/source-package-map.md",
        "../millrace-py/docs/runtime/millrace-compiler-and-frozen-plans.md",
        "../millrace-py/docs/runtime/millrace-entrypoint-mapping.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/learning/librarian.md",
        "../millrace-py/src/millrace_ai/assets/graphs/learning/standard.json",
        "../millrace-py/src/millrace_ai/runtime/learning_triggers.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/src/millrace_ai/runners/normalization.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.18.3 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/millrace-technical-overview.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-entrypoint-mapping.md",
        "docs/runtime/millrace-loop-authoring.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "docs/runtime/millrace-runtime-lifecycle-diagram.md",
        "docs/runtime/millrace-workspace-baselines-and-upgrades.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/compiler_parity/README.md",
        "tests/fixtures/runtime_json/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.18.3 release local doc reference {doc_path}"
        );
    }

    let mapping_evidence = &fixture["changed_path_mapping_evidence"];
    assert_eq!(
        mapping_evidence["generated_scout"],
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.2-to-v0.18.3-rust-0.3.3.md"
    );
    assert_eq!(
        mapping_evidence["evidence_fixture"],
        "tests/fixtures/cli_parity/auto_port_v0_18_3_parity_evidence.json"
    );
    assert_eq!(mapping_evidence["mapped_python_paths"], 50);
    let mapped_target_kinds: BTreeSet<_> = mapping_evidence["mapped_target_kinds"]
        .as_array()
        .expect("mapped target kinds are present")
        .iter()
        .map(|kind| kind.as_str().expect("mapped target kind"))
        .collect();
    assert_eq!(
        mapped_target_kinds,
        BTreeSet::from([
            "implementation",
            "test",
            "documentation",
            "fixture",
            "package_evidence",
            "unsupported_gap_evidence",
            "reference_evidence",
        ])
    );

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.3 release validation command {command}"
        );
    }
    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }

    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.18.3 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.3.3"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.3.3"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["result_class"],
        "arbiter_visible_remediation_evidence"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["observed_terminal_warning"],
        "aborting upload due to dry run"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["compressed_size"],
        "638.2KiB"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline --list | rg \"__pycache__|\\.pyc|\\.pyo\""]
            ["status"],
        "passed_no_matches"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.3.3");
    let completion_withheld_until = closure_guard["completion_withheld_until"]
        .as_array()
        .expect("completion guard reasons are present");
    assert!(completion_withheld_until.iter().any(|reason| {
        reason.as_str().is_some_and(|reason| {
            reason.contains(
                "same-lineage tasks auto-port-0-18-3-01 through auto-port-0-18-3-07 are done",
            )
        })
    }));
    assert!(completion_withheld_until.iter().any(|reason| {
        reason.as_str().is_some_and(|reason| {
            reason.contains("plain cargo publish --dry-run is rerun in a clean committed worktree")
        })
    }));

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_18_3_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_3_version_package_evidence"]["python_target_tag"],
        "v0.18.3"
    );
    assert_eq!(
        dashboard_fixture["v0_18_3_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.3 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );
}

#[test]
fn committed_auto_port_v0_18_1_release_parity_evidence_covers_version_docs_package_recon_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_1_release_parity_evidence.json")
            .expect("read v0.18.1 release parity evidence fixture"),
    )
    .expect("parse v0.18.1 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_1_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.0");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.1");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "0396c7852793b212d31345862b38a7d6f3f02854"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.0..v0.18.1"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.1");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.3.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.1"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.18.1 release package include surface {expected}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "Recon managed assets included under src/assets/baseline/entrypoints/planning/recon.md",
        "Recon stage-kind registry included under src/assets/baseline/registry/stage_kinds/planning/recon.json",
        "Recon core skill included under src/assets/baseline/skills/stage/planning/recon-core/SKILL.md",
        "mode runner bindings included under src/assets/baseline/modes/",
        "probe/recon parity fixtures included under tests/fixtures/",
        "release fixture included under tests/fixtures/cli_parity/auto_port_v0_18_1_release_parity_evidence.json",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "plain cargo publish --dry-run checked and blocked only by the Builder dirty worktree",
        "cargo publish --dry-run --allow-dirty checked the release candidate without uploading",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.18.1 package readiness evidence {expected}"
        );
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "probe_recon_release_docs",
        "package_release_evidence",
        "web_dashboard_version_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
    ]);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_version_subcommand_matches_version_flag",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_3_1",
        "tests/parity_cli.rs::committed_auto_port_v0_18_1_parity_fixture_covers_probe_recon_release_guardrails",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::committed_auto_port_v0_18_1_release_parity_evidence_covers_version_docs_package_recon_and_web_gap",
        "tests/compiler_parity.rs::compiler_parity_scout_pins_python_v0_18_1_recon_assets_and_graph_sources",
        "tests/contracts_runtime_json.rs::auto_port_v0_18_1_runtime_contract_scout_pins_probe_recon_sources",
        "tests/runtime_serial.rs::serial_tick_claims_probe_for_recon_stage_request_metadata",
        "tests/runtime_serial.rs::recon_to_execution_persists_packet_marks_probe_done_enqueues_task_and_traces",
        "tests/runtime_serial.rs::recon_to_planning_persists_packet_marks_probe_done_enqueues_spec",
        "tests/runtime_daemon.rs::daemon_mailbox_drains_control_and_intake_commands_into_processed_archives",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.1 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.18.1 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.18.1 release fixture references stale Rust test {test_file}::{test_name}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.18.1 release fixture references unknown Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.18.1 release Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/ROADMAP.md",
        "../millrace-py/docs/source-package-map.md",
        "../millrace-py/docs/runtime/millrace-cli-reference.md",
        "../millrace-py/docs/runtime/millrace-modes-and-loops.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/src/millrace_ai/contracts/work_documents.py",
        "../millrace-py/src/millrace_ai/recon_packets.py",
        "../millrace-py/src/millrace_ai/runtime/recon_transitions.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.18.1 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/compiler_parity/README.md",
        "tests/fixtures/runtime_json/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.18.1 release local doc reference {doc_path}"
        );
    }

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo test --test parity_cli",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.1 release validation command {command}"
        );
    }
    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }

    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.18.1 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo clippy --all-targets --all-features -- -D warnings"]["status"],
        "passed"
    );
    assert_eq!(results_by_command["cargo test --all"]["status"], "passed");
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.3.1"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.3.1"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_by_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["exit_code"],
        101
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed_dry_run_without_upload"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["status"],
        "passed"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.3.1");
    let completion_withheld_until = closure_guard["completion_withheld_until"]
        .as_array()
        .expect("completion guard reasons are present");
    assert!(completion_withheld_until.iter().any(|reason| {
        reason.as_str().is_some_and(|reason| {
            reason.contains("same-lineage work is done and Checker validates")
        })
    }));

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_18_1_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_1_version_package_evidence"]["python_target_tag"],
        "v0.18.1"
    );
    assert_eq!(
        dashboard_fixture["v0_18_1_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.1 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );
}

#[test]
fn committed_auto_port_v0_18_2_release_parity_evidence_covers_version_docs_package_integrator_status_recon_ownership_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_2_release_parity_evidence.json")
            .expect("read v0.18.2 release parity evidence fixture"),
    )
    .expect("parse v0.18.2 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_2_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.18.1");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.2");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "5444cb9485ea90b67b2ed6ba7e0723ae9fe7b79f"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.18.1..v0.18.2"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.2");
    assert_eq!(fixture["rust_release"]["previous_crate_version"], "0.3.1");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.2"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.18.2 release package include surface {expected}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "Integrator managed assets included under src/assets/baseline/entrypoints/execution/integrator.md",
        "Integrator graph and loop assets included under src/assets/baseline/graphs/execution/with_integrator.json and src/assets/baseline/loops/execution/with_integrator.json",
        "Integrated Codex mode assets included under src/assets/baseline/modes/default_codex_integrated.json and src/assets/baseline/modes/learning_codex_integrated.json",
        "Integrator stage-kind registry and core skill included under src/assets/baseline/registry/stage_kinds/execution/integrator.json and src/assets/baseline/skills/stage/execution/integrator-core/SKILL.md",
        "runtime docs included under docs/runtime/",
        "source package map included under docs/source-package-map.md",
        "v0.18.2 parity fixtures included under tests/fixtures/cli_parity/auto_port_v0_18_2_parity_evidence.json, tests/fixtures/compiler_parity/auto_port_v0_18_2_compiler_contract_scout.json, and tests/fixtures/runtime_json/auto_port_v0_18_2_runtime_contract_scout.json",
        "release fixture included under tests/fixtures/cli_parity/auto_port_v0_18_2_release_parity_evidence.json",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "plain cargo publish --dry-run checked and blocked only by the Builder dirty worktree",
        "cargo publish --dry-run --allow-dirty checked the release candidate without uploading",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.18.2 package readiness evidence {expected}"
        );
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "integrator_status_recon_ownership_release_docs",
        "package_release_evidence",
        "web_dashboard_v0_18_2_package_gap",
        "final_release_validation",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&[
        "tests/parity_cli.rs",
        "tests/compiler_parity.rs",
        "tests/contracts_runtime_json.rs",
        "tests/compiler_assets.rs",
        "tests/compiler_materialization.rs",
        "tests/workspace_assets_baseline.rs",
        "tests/contracts_stage_metadata.rs",
        "tests/compiler_contracts.rs",
        "tests/runtime_serial.rs",
        "tests/runtime_daemon.rs",
    ]);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_version_subcommand_matches_version_flag",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_3_2",
        "tests/parity_cli.rs::committed_auto_port_v0_18_2_parity_fixture_covers_integrator_status_recon_ownership_guardrails",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::committed_auto_port_v0_18_2_release_parity_evidence_covers_version_docs_package_integrator_status_recon_ownership_and_web_gap",
        "tests/compiler_parity.rs::compiler_parity_scout_pins_python_v0_18_2_integrator_assets_and_graph_sources",
        "tests/contracts_runtime_json.rs::auto_port_v0_18_2_runtime_contract_scout_pins_status_recon_ownership_sources",
        "tests/compiler_assets.rs::opt_in_integrated_execution_assets_resolve_without_changing_default_mode",
        "tests/compiler_materialization.rs::opt_in_integrated_execution_graph_materializes_and_exports_integrator_node",
        "tests/workspace_assets_baseline.rs::initialized_workspace_integrator_assets_match_packaged_baseline",
        "tests/contracts_stage_metadata.rs::stage_work_item_ownership_matrix_matches_runtime_contracts",
        "tests/contracts_runtime_json.rs::read_only_status_payload_serializes_python_compatible_json_fields",
        "tests/compiler_contracts.rs::recon_handoff_outcomes_cannot_route_directly_to_stage_nodes",
        "tests/runtime_serial.rs::integrated_mode_routes_builder_to_integrator_then_checker_and_traces_sequence",
        "tests/runtime_serial.rs::integrated_mode_routes_integrator_blocked_to_recovery_and_threshold_consultant",
        "tests/runtime_serial.rs::recon_packet_decision_mismatch_blocks_probe_with_invalid_handoff_evidence",
        "tests/runtime_serial.rs::serial_tick_requeues_and_blocks_stage_work_item_ownership_mismatches",
        "tests/runtime_daemon.rs::daemon_supervisor_integrated_mode_drains_builder_integrator_checker_sequence",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.2 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);
        for source in surface["python_sources"]
            .as_array()
            .expect("python_sources array")
        {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }
        for rust_test in surface["rust_tests"].as_array().expect("rust_tests array") {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                available_tests.contains_key(test_file),
                "v0.18.2 release fixture references unsupported Rust test file {test_file}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.18.2 release fixture references stale Rust test {test_file}::{test_name}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.18.2 release fixture references unknown Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.18.2 release Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/ROADMAP.md",
        "../millrace-py/docs/source-package-map.md",
        "../millrace-py/docs/runtime/millrace-cli-reference.md",
        "../millrace-py/docs/runtime/millrace-modes-and-loops.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/src/millrace_ai/assets/entrypoints/execution/integrator.md",
        "../millrace-py/src/millrace_ai/assets/graphs/execution/with_integrator.json",
        "../millrace-py/src/millrace_ai/assets/modes/default_codex_integrated.json",
        "../millrace-py/src/millrace_ai/cli/status_view.py",
        "../millrace-py/src/millrace_ai/contracts/recon.py",
        "../millrace-py/src/millrace_ai/runtime/stage_requests.py",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.18.2 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/testing.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
        "tests/fixtures/compiler_parity/README.md",
        "tests/fixtures/runtime_json/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.18.2 release local doc reference {doc_path}"
        );
    }

    let mapping_evidence = &fixture["changed_path_mapping_evidence"];
    assert_eq!(
        mapping_evidence["generated_scout"],
        "millrace-agents/auto-port/generated/auto-port-python-v0.18.1-to-v0.18.2-rust-0.3.2.md"
    );
    assert_eq!(
        mapping_evidence["evidence_fixture"],
        "tests/fixtures/cli_parity/auto_port_v0_18_2_parity_evidence.json"
    );
    assert_eq!(mapping_evidence["mapped_python_paths"], 57);
    let mapped_target_kinds: BTreeSet<_> = mapping_evidence["mapped_target_kinds"]
        .as_array()
        .expect("mapped target kinds are present")
        .iter()
        .map(|kind| kind.as_str().expect("mapped target kind"))
        .collect();
    assert_eq!(
        mapped_target_kinds,
        BTreeSet::from([
            "implementation",
            "test",
            "documentation",
            "fixture",
            "package_evidence",
            "unsupported_gap_evidence",
            "reference_evidence",
        ])
    );

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo test --test parity_cli",
        "cargo test --test compiler_parity",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.2 release validation command {command}"
        );
    }
    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }

    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            results_by_command.contains_key(command),
            "missing v0.18.2 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo clippy --all-targets --all-features -- -D warnings"]["status"],
        "passed"
    );
    assert_eq!(results_by_command["cargo test --all"]["status"], "passed");
    assert_eq!(
        results_by_command["cargo metadata --no-deps --format-version 1"]["observed_package_version"],
        "0.3.2"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.3.2"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_by_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["exit_code"],
        101
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed_dry_run_without_upload"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["status"],
        "passed"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.3.2");
    let completion_withheld_until = closure_guard["completion_withheld_until"]
        .as_array()
        .expect("completion guard reasons are present");
    assert!(completion_withheld_until.iter().any(|reason| {
        reason.as_str().is_some_and(|reason| {
            reason.contains(
                "same-lineage tasks auto-port-0-18-2-01 through auto-port-0-18-2-07 are done",
            )
        })
    }));

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["v0_18_2_version_package_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["v0_18_2_version_package_evidence"]["python_target_tag"],
        "v0.18.2"
    );
    assert_eq!(
        dashboard_fixture["v0_18_2_version_package_evidence"]["rust_release_handling"],
        "Recorded as v0.18.2 package/version evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );
}

#[test]
fn committed_auto_port_v0_17_4_release_parity_evidence_covers_version_docs_package_and_web_sync() {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_17_4_release_parity_evidence.json")
            .expect("read v0.17.4 release parity evidence fixture"),
    )
    .expect("parse v0.17.4 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_17_4_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.17.3");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.17.4");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "304e537964ff772c815689b87e4c1e3b805c656c"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.17.3..v0.17.4"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.2.1");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.2.1"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.17.4 release package include surface {expected}"
        );
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "learning_noop_release_docs",
        "trigger_safety_runtime_docs",
        "web_dashboard_version_sync_gap",
        "assets_docs_release_package",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let test_files = [
        "tests/compiler_materialization.rs",
        "tests/contracts_runtime_json.rs",
        "tests/parity_cli.rs",
        "tests/runtime_serial.rs",
        "tests/workspace_assets_baseline.rs",
    ];
    let available_tests = rust_test_functions_by_file(&test_files);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::committed_auto_port_v0_17_4_parity_fixture_covers_noop_trigger_runtime_and_cli_surfaces",
        "tests/runtime_serial.rs::python_v0_17_4_learning_noop_terminal_marks_request_done_with_noop_evidence",
        "tests/contracts_runtime_json.rs::python_v0_17_4_stage_result_no_op_runtime_json_fixture_round_trips_as_non_success",
        "tests/compiler_materialization.rs::direct_curator_learning_trigger_requires_safe_destination",
        "tests/compiler_materialization.rs::direct_curator_learning_trigger_accepts_targeted_destination",
        "tests/runtime_serial.rs::python_v0_17_4_runtime_generated_learning_request_copies_trigger_destination_metadata",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_2_1",
        "tests/workspace_assets_baseline.rs::packaged_baseline_manifest_is_sorted_hashed_and_deterministic",
        "tests/workspace_assets_baseline.rs::initialize_workspace_deploys_managed_assets_and_manifest_io",
        "tests/parity_cli.rs::committed_auto_port_v0_17_4_release_parity_evidence_covers_version_docs_package_and_web_sync",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.17.4 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);

        let python_sources = surface["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "surface {surface_name} is missing Python source references"
        );
        for source in python_sources {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }

        let rust_tests = surface["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "surface {surface_name} is missing Rust test references"
        );
        for rust_test in rust_tests {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                is_snake_case_rust_test_name(test_name),
                "v0.17.4 release fixture has malformed Rust test name {test_name}"
            );
            assert!(
                available_tests.contains_key(test_file),
                "v0.17.4 release fixture references unsupported Rust test file {test_file}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.17.4 release fixture references unknown Rust test {rust_ref}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.17.4 release fixture references stale Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.17.4 release Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/ROADMAP.md",
        "../millrace-py/docs/runtime/README.md",
        "../millrace-py/docs/runtime/millrace-compiler-and-frozen-plans.md",
        "../millrace-py/docs/runtime/millrace-loop-authoring.md",
        "../millrace-py/docs/runtime/millrace-modes-and-loops.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/docs/runtime/millrace-runtime-lifecycle-diagram.md",
        "../millrace-py/docs/skills/millrace-ops-agent-manual/SKILL.md",
        "../millrace-py/packages/millrace-web/pyproject.toml",
        "../millrace-py/packages/millrace-web/src/millrace_web/__init__.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/app.py",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.17.4 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.17.4 release local doc reference {doc_path}"
        );
    }

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.17.4 release validation command {command}"
        );
    }

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["version_sync_only"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    assert_eq!(
        dashboard_fixture["version_sync_evidence"]["python_target_tag"],
        "v0.17.4"
    );
    assert_eq!(
        dashboard_fixture["version_sync_evidence"]["rust_release_handling"],
        "Recorded as version/dependency sync evidence for the existing unsupported dashboard gap; no Rust web server, static shell, SSE stream, or separate dashboard package is added."
    );
}

#[test]
fn committed_auto_port_v0_18_0_release_parity_evidence_covers_version_docs_package_graph_trace_and_web_gap()
 {
    let fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/auto_port_v0_18_0_release_parity_evidence.json")
            .expect("read v0.18.0 release parity evidence fixture"),
    )
    .expect("parse v0.18.0 release parity evidence fixture");
    assert_eq!(fixture["kind"], "auto_port_v0_18_0_release_parity_evidence");
    assert_eq!(fixture["schema_version"], "1.0");
    assert_eq!(fixture["python_reference"]["previous_tag"], "v0.17.4");
    assert_eq!(fixture["python_reference"]["target_tag"], "v0.18.0");
    assert_eq!(
        fixture["python_reference"]["target_commit"],
        "e4ccf099c8345a8b8708cdaa1ac510bdc7851387"
    );
    assert_eq!(
        fixture["python_reference"]["diff_range"],
        "v0.17.4..v0.18.0"
    );
    assert_eq!(fixture["rust_release"]["crate_version"], "0.3.0");
    assert_eq!(
        fixture["rust_release"]["version_command_expectation"],
        "millrace 0.3.0"
    );

    let include = fixture["rust_release"]["package_include_surfaces"]
        .as_array()
        .expect("package include surfaces are present");
    for expected in [
        "Cargo.lock",
        "CHANGELOG.md",
        "README.md",
        "ROADMAP.md",
        "docs/**/*.md",
        "src/assets/**/*",
        "src/**/*.rs",
        "tests/**/*.rs",
        "tests/fixtures/**/*",
        "tests/support/**/*",
    ] {
        assert!(
            include.iter().any(|value| value.as_str() == Some(expected)),
            "missing v0.18.0 release package include surface {expected}"
        );
    }

    let readiness = fixture["rust_release"]["package_readiness_evidence"]
        .as_array()
        .expect("package readiness evidence is present");
    for expected in [
        "runtime docs included under docs/runtime/",
        "compiled graph docs included under docs/runtime/millrace-compiled-stage-graphs-and-run-traces.md",
        "graph/trace CLI fixture docs included under tests/fixtures/cli_parity/",
        "release fixture included under tests/fixtures/cli_parity/auto_port_v0_18_0_release_parity_evidence.json",
        "web-dashboard unsupported-gap evidence included under tests/fixtures/cli_parity/web_dashboard_parity_decision.json",
        "version-visible CLI output checked by cargo run --quiet -- --version",
        "plain cargo publish --dry-run checked and blocked only by the Builder dirty worktree",
        "cargo publish --dry-run --allow-dirty checked the release candidate without uploading",
        "package content checked by cargo package --allow-dirty --offline",
    ] {
        assert!(
            readiness
                .iter()
                .any(|value| value.as_str() == Some(expected)),
            "missing v0.18.0 package readiness evidence {expected}"
        );
    }

    let required_surfaces = fixture["required_surfaces"]
        .as_array()
        .expect("required surfaces are present");
    let required_surface_names: BTreeSet<_> = required_surfaces
        .iter()
        .map(|surface| surface.as_str().expect("surface name"))
        .collect();
    let expected_surfaces = BTreeSet::from([
        "graph_trace_release_docs",
        "cli_graph_trace_version_output",
        "web_dashboard_graph_trace_gap",
        "package_release_evidence",
    ]);
    assert_eq!(required_surface_names, expected_surfaces);

    let available_tests = rust_test_functions_by_file(&["tests/parity_cli.rs"]);
    let required_rust_refs = BTreeSet::from([
        "tests/parity_cli.rs::committed_auto_port_v0_18_0_parity_fixture_covers_graph_trace_and_web_gap_scout",
        "tests/parity_cli.rs::rust_compile_graph_renders_text_json_plane_filter_errors_and_output_files",
        "tests/parity_cli.rs::rust_runs_trace_renders_text_json_output_and_fallbacks_without_mutation",
        "tests/parity_cli.rs::rust_version_command_has_millrace_shape",
        "tests/parity_cli.rs::rust_version_subcommand_matches_version_flag",
        "tests/parity_cli.rs::committed_web_dashboard_parity_decision_records_unsupported_gap_with_sources",
        "tests/parity_cli.rs::rust_crate_release_metadata_and_package_include_rules_are_0_3_0",
        "tests/parity_cli.rs::committed_auto_port_v0_18_0_release_parity_evidence_covers_version_docs_package_graph_trace_and_web_gap",
    ]);

    let surfaces = fixture["surfaces"]
        .as_array()
        .expect("surface entries are present");
    let mut covered_surfaces = BTreeSet::new();
    let mut referenced_paths = BTreeSet::new();
    let mut seen_rust_refs = BTreeSet::new();
    for surface in surfaces {
        let surface_name = surface["surface"].as_str().expect("surface name");
        assert!(
            required_surface_names.contains(surface_name),
            "v0.18.0 release fixture references unknown surface {surface_name}"
        );
        covered_surfaces.insert(surface_name);

        let python_sources = surface["python_sources"]
            .as_array()
            .expect("python_sources array");
        assert!(
            !python_sources.is_empty(),
            "surface {surface_name} is missing Python source references"
        );
        for source in python_sources {
            referenced_paths.insert(source.as_str().expect("Python source path"));
        }

        let rust_tests = surface["rust_tests"].as_array().expect("rust_tests array");
        assert!(
            !rust_tests.is_empty(),
            "surface {surface_name} is missing Rust test references"
        );
        for rust_test in rust_tests {
            let test_file = rust_test["file"].as_str().expect("Rust test file");
            let test_name = rust_test["name"].as_str().expect("Rust test name");
            assert!(
                is_snake_case_rust_test_name(test_name),
                "v0.18.0 release fixture has malformed Rust test name {test_name}"
            );
            assert!(
                available_tests.contains_key(test_file),
                "v0.18.0 release fixture references unsupported Rust test file {test_file}"
            );
            let rust_ref = format!("{test_file}::{test_name}");
            assert!(
                required_rust_refs.contains(rust_ref.as_str()),
                "v0.18.0 release fixture references unknown Rust test {rust_ref}"
            );
            assert!(
                available_tests[test_file].contains(test_name),
                "v0.18.0 release fixture references stale Rust test {rust_ref}"
            );
            seen_rust_refs.insert(rust_ref);
        }
    }
    assert_eq!(covered_surfaces, expected_surfaces);
    for rust_ref in &required_rust_refs {
        assert!(
            seen_rust_refs.contains(*rust_ref),
            "missing required v0.18.0 release Rust test {rust_ref}"
        );
    }

    for source_path in [
        "../millrace-py/CHANGELOG.md",
        "../millrace-py/README.md",
        "../millrace-py/ROADMAP.md",
        "../millrace-py/docs/source-package-map.md",
        "../millrace-py/docs/runtime/README.md",
        "../millrace-py/docs/runtime/millrace-cli-reference.md",
        "../millrace-py/docs/runtime/millrace-compiled-stage-graphs-and-run-traces.md",
        "../millrace-py/docs/runtime/millrace-compiler-and-frozen-plans.md",
        "../millrace-py/docs/runtime/millrace-modes-and-loops.md",
        "../millrace-py/docs/runtime/millrace-runtime-architecture.md",
        "../millrace-py/docs/skills/millrace-autonomous-delegation/SKILL.md",
        "../millrace-py/docs/skills/millrace-ops-agent-manual/SKILL.md",
        "../millrace-py/src/millrace_ai/cli/commands/compile.py",
        "../millrace-py/src/millrace_ai/cli/commands/runs.py",
        "../millrace-py/src/millrace_ai/cli/formatting.py",
        "../millrace-py/tests/cli/test_graph_trace_cli.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/compiled_plan_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/services/run_reader.py",
        "../millrace-py/packages/millrace-web/src/millrace_web/static/assets/app.js",
    ] {
        assert!(
            referenced_paths.contains(source_path),
            "missing v0.18.0 release Python source reference {source_path}"
        );
    }

    let local_docs = fixture["local_docs"]
        .as_array()
        .expect("local docs list is present");
    for doc_path in [
        "README.md",
        "CHANGELOG.md",
        "ROADMAP.md",
        "docs/rust-port-roadmap.md",
        "docs/source-package-map.md",
        "docs/runtime/README.md",
        "docs/runtime/millrace-cli-reference.md",
        "docs/runtime/millrace-compiled-stage-graphs-and-run-traces.md",
        "docs/runtime/millrace-compiler-and-frozen-plans.md",
        "docs/runtime/millrace-modes-and-loops.md",
        "docs/runtime/millrace-runtime-architecture.md",
        "millrace-agents/outline.md",
        "tests/fixtures/cli_parity/README.md",
    ] {
        assert!(
            local_docs
                .iter()
                .any(|value| value.as_str() == Some(doc_path)),
            "missing v0.18.0 release local doc reference {doc_path}"
        );
    }

    let validation = fixture["release_readiness_commands"]
        .as_array()
        .expect("release-readiness commands are present");
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo test --test parity_cli",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            validation
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.0 release validation command {command}"
        );
    }
    let forbidden = fixture["forbidden_release_actions"]
        .as_array()
        .expect("forbidden release actions are present");
    for command in validation {
        let command = command.as_str().expect("release command");
        assert!(
            !forbidden
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "release-readiness command must not be a forbidden release action: {command}"
        );
    }
    let results = fixture["release_readiness_results"]
        .as_array()
        .expect("release-readiness results are present");
    let results_by_command: BTreeMap<_, _> = results
        .iter()
        .map(|result| (result["command"].as_str().expect("result command"), result))
        .collect();
    for command in [
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all",
        "cargo metadata --no-deps --format-version 1",
        "cargo run --quiet -- --version",
        "cargo publish --dry-run",
        "cargo publish --dry-run --allow-dirty",
        "cargo package --allow-dirty --offline",
    ] {
        assert!(
            results_by_command.contains_key(command),
            "missing v0.18.0 release result for {command}"
        );
    }
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["status"],
        "blocked_by_dirty_worktree"
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run"]["exit_code"],
        101
    );
    assert_eq!(
        results_by_command["cargo publish --dry-run --allow-dirty"]["status"],
        "passed_dry_run_without_upload"
    );
    assert_eq!(
        results_by_command["cargo package --allow-dirty --offline"]["status"],
        "passed"
    );
    assert_eq!(
        results_by_command["cargo run --quiet -- --version"]["observed_stdout"],
        "millrace 0.3.0"
    );

    let closure_guard = &fixture["arbiter_closure_guard"];
    assert_eq!(closure_guard["crate_version_gate"], "0.3.0");
    assert_eq!(
        closure_guard["remaining_parity_gap"],
        "python_packages_millrace_web deferred_unsupported_gap"
    );
    let completion_withheld_until = closure_guard["completion_withheld_until"]
        .as_array()
        .expect("completion guard reasons are present");
    assert!(completion_withheld_until.iter().any(|reason| {
        reason.as_str().is_some_and(|reason| {
            reason.contains("plain cargo publish --dry-run dirty-worktree limitation")
        })
    }));

    let gaps = fixture["explicit_gaps"]
        .as_array()
        .expect("explicit gaps are present");
    assert!(gaps.iter().any(|gap| {
        gap["surface"].as_str() == Some("python_packages_millrace_web")
            && gap["status"].as_str() == Some("deferred_unsupported_gap")
            && gap["graph_trace_gap"].as_bool() == Some(true)
            && gap["evidence_fixture"].as_str()
                == Some("tests/fixtures/cli_parity/web_dashboard_parity_decision.json")
    }));

    let dashboard_fixture: Value = serde_json::from_str(
        &read_fixture("cli_parity/web_dashboard_parity_decision.json")
            .expect("read web dashboard parity decision fixture"),
    )
    .expect("parse web dashboard parity decision fixture");
    let graph_trace_gap_surfaces =
        dashboard_fixture["v0_18_0_graph_trace_evidence"]["required_gap_surfaces"]
            .as_array()
            .expect("v0.18.0 web gap surface list");
    for surface in [
        "compiled_plan_graph_api_summary",
        "run_trace_api_summary",
        "recent_trace_flow_overlay",
        "trace_outcome_labels",
        "package_version_dependency_sync",
        "read_only_no_lock_guarantee",
    ] {
        assert!(
            graph_trace_gap_surfaces
                .iter()
                .any(|value| value.as_str() == Some(surface)),
            "missing v0.18.0 release web gap surface {surface}"
        );
    }
    let rust_shadow_commands =
        dashboard_fixture["v0_18_0_graph_trace_evidence"]["rust_shadow_commands"]
            .as_array()
            .expect("v0.18.0 web shadow command list");
    for command in ["millrace compile graph", "millrace runs trace <run_id>"] {
        assert!(
            rust_shadow_commands
                .iter()
                .any(|value| value.as_str() == Some(command)),
            "missing v0.18.0 web shadow command {command}"
        );
    }
}

#[test]
fn rust_unknown_commands_keep_exit_code_2() {
    let output = run_rust_millrace(["not-a-command"]).expect("run Rust millrace unknown command");

    assert_exit_code(&output, 2);
    assert_eq!(output.stdout, "");
    assert_eq!(output.stderr, "error: unknown command `not-a-command`\n");
}

#[test]
fn rust_init_cli_creates_canonical_workspace_tree() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let output = run_rust_millrace(["init", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace init");
    let paths = workspace_paths(&root);

    output.assert_success();
    assert_eq!(
        output.stdout,
        format!("workspace: {}\ninitialized: true\n", paths.root.display())
    );
    assert_eq!(output.stderr, "");

    for directory in paths.directories() {
        assert!(
            directory.is_dir(),
            "missing initialized directory: {}",
            directory.display()
        );
    }
    assert!(paths.runtime_config_file.is_file());
    assert!(paths.runtime_snapshot_file.is_file());
    assert!(paths.recovery_counters_file.is_file());
    assert!(paths.baseline_manifest_file.is_file());
    assert!(
        paths
            .runtime_root
            .join("entrypoints/execution/builder.md")
            .is_file()
    );
}

#[test]
fn rust_init_cli_then_doctor_reports_healthy_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);

    let output = run_rust_millrace(["doctor", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace doctor");

    output.assert_success();
    assert_eq!(output.stdout, "ok: true\nerrors: 0\nwarnings: 0\n");
    assert_eq!(output.stderr, "");
}

#[test]
fn rust_doctor_reports_duplicate_task_lifecycle_state_paths() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let task = runnable_task_markdown("task-duplicate");
    fs::write(paths.tasks_done_dir.join("task-duplicate.md"), &task).unwrap();
    fs::write(
        paths.tasks_blocked_dir.join("task-duplicate.md"),
        task.replace(
            "Summary: Exercise bounded daemon CLI",
            "Summary: stale blocked predecessor",
        ),
    )
    .unwrap();

    let output = run_rust_millrace(["doctor", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace doctor duplicate lifecycle");

    assert_exit_code(&output, 1);
    assert_eq!(output.stderr, "");
    assert!(
        output
            .stdout
            .contains("ok: false\nerrors: 1\nwarnings: 0\n")
    );
    assert!(
        output
            .stdout
            .contains("error: duplicate_task_lifecycle_state ")
    );
    assert!(output.stdout.contains(" task task-duplicate appears in multiple lifecycle states: done:millrace-agents/tasks/done/task-duplicate.md, blocked:millrace-agents/tasks/blocked/task-duplicate.md\n"));
}

#[test]
fn rust_init_cli_is_idempotent_and_preserves_operator_edits() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let managed_asset = paths.runtime_root.join("entrypoints/execution/builder.md");
    let edited_files = [
        (&paths.execution_status_file, "### CHECKER_PASS\n"),
        (&paths.planning_status_file, "### PLANNER_COMPLETE\n"),
        (&paths.learning_status_file, "### ANALYST_COMPLETE\n"),
        (&paths.outline_file, "# Existing Outline\n"),
        (&paths.historylog_file, "existing history\n"),
        (&paths.runtime_snapshot_file, "{\"custom\": true}\n"),
        (
            &paths.recovery_counters_file,
            "{\"entries\": [\"custom\"]}\n",
        ),
        (&paths.learning_events_file, "{\"event\": true}\n"),
        (
            &paths.runtime_config_file,
            "[runtime]\ndefault_mode = \"custom\"\n",
        ),
        (&managed_asset, "operator asset edit\n"),
    ];

    for (path, payload) in edited_files {
        fs::write(path, payload).unwrap();
    }

    run_rust_millrace(["init", "--workspace", root.to_str().unwrap()])
        .expect("rerun Rust millrace init")
        .assert_success();

    for (path, payload) in edited_files {
        assert_eq!(fs::read_to_string(path).unwrap(), payload);
    }
}

#[test]
fn rust_compile_validate_persists_artifacts_for_supported_modes() {
    let cases = [
        ("default_codex", "default_codex"),
        ("default_pi", "default_pi"),
        ("learning_codex", "learning_codex"),
        ("learning_pi", "learning_pi"),
        ("standard_plain", "default_codex"),
    ];

    for (requested_mode, expected_mode) in cases {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("workspace");
        run_init_for(&root);

        let output = run_rust_millrace([
            "compile",
            "validate",
            "--workspace",
            root.to_str().unwrap(),
            "--mode",
            requested_mode,
        ])
        .expect("run Rust millrace compile validate");
        let paths = workspace_paths(&root);

        output.assert_success();
        assert_eq!(output.stderr, "");
        assert!(output.stdout.contains("ok: true\n"));
        assert!(
            output
                .stdout
                .contains(&format!("mode_id: {expected_mode}\n"))
        );
        assert!(output.stdout.contains("used_last_known_good: false\n"));
        assert!(
            output
                .stdout
                .contains(&format!("compile_input.mode_id: {expected_mode}\n"))
        );
        assert!(paths.compiled_plan_file.is_file());
        assert!(paths.compile_diagnostics_file.is_file());
        assert!(!paths.state_dir.join("compiled_graph_plan.json").exists());

        let plan: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.compiled_plan_file).unwrap()).unwrap();
        let diagnostics: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.compile_diagnostics_file).unwrap())
                .unwrap();
        assert_eq!(plan["kind"], "compiled_run_plan");
        assert_eq!(plan["mode_id"], expected_mode);
        assert_eq!(diagnostics["kind"], "compile_diagnostics");
        assert_eq!(diagnostics["ok"], true);
        assert_eq!(diagnostics["mode_id"], expected_mode);
    }
}

#[test]
fn rust_compile_show_renders_representative_inspection_fields() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);

    let output = run_rust_millrace([
        "compile",
        "show",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "learning_codex",
    ])
    .expect("run Rust millrace compile show");

    output.assert_success();
    assert_eq!(output.stderr, "");
    for expected in [
        "ok: true\n",
        "mode_id: learning_codex\n",
        "compiled_plan_currentness: current\n",
        "compiled_plan_id: plan-learning_codex-",
        "execution_loop_id: execution.standard\n",
        "planning_loop_id: planning.standard\n",
        "learning_loop_id: learning.standard\n",
        "baseline_manifest_id: ",
        "compile_input.config_fingerprint: cfg-",
        "persisted_compile_input.assets_fingerprint: assets-",
        "execution_capabilities.total_grants: ",
        "execution_capabilities.by_decision.granted: ",
        "execution_capabilities.execution.total_grants: ",
        "entry: execution.task -> builder\n",
        "entry: learning.learning_request -> analyst\n",
        "completion: closure_target -> arbiter\n",
        "completion_behavior.trigger: backlog_drained\n",
        "learning_triggers: 4\n",
        "learning_trigger: execution.doublechecker.success-to-analyst\n",
        "learning_trigger: planning.planner.complete-to-librarian\n",
        "learning_trigger.target_stage: librarian\n",
        "concurrency_policy: present\n",
        "concurrency_policy.mutually_exclusive_planes: execution, planning\n",
        "workflow_primitives.work_item_families: ",
        "workflow_primitives.runtime_effect_rules: ",
        "workflow_primitive_fingerprint.work_item_families: ",
        "workspace_schema_epoch: ",
        "lane_policy: compiled.default\n",
        "lane: execution.main plane=execution families=task",
        "pending_compiled_plan: none\n",
        "node_order: execution.0 -> builder\n",
        "node_order: learning.0 -> analyst\n",
        "stage: execution.builder\n",
        "stage_kind_id: builder\n",
        "entrypoint_path: entrypoints/execution/builder.md\n",
        "running_status_marker: BUILDER_RUNNING\n",
        "required_skills: skills/stage/execution/builder-core/SKILL.md\n",
        "runner_name: codex_cli\n",
        "thinking_level: none\n",
        "model_reasoning_effort: none\n",
        "timeout_seconds: 3600\n",
        "execution_capability_policy_fingerprint: cap-pol-",
        "execution_capability_grant: builder runner.invoke granted runtime_enforced grant-",
        "execution_capability_grant: builder workspace.read granted advisory_only grant-",
        "execution_capability_grant: builder artifact.write granted runtime_enforced grant-",
        "execution_capability_warning: builder workspace.read required_advisory required capability workspace.read resolved to advisory-only\n",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected output fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
}

#[test]
fn rust_compile_commands_reject_uninitialized_workspace_without_creating_it() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    for command in ["validate", "graph"] {
        let output = run_rust_millrace(["compile", command, "--workspace", root.to_str().unwrap()])
            .expect("run Rust millrace compile command");

        assert_exit_code(&output, 1);
        assert!(
            output
                .stdout
                .starts_with("error: workspace is not initialized: ")
        );
        assert_eq!(output.stderr, "");
        assert!(!root.join("millrace-agents").exists());
    }
}

#[test]
fn rust_compile_graph_renders_text_json_plane_filter_errors_and_output_files() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let snapshot_before = fs::read(&paths.runtime_snapshot_file).unwrap();

    let text = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "learning_codex",
        "--plane",
        "execution",
    ])
    .expect("run Rust millrace compile graph text");
    text.assert_success();
    assert_eq!(text.stderr, "");
    assert!(
        text.stdout
            .contains("compiled_plan_id: plan-learning_codex-")
    );
    assert!(text.stdout.contains("mode_id: learning_codex\n"));
    assert!(text.stdout.contains("planes: execution\n"));
    assert!(text.stdout.contains("execution:\n"));
    assert!(
        text.stdout
            .contains("  builder --BUILDER_COMPLETE--> checker\n")
    );
    assert!(!text.stdout.contains("planning:\n"));

    let json_output = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "learning_codex",
        "--format",
        "json",
    ])
    .expect("run Rust millrace compile graph json");
    json_output.assert_success();
    assert_eq!(json_output.stderr, "");
    let graphs: Vec<Value> =
        serde_json::from_str(&json_output.stdout).expect("parse compile graph JSON output");
    let planes: Vec<_> = graphs
        .iter()
        .map(|graph| graph["plane"].as_str().unwrap())
        .collect();
    assert_eq!(planes, vec!["execution", "learning", "planning"]);
    assert_eq!(graphs[0]["kind"], "compiled_stage_graph");
    assert_eq!(graphs[0]["nodes"][0]["node_id"], "builder");
    assert_eq!(graphs[0]["lane_policy"]["policy_id"], "compiled.default");

    let blueprint_text = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "blueprint_codex",
        "--plane",
        "planning",
    ])
    .expect("run Rust millrace compile graph blueprint text");
    blueprint_text.assert_success();
    for expected in [
        "lane_policy: compiled.default\n",
        "  node manager_blueprint lane=planning.main request_context_profile_id=manager_blueprint.default\n",
        "  runtime_effect_rule_selection manager_blueprint manager_blueprint_manifest_to_blueprint_drafts\n",
        "  node evaluator_blueprint lane=planning.main request_context_profile_id=evaluator_blueprint.default\n",
        "  runtime_effect_rule_selection evaluator_blueprint evaluator_blueprint_approved_to_task\n",
    ] {
        assert!(
            blueprint_text.stdout.contains(expected),
            "missing expected blueprint compile graph fragment: {expected}\nstdout:\n{}",
            blueprint_text.stdout
        );
    }

    let output_path = temp_dir.path().join("planning-graph.json");
    let file_output = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--plane",
        "planning",
        "--format",
        "json",
        "--output",
        output_path.to_str().unwrap(),
    ])
    .expect("run Rust millrace compile graph output file");
    file_output.assert_success();
    assert_eq!(file_output.stdout, "");
    assert_eq!(file_output.stderr, "");
    let file_graphs: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(&output_path).unwrap()).unwrap();
    assert_eq!(file_graphs.len(), 1);
    assert_eq!(file_graphs[0]["plane"], "planning");

    let invalid_format = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "yaml",
    ])
    .expect("run Rust millrace compile graph invalid format");
    assert_exit_code(&invalid_format, 1);
    assert_eq!(
        invalid_format.stdout,
        "error: --format must be text or json\n"
    );
    assert_eq!(invalid_format.stderr, "");

    let missing_plane = run_rust_millrace([
        "compile",
        "graph",
        "--workspace",
        root.to_str().unwrap(),
        "--plane",
        "learning",
    ])
    .expect("run Rust millrace compile graph missing plane");
    assert_exit_code(&missing_plane, 1);
    assert_eq!(
        missing_plane.stdout,
        "error: compiled plan does not include plane: learning\n"
    );
    assert_eq!(missing_plane.stderr, "");
    assert_eq!(
        fs::read(&paths.runtime_snapshot_file).unwrap(),
        snapshot_before
    );
    for dir in [
        &paths.tasks_queue_dir,
        &paths.tasks_active_dir,
        &paths.specs_queue_dir,
        &paths.incidents_incoming_dir,
        &paths.learning_requests_queue_dir,
    ] {
        assert_eq!(fs::read_dir(dir).unwrap().count(), 0);
    }
}

#[test]
fn rust_compile_validate_reports_invalid_mode_as_validation_failure() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);

    let output = run_rust_millrace([
        "compile",
        "validate",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "missing_mode",
    ])
    .expect("run Rust millrace compile validate");
    let paths = workspace_paths(&root);

    assert_exit_code(&output, 1);
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("ok: false\n"));
    assert!(output.stdout.contains("mode_id: missing_mode\n"));
    assert!(output.stdout.contains("error: "));
    assert!(output.stdout.contains("missing_mode"));
    assert!(!paths.compiled_plan_file.exists());
    assert!(paths.compile_diagnostics_file.is_file());
}

#[test]
fn rust_compile_cli_rejects_parse_errors_with_exit_code_2() {
    let cases = [
        (
            vec!["compile", "validate", "--unknown"],
            "error: unknown option `--unknown`\n",
        ),
        (
            vec!["compile", "validate", "--mode"],
            "error: missing value for `--mode`\n",
        ),
        (
            vec!["compile", "show", "--workspace="],
            "error: `--workspace` value must not be empty\n",
        ),
        (
            vec!["compile", "inspect"],
            "error: unknown compile command `inspect`\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_rust_millrace(args).expect("run Rust millrace compile parse failure");

        assert_exit_code(&output, 2);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, expected_stderr);
    }
}

#[test]
fn rust_queue_repair_lineage_preview_writes_report_skips_unsafe_findings_and_does_not_mutate() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let target = closure_target_state("spec-root-target", "idea-001");
    save_closure_target_state(&paths, &target).unwrap();

    let queued_task = lineage_task_markdown("task-queued", "old-root");
    let active_task = lineage_task_markdown("task-active", "old-root");
    let queued_spec = lineage_spec_markdown("spec-queued", "old-root");
    let queued_incident = lineage_incident_markdown("inc-queued", "old-root");
    fs::write(paths.tasks_queue_dir.join("task-queued.md"), &queued_task).unwrap();
    fs::write(paths.tasks_active_dir.join("task-active.md"), &active_task).unwrap();
    fs::write(paths.specs_queue_dir.join("spec-queued.md"), &queued_spec).unwrap();
    fs::write(
        paths.incidents_incoming_dir.join("inc-queued.md"),
        &queued_incident,
    )
    .unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.queue_depth_execution = 77;
    snapshot.queue_depth_planning = 88;
    save_snapshot(&paths, &snapshot).unwrap();

    let output = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "spec-root-target",
    ])
    .expect("run Rust millrace queue repair-lineage preview");

    output.assert_success();
    assert_eq!(output.stderr, "");
    for expected in [
        "root_spec_id: spec-root-target\n",
        "apply: false\n",
        "repair_count: 2\n",
        "change_count: 3\n",
        "repaired_count: 0\n",
        "skipped_count: 2\n",
        "change: incident inc-queued root_spec_id old-root -> spec-root-target\n",
        "change: task task-queued root_spec_id old-root -> spec-root-target\n",
        "change: task task-queued spec_id old-root -> spec-root-target\n",
        "skipped: spec spec-queued state=queue\n",
        "skipped: task task-active state=active\n",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected repair-lineage preview fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    let report_path = lineage_repair_report_path(&output.stdout);
    let report = assert_lineage_report_applied(&report_path, false);
    assert_eq!(report["changes"].as_array().unwrap().len(), 3);
    assert_eq!(report["skipped_findings"].as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
        queued_task
    );
    assert_eq!(
        fs::read_to_string(paths.tasks_active_dir.join("task-active.md")).unwrap(),
        active_task
    );
    assert_eq!(
        fs::read_to_string(paths.specs_queue_dir.join("spec-queued.md")).unwrap(),
        queued_spec
    );
    assert_eq!(
        fs::read_to_string(paths.incidents_incoming_dir.join("inc-queued.md")).unwrap(),
        queued_incident
    );
    let loaded_snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(loaded_snapshot.queue_depth_execution, 77);
    assert_eq!(loaded_snapshot.queue_depth_planning, 88);
    assert!(!paths.logs_dir.join("runtime_events.jsonl").exists());
}

#[test]
fn rust_queue_repair_lineage_apply_repairs_safe_documents_refreshes_snapshot_and_emits_event() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let target = closure_target_state("spec-root-target", "idea-001");
    save_closure_target_state(&paths, &target).unwrap();

    fs::write(
        paths.tasks_queue_dir.join("task-queued.md"),
        lineage_task_markdown("task-queued", "old-root"),
    )
    .unwrap();
    fs::write(
        paths.incidents_incoming_dir.join("inc-queued.md"),
        lineage_incident_markdown("inc-queued", "old-root"),
    )
    .unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.queue_depth_execution = 77;
    snapshot.queue_depth_planning = 88;
    save_snapshot(&paths, &snapshot).unwrap();

    let output = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "spec-root-target",
        "--apply",
    ])
    .expect("run Rust millrace queue repair-lineage apply");

    output.assert_success();
    assert_eq!(output.stderr, "");
    for expected in [
        "root_spec_id: spec-root-target\n",
        "apply: true\n",
        "repair_count: 2\n",
        "change_count: 3\n",
        "repaired_count: 2\n",
        "skipped_count: 0\n",
        "change: incident inc-queued root_spec_id old-root -> spec-root-target\n",
        "change: task task-queued root_spec_id old-root -> spec-root-target\n",
        "change: task task-queued spec_id old-root -> spec-root-target\n",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected repair-lineage apply fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    let report_path = lineage_repair_report_path(&output.stdout);
    assert_lineage_report_applied(&report_path, true);
    let repaired_task = fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap();
    assert!(repaired_task.contains("Root-Spec-ID: spec-root-target\n"));
    assert!(repaired_task.contains("Spec-ID: spec-root-target\n"));
    let repaired_incident =
        fs::read_to_string(paths.incidents_incoming_dir.join("inc-queued.md")).unwrap();
    assert!(repaired_incident.contains("Root-Spec-ID: spec-root-target\n"));
    let loaded_snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(loaded_snapshot.queue_depth_execution, 1);
    assert_eq!(loaded_snapshot.queue_depth_planning, 1);
    let events = fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl")).unwrap();
    assert!(events.contains("\"event_type\":\"closure_lineage_repaired\""));
    assert!(events.contains("\"root_spec_id\":\"spec-root-target\""));
}

#[test]
fn rust_queue_repair_lineage_apply_refuses_active_ownership_lock_before_document_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let target = closure_target_state("spec-root-target", "idea-001");
    save_closure_target_state(&paths, &target).unwrap();
    let queued_task = lineage_task_markdown("task-queued", "old-root");
    fs::write(paths.tasks_queue_dir.join("task-queued.md"), &queued_task).unwrap();
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("daemon-session"))
        .unwrap();

    let output = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "spec-root-target",
        "--apply",
    ])
    .expect("run Rust millrace queue repair-lineage with active lock");

    assert_exit_code(&output, 1);
    assert_eq!(
        output.stdout,
        "error: active runtime ownership lock prevents lineage repair\n"
    );
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
        queued_task
    );
    assert!(!paths.logs_dir.join("runtime_events.jsonl").exists());
}

#[test]
fn rust_queue_repair_lineage_apply_refuses_active_stage_before_document_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let target = closure_target_state("spec-root-target", "idea-001");
    save_closure_target_state(&paths, &target).unwrap();
    let queued_task = lineage_task_markdown("task-queued", "old-root");
    fs::write(paths.tasks_queue_dir.join("task-queued.md"), &queued_task).unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.active_plane = Some(Plane::Execution);
    snapshot.active_stage = Some(StageName::Builder);
    snapshot.active_node_id = Some("builder".to_owned());
    snapshot.active_stage_kind_id = Some("builder".to_owned());
    snapshot.active_run_id = Some("run-001".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    snapshot.active_work_item_id = Some("task-queued".to_owned());
    snapshot.active_since = Some(timestamp("2026-04-15T00:00:00Z"));
    save_snapshot(&paths, &snapshot).unwrap();

    let output = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "spec-root-target",
        "--apply",
    ])
    .expect("run Rust millrace queue repair-lineage with active stage");

    assert_exit_code(&output, 1);
    assert_eq!(
        output.stdout,
        "error: active runtime stage prevents lineage repair\n"
    );
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read_to_string(paths.tasks_queue_dir.join("task-queued.md")).unwrap(),
        queued_task
    );
    assert!(!paths.logs_dir.join("runtime_events.jsonl").exists());
}

#[test]
fn rust_queue_repair_lineage_reports_parse_and_target_failures_without_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let missing_root_spec = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue repair-lineage without root spec");
    assert_exit_code(&missing_root_spec, 2);
    assert_eq!(missing_root_spec.stdout, "");
    assert_eq!(
        missing_root_spec.stderr,
        "error: missing required option `--root-spec-id <ROOT_SPEC_ID>`\n"
    );

    let empty_root_spec = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id=",
    ])
    .expect("run Rust millrace queue repair-lineage empty root spec");
    assert_exit_code(&empty_root_spec, 2);
    assert_eq!(empty_root_spec.stdout, "");
    assert_eq!(
        empty_root_spec.stderr,
        "error: `--root-spec-id` value must not be empty\n"
    );

    let before = runtime_tree_snapshot(&root);
    let invalid_root_spec = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "../bad",
    ])
    .expect("run Rust millrace queue repair-lineage invalid root spec");
    assert_exit_code(&invalid_root_spec, 1);
    assert!(
        invalid_root_spec
            .stdout
            .contains("error: invalid root spec id")
    );
    assert_eq!(invalid_root_spec.stderr, "");
    assert_eq!(runtime_tree_snapshot(&root), before);

    let missing_target = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "missing-target",
    ])
    .expect("run Rust millrace queue repair-lineage missing target");
    assert_exit_code(&missing_target, 1);
    assert!(
        missing_target
            .stdout
            .contains("error: failed to load closure target: closure target does not exist at ")
    );
    assert_eq!(missing_target.stderr, "");

    fs::write(
        paths.arbiter_targets_dir.join("malformed-target.json"),
        "{\n",
    )
    .unwrap();
    let before_malformed = runtime_tree_snapshot(&root);
    let malformed_target = run_rust_millrace([
        "queue",
        "repair-lineage",
        "--workspace",
        root.to_str().unwrap(),
        "--root-spec-id",
        "malformed-target",
    ])
    .expect("run Rust millrace queue repair-lineage malformed target");
    assert_exit_code(&malformed_target, 1);
    assert!(
        malformed_target
            .stdout
            .contains("error: failed to load closure target: failed to decode closure target JSON")
    );
    assert_eq!(malformed_target.stderr, "");
    assert_eq!(runtime_tree_snapshot(&root), before_malformed);
}

#[test]
fn rust_queue_retry_blocked_requeues_retryable_task_refreshes_snapshot_and_audits() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let store = QueueStore::from_paths(paths.clone());
    fs::write(
        paths.tasks_queue_dir.join("task-retry.md"),
        runnable_task_markdown("task-retry"),
    )
    .unwrap();
    store.claim_next_execution_task(None).unwrap().unwrap();
    store.mark_task_blocked("task-retry").unwrap();
    write_blocked_task_metadata(&paths, "task-retry", "network_unavailable", true);

    let retry = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-retry",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        " retry after network_unavailable ",
    ])
    .expect("run Rust millrace queue retry-blocked");

    retry.assert_success();
    assert_eq!(retry.stderr, "");
    for expected in [
        "requeued_task: task-retry\n",
        "source_state: blocked\n",
        "destination_state: queue\n",
        "actor: operator\n",
        "auto: false\n",
        "attempt_number: 1\n",
        "failure_class: network_unavailable\n",
    ] {
        assert!(
            retry.stdout.contains(expected),
            "missing retry output fragment: {expected}\nstdout:\n{}",
            retry.stdout
        );
    }
    assert!(paths.tasks_queue_dir.join("task-retry.md").is_file());
    assert!(!paths.tasks_blocked_dir.join("task-retry.md").exists());

    let audit = read_json_lines(&paths.tasks_queue_dir.join("task-retry.requeue.jsonl"));
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["actor"], "operator");
    assert_eq!(audit[0]["attempt_number"], 1);
    assert_eq!(audit[0]["auto"], false);
    assert_eq!(audit[0]["destination_state"], "queue");
    assert_eq!(audit[0]["failure_class"], "network_unavailable");
    assert_eq!(audit[0]["kind"], "task");
    assert_eq!(audit[0]["reason"], "retry after network_unavailable");
    assert_eq!(audit[0]["source_state"], "blocked");

    let snapshot = load_snapshot(&paths).unwrap();
    assert_eq!(snapshot.queue_depth_execution, 1);
    assert_eq!(snapshot.queue_depth_planning, 0);
    let events = read_json_lines(&paths.logs_dir.join("runtime_events.jsonl"));
    assert!(events.iter().any(|event| {
        event["event_type"] == "blocked_task_requeued"
            && event["data"]["task_id"] == "task-retry"
            && event["data"]["actor"] == "operator"
            && event["data"]["auto"] == false
            && event["data"]["attempt_number"] == 1
            && event["data"]["failure_class"] == "network_unavailable"
    }));
}

#[test]
fn rust_queue_retry_blocked_refuses_guards_and_force_overrides_retryability() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    write_blocked_task(&paths, "task-semantic-blocked");
    write_blocked_task_metadata(
        &paths,
        "task-semantic-blocked",
        "missing_terminal_result",
        false,
    );
    let refused = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-semantic-blocked",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "manual retry",
    ])
    .expect("run Rust millrace queue retry-blocked nonretryable");
    assert_exit_code(&refused, 1);
    assert!(refused.stdout.contains(
        "error: failed to retry blocked task: blocked task is not retryable; rerun with --force to override\n"
    ));
    assert!(
        paths
            .tasks_blocked_dir
            .join("task-semantic-blocked.md")
            .is_file()
    );

    let forced = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-semantic-blocked",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator reviewed semantic block",
        "--force",
    ])
    .expect("run Rust millrace queue retry-blocked force");
    forced.assert_success();
    assert!(
        forced
            .stdout
            .contains("failure_class: missing_terminal_result\n")
    );
    assert!(
        paths
            .tasks_queue_dir
            .join("task-semantic-blocked.md")
            .is_file()
    );

    write_blocked_task(&paths, "task-missing-metadata");
    let missing_metadata = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-missing-metadata",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "missing metadata retry",
    ])
    .expect("run Rust millrace queue retry-blocked missing metadata");
    assert_exit_code(&missing_metadata, 1);
    assert!(
        missing_metadata
            .stdout
            .contains("blocked task is not retryable")
    );
    assert!(
        paths
            .tasks_blocked_dir
            .join("task-missing-metadata.md")
            .is_file()
    );

    write_blocked_task(&paths, "task-root-guard");
    write_blocked_task_metadata(&paths, "task-root-guard", "network_unavailable", true);
    let root_guard = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-root-guard",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "root mismatch",
        "--root-spec-id",
        "spec-other",
    ])
    .expect("run Rust millrace queue retry-blocked root guard");
    assert_exit_code(&root_guard, 1);
    assert!(
        root_guard
            .stdout
            .contains("blocked task task-root-guard does not belong to root spec spec-other")
    );
    assert!(paths.tasks_blocked_dir.join("task-root-guard.md").is_file());

    write_blocked_task(&paths, "task-budget");
    write_blocked_task_metadata(&paths, "task-budget", "network_unavailable", true);
    fs::write(
        paths.tasks_queue_dir.join("task-budget.requeue.jsonl"),
        "{\"auto\":true}\n{\"auto\":true}\n{\"auto\":true}\n",
    )
    .unwrap();
    let budget = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-budget",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "budget check",
    ])
    .expect("run Rust millrace queue retry-blocked budget");
    assert_exit_code(&budget, 1);
    assert!(
        budget
            .stdout
            .contains("blocked task retry budget is exhausted")
    );
    assert!(paths.tasks_blocked_dir.join("task-budget.md").is_file());

    write_blocked_task(&paths, "task-locked");
    write_blocked_task_metadata(&paths, "task-locked", "network_unavailable", true);
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("retry-blocked-lock"))
        .unwrap();
    let locked = run_rust_millrace([
        "queue",
        "retry-blocked",
        "task-locked",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "locked retry",
    ])
    .expect("run Rust millrace queue retry-blocked locked");
    assert_exit_code(&locked, 1);
    assert!(locked.stdout.contains(
        "error: failed to retry blocked task: active runtime ownership lock prevents blocked retry\n"
    ));
    assert!(paths.tasks_blocked_dir.join("task-locked.md").is_file());
}

#[test]
fn rust_cli_queue_operator_interventions_render_control_results_and_mutate_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    fs::write(
        paths.tasks_queue_dir.join("task-cancel-cli.md"),
        runnable_task_markdown("task-cancel-cli"),
    )
    .unwrap();
    let cancel = run_rust_millrace([
        "queue",
        "cancel",
        "task-cancel-cli",
        "--kind",
        "task",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator cancelled bad intake",
        "--force",
    ])
    .expect("run Rust millrace queue cancel");
    cancel.assert_success();
    for expected in [
        "action: cancel_work_item\n",
        "mode: direct\n",
        "applied: true\n",
    ] {
        assert!(
            cancel.stdout.contains(expected),
            "missing queue cancel output fragment {expected:?}\nstdout:\n{}",
            cancel.stdout
        );
    }
    assert!(!paths.tasks_queue_dir.join("task-cancel-cli.md").exists());
    assert!(archive_contains(
        &paths.tasks_queue_dir.join("cancelled"),
        "task-cancel-cli."
    ));

    write_blocked_task(&paths, "task-archive-cli");
    let archive = run_rust_millrace([
        "queue",
        "archive-blocked",
        "task-archive-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "obsolete blocked work",
    ])
    .expect("run Rust millrace queue archive-blocked");
    archive.assert_success();
    assert!(archive.stdout.contains("action: archive_blocked_task\n"));
    assert!(archive.stdout.contains("applied: true\n"));
    assert!(!paths.tasks_blocked_dir.join("task-archive-cli.md").exists());
    assert!(archive_contains(
        &paths.tasks_blocked_dir.join("cancelled"),
        "task-archive-cli."
    ));

    fs::write(
        paths.tasks_queue_dir.join("task-old-cli.md"),
        runnable_task_markdown("task-old-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-new-cli.md"),
        runnable_task_markdown("task-new-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-dependent-cli.md"),
        lineage_task_markdown("task-dependent-cli", "spec-root-001")
            .replace("task-prereq", "task-old-cli"),
    )
    .unwrap();
    let supersede = run_rust_millrace([
        "queue",
        "supersede",
        "task-old-cli",
        "--replacement",
        "task-new-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "replacement has corrected scope",
        "--cascade",
        "retarget",
    ])
    .expect("run Rust millrace queue supersede");
    supersede.assert_success();
    assert!(supersede.stdout.contains("action: supersede_task\n"));
    assert!(
        supersede
            .stdout
            .contains("affected_dependents=task-dependent-cli")
    );
    assert!(archive_contains(
        &paths.tasks_queue_dir.join("superseded"),
        "task-old-cli."
    ));
    let retargeted =
        fs::read_to_string(paths.tasks_queue_dir.join("task-dependent-cli.md")).unwrap();
    assert!(retargeted.contains("- task-new-cli\n"));

    fs::write(
        paths.tasks_queue_dir.join("task-old-cancel-cli.md"),
        runnable_task_markdown("task-old-cancel-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-new-cancel-cli.md"),
        runnable_task_markdown("task-new-cancel-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-dependent-cancel-cli.md"),
        lineage_task_markdown("task-dependent-cancel-cli", "spec-root-001")
            .replace("task-prereq", "task-old-cancel-cli"),
    )
    .unwrap();
    let cancel_cascade = run_rust_millrace([
        "queue",
        "supersede",
        "task-old-cancel-cli",
        "--replacement",
        "task-new-cancel-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "dependent should be cancelled",
        "--cascade",
        "cancel",
    ])
    .expect("run Rust millrace queue supersede cancel cascade");
    cancel_cascade.assert_success();
    assert!(archive_contains(
        &paths.tasks_queue_dir.join("cancelled"),
        "task-dependent-cancel-cli."
    ));

    fs::write(
        paths.tasks_queue_dir.join("task-old-manual-cli.md"),
        runnable_task_markdown("task-old-manual-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-new-manual-cli.md"),
        runnable_task_markdown("task-new-manual-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-retarget-cli.md"),
        lineage_task_markdown("task-retarget-cli", "spec-root-001")
            .replace("task-prereq", "task-old-manual-cli"),
    )
    .unwrap();
    let retarget = run_rust_millrace([
        "queue",
        "retarget-dependency",
        "task-retarget-cli",
        "--from",
        "task-old-manual-cli",
        "--to",
        "task-new-manual-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "replacement is queued",
    ])
    .expect("run Rust millrace queue retarget-dependency");
    retarget.assert_success();
    assert!(
        retarget
            .stdout
            .contains("action: retarget_task_dependency\n")
    );
    let manually_retargeted =
        fs::read_to_string(paths.tasks_queue_dir.join("task-retarget-cli.md")).unwrap();
    assert!(manually_retargeted.contains("- task-new-manual-cli\n"));
}

#[test]
fn rust_cli_incident_interventions_render_control_results_and_validate_filenames() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    fs::write(
        paths.incidents_incoming_dir.join("incident-resolve-cli.md"),
        lineage_incident_markdown("incident-resolve-cli", "spec-root-001"),
    )
    .unwrap();
    let resolve = run_rust_millrace([
        "incident",
        "resolve",
        "incident-resolve-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator resolved stale incident",
    ])
    .expect("run Rust millrace incident resolve");
    resolve.assert_success();
    assert!(resolve.stdout.contains("action: resolve_incident\n"));
    assert!(resolve.stdout.contains("applied: true\n"));
    assert!(archive_contains(
        &paths.incidents_resolved_dir.join("operator"),
        "incident-resolve-cli."
    ));

    fs::write(
        paths.incidents_active_dir.join("incident-cancel-cli.md"),
        lineage_incident_markdown("incident-cancel-cli", "spec-root-001"),
    )
    .unwrap();
    let cancel = run_rust_millrace([
        "incident",
        "cancel",
        "incident-cancel-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "incident came from superseded intake",
    ])
    .expect("run Rust millrace incident cancel");
    cancel.assert_success();
    assert!(cancel.stdout.contains("action: cancel_incident\n"));
    assert!(archive_contains(
        &paths.incidents_active_dir.join("cancelled"),
        "incident-cancel-cli."
    ));

    fs::write(
        paths
            .incidents_incoming_dir
            .join("incident-invalid.md.invalid"),
        "not a valid incident document",
    )
    .unwrap();
    let archive_invalid = run_rust_millrace([
        "incident",
        "archive-invalid",
        "incident-invalid.md.invalid",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "malformed generated incident artifact",
    ])
    .expect("run Rust millrace incident archive-invalid");
    archive_invalid.assert_success();
    assert!(
        archive_invalid
            .stdout
            .contains("action: archive_invalid_incident\n")
    );
    assert!(
        fs::read_dir(paths.incidents_incoming_dir.join("invalid-archived"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("incident-invalid.md.invalid.")))
    );

    let invalid_filename = run_rust_millrace([
        "incident",
        "archive-invalid",
        "../incident-invalid.md.invalid",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "unsafe filename",
    ])
    .expect("run Rust millrace incident archive-invalid unsafe filename");
    assert_exit_code(&invalid_filename, 1);
    assert!(invalid_filename.stdout.contains(
        "error: failed to archive invalid incident artifact: runtime control JSON contract error"
    ));
    assert!(
        invalid_filename
            .stdout
            .contains("filename is invalid: must be a single relative filename")
    );
}

#[test]
fn rust_queue_read_only_surfaces_intervention_archive_counts_and_lookup() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    fs::write(
        paths.tasks_queue_dir.join("task-cancel-readonly.md"),
        runnable_task_markdown("task-cancel-readonly"),
    )
    .unwrap();
    run_rust_millrace([
        "queue",
        "cancel",
        "task-cancel-readonly",
        "--kind",
        "task",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "cancel readonly task",
    ])
    .expect("run Rust millrace queue cancel readonly task")
    .assert_success();

    write_blocked_task(&paths, "task-archive-readonly");
    run_rust_millrace([
        "queue",
        "archive-blocked",
        "task-archive-readonly",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "archive readonly blocked task",
    ])
    .expect("run Rust millrace queue archive-blocked readonly task")
    .assert_success();

    fs::write(
        paths.tasks_queue_dir.join("task-old-readonly.md"),
        runnable_task_markdown("task-old-readonly"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-new-readonly.md"),
        runnable_task_markdown("task-new-readonly"),
    )
    .unwrap();
    run_rust_millrace([
        "queue",
        "supersede",
        "task-old-readonly",
        "--replacement",
        "task-new-readonly",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "supersede readonly task",
    ])
    .expect("run Rust millrace queue supersede readonly task")
    .assert_success();

    let probe = read_fixture("work_documents/probe.md")
        .unwrap()
        .replace("probe-fixture", "probe-cancel-readonly");
    fs::write(
        paths.probes_queue_dir.join("probe-cancel-readonly.md"),
        probe,
    )
    .unwrap();
    run_rust_millrace([
        "queue",
        "cancel",
        "probe-cancel-readonly",
        "--kind",
        "probe",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "cancel readonly probe",
    ])
    .expect("run Rust millrace queue cancel readonly probe")
    .assert_success();

    let spec = read_fixture("work_documents/spec.md")
        .unwrap()
        .replace("spec-fixture", "spec-cancel-readonly");
    fs::write(paths.specs_queue_dir.join("spec-cancel-readonly.md"), spec).unwrap();
    run_rust_millrace([
        "queue",
        "cancel",
        "spec-cancel-readonly",
        "--kind",
        "spec",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "cancel readonly spec",
    ])
    .expect("run Rust millrace queue cancel readonly spec")
    .assert_success();

    fs::write(
        paths
            .incidents_incoming_dir
            .join("incident-cancel-readonly.md"),
        lineage_incident_markdown("incident-cancel-readonly", "spec-root-001"),
    )
    .unwrap();
    run_rust_millrace([
        "incident",
        "cancel",
        "incident-cancel-readonly",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "cancel readonly incident",
    ])
    .expect("run Rust millrace incident cancel readonly")
    .assert_success();

    fs::write(
        paths
            .incidents_incoming_dir
            .join("incident-resolve-readonly.md"),
        lineage_incident_markdown("incident-resolve-readonly", "spec-root-001"),
    )
    .unwrap();
    run_rust_millrace([
        "incident",
        "resolve",
        "incident-resolve-readonly",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "resolve readonly incident",
    ])
    .expect("run Rust millrace incident resolve readonly")
    .assert_success();

    let queue = run_rust_millrace(["queue", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace queue ls readonly archives");
    queue.assert_success();
    for expected in [
        "cancelled_task_count: 2\n",
        "superseded_task_count: 1\n",
        "cancelled_incident_count: 1\n",
        "operator_resolved_incident_count: 1\n",
    ] {
        assert!(
            queue.stdout.contains(expected),
            "missing queue archive counter {expected:?}\nstdout:\n{}",
            queue.stdout
        );
    }

    for (work_item_id, kind, state) in [
        ("task-cancel-readonly", "task", "queue_cancelled"),
        ("task-archive-readonly", "task", "blocked_cancelled"),
        ("task-old-readonly", "task", "queue_superseded"),
        ("probe-cancel-readonly", "probe", "queue_cancelled"),
        ("spec-cancel-readonly", "spec", "queue_cancelled"),
        ("incident-cancel-readonly", "incident", "incoming_cancelled"),
        ("incident-resolve-readonly", "incident", "operator_resolved"),
    ] {
        let output = run_rust_millrace([
            "queue",
            "show",
            work_item_id,
            "--workspace",
            root.to_str().unwrap(),
        ])
        .expect("run Rust millrace queue show readonly archive");
        output.assert_success();
        assert!(
            output
                .stdout
                .contains(&format!("work_item_id: {work_item_id}\n"))
        );
        assert!(output.stdout.contains(&format!("work_item_kind: {kind}\n")));
        assert!(
            output
                .stdout
                .contains(&format!("work_item_state: {state}\n"))
        );
    }

    let status = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status latest intervention");
    status.assert_success();
    assert!(
        status
            .stdout
            .contains("latest_operator_intervention: event=incident_resolved_by_operator")
    );
    assert!(
        status
            .stdout
            .contains("work_item_id=incident-resolve-readonly")
    );
    assert!(
        status
            .stdout
            .contains("destination_path=millrace-agents/incidents/resolved/operator/")
    );

    let status_json = run_rust_millrace([
        "status",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
    ])
    .expect("run Rust millrace status JSON latest intervention");
    status_json.assert_success();
    let payload: Value =
        serde_json::from_str(&status_json.stdout).expect("parse status latest intervention JSON");
    assert_eq!(
        payload["latest_operator_intervention"]["event_type"],
        "incident_resolved_by_operator"
    );
    assert_eq!(
        payload["latest_operator_intervention"]["work_item_id"],
        "incident-resolve-readonly"
    );
}

#[test]
fn rust_cli_intervention_commands_route_to_mailbox_when_daemon_owns_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("cli-intervention"))
        .unwrap();

    let queue_cancel = run_rust_millrace([
        "queue",
        "cancel",
        "task-mailbox-cli",
        "--kind",
        "task",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator mailbox cancellation",
    ])
    .expect("run Rust millrace queue cancel mailbox");
    queue_cancel.assert_success();
    assert!(queue_cancel.stdout.contains("mode: mailbox\n"));
    assert!(queue_cancel.stdout.contains("applied: false\n"));

    let incident_resolve = run_rust_millrace([
        "incident",
        "resolve",
        "incident-mailbox-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator mailbox incident resolution",
    ])
    .expect("run Rust millrace incident resolve mailbox");
    incident_resolve.assert_success();
    assert!(incident_resolve.stdout.contains("mode: mailbox\n"));

    let envelopes = read_mailbox_json_values(&paths);
    let commands: BTreeSet<_> = envelopes
        .iter()
        .map(|envelope| envelope["command"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        BTreeSet::from(["cancel_work_item", "resolve_incident"])
    );
    assert!(envelopes.iter().any(|envelope| {
        envelope["command"] == "cancel_work_item"
            && envelope["payload"]["work_item_id"] == "task-mailbox-cli"
            && envelope["payload"]["work_item_kind"] == "task"
            && envelope["payload"]["reason"] == "operator mailbox cancellation"
    }));
    assert!(envelopes.iter().any(|envelope| {
        envelope["command"] == "resolve_incident"
            && envelope["payload"]["incident_id"] == "incident-mailbox-cli"
            && envelope["payload"]["reason"] == "operator mailbox incident resolution"
    }));
}

#[test]
fn rust_cli_intervention_commands_reject_empty_unsafe_ambiguous_and_invalid_targets() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    fs::write(
        paths.tasks_queue_dir.join("ambiguous-cli.md"),
        runnable_task_markdown("ambiguous-cli"),
    )
    .unwrap();
    fs::write(
        paths.specs_queue_dir.join("ambiguous-cli.md"),
        lineage_spec_markdown("ambiguous-cli", "spec-root-001"),
    )
    .unwrap();
    let ambiguous = run_rust_millrace([
        "queue",
        "cancel",
        "ambiguous-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator cancellation needs kind",
    ])
    .expect("run Rust millrace queue cancel ambiguous");
    assert_exit_code(&ambiguous, 1);
    assert!(
        ambiguous
            .stdout
            .contains("work item id is ambiguous; pass --kind for ambiguous-cli")
    );

    let empty_reason = run_rust_millrace([
        "queue",
        "cancel",
        "ambiguous-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "",
    ])
    .expect("run Rust millrace queue cancel empty reason");
    assert_exit_code(&empty_reason, 2);
    assert_eq!(
        empty_reason.stderr,
        "error: `--reason` value must not be empty\n"
    );

    let unsafe_id = run_rust_millrace([
        "queue",
        "cancel",
        "../escape",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "unsafe cancellation",
    ])
    .expect("run Rust millrace queue cancel unsafe id");
    assert_exit_code(&unsafe_id, 1);
    assert!(unsafe_id.stdout.contains("failed to cancel work item"));
    assert!(unsafe_id.stdout.contains("work_item_id"));

    fs::write(
        paths
            .tasks_queue_dir
            .join("task-missing-replacement-cli.md"),
        runnable_task_markdown("task-missing-replacement-cli"),
    )
    .unwrap();
    let missing_replacement = run_rust_millrace([
        "queue",
        "supersede",
        "task-missing-replacement-cli",
        "--replacement",
        "task-replacement-missing-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "replacement should exist",
    ])
    .expect("run Rust millrace queue supersede missing replacement");
    assert_exit_code(&missing_replacement, 1);
    assert!(
        missing_replacement.stdout.contains(
            "replacement task is not queued, active, or done: task-replacement-missing-cli"
        )
    );

    fs::write(
        paths.tasks_queue_dir.join("task-no-old-dependency-cli.md"),
        runnable_task_markdown("task-no-old-dependency-cli"),
    )
    .unwrap();
    fs::write(
        paths.tasks_queue_dir.join("task-new-dependency-cli.md"),
        runnable_task_markdown("task-new-dependency-cli"),
    )
    .unwrap();
    let absent_dependency = run_rust_millrace([
        "queue",
        "retarget-dependency",
        "task-no-old-dependency-cli",
        "--from",
        "task-absent-dependency-cli",
        "--to",
        "task-new-dependency-cli",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "dependency should exist",
    ])
    .expect("run Rust millrace queue retarget absent dependency");
    assert_exit_code(&absent_dependency, 1);
    assert!(
        absent_dependency.stdout.contains(
            "task task-no-old-dependency-cli does not depend on task-absent-dependency-cli"
        )
    );
}

#[test]
fn rust_cli_direct_intervention_refuses_active_runtime_stage_without_mailbox() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    fs::write(
        paths.tasks_queue_dir.join("task-active-refusal-cli.md"),
        runnable_task_markdown("task-active-refusal-cli"),
    )
    .unwrap();
    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.active_plane = Some(Plane::Execution);
    snapshot.active_stage = Some(StageName::Builder);
    snapshot.active_node_id = Some("builder".to_owned());
    snapshot.active_stage_kind_id = Some("builder".to_owned());
    snapshot.active_run_id = Some("run-active-refusal".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Task);
    snapshot.active_work_item_id = Some("task-active-refusal-cli".to_owned());
    snapshot.active_since = Some(timestamp("2026-04-15T00:00:00Z"));
    save_snapshot(&paths, &snapshot).unwrap();

    let refused = run_rust_millrace([
        "queue",
        "cancel",
        "task-active-refusal-cli",
        "--kind",
        "task",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "do not mutate active runtime",
    ])
    .expect("run Rust millrace queue cancel active-stage refusal");
    refused.assert_success();
    assert!(refused.stdout.contains("mode: direct\n"));
    assert!(refused.stdout.contains("applied: false\n"));
    assert!(
        refused
            .stdout
            .contains("active runtime stage prevents operator intervention")
    );
    assert!(
        paths
            .tasks_queue_dir
            .join("task-active-refusal-cli.md")
            .is_file()
    );
    assert!(read_mailbox_json_values(&paths).is_empty());
}

#[test]
fn rust_run_once_public_surface_is_rejected_without_workspace_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let uninitialized = run_rust_millrace(["run", "once", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace removed run once command before init");
    assert_exit_code(&uninitialized, 2);
    assert_eq!(uninitialized.stdout, "");
    assert_eq!(uninitialized.stderr, "error: unknown run command `once`\n");
    assert!(!root.join("millrace-agents").exists());

    run_init_for(&root);
    let before = runtime_tree_snapshot(&root);
    let initialized = run_rust_millrace(["run", "once", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace removed run once command after init");
    assert_exit_code(&initialized, 2);
    assert_eq!(initialized.stdout, "");
    assert_eq!(initialized.stderr, "error: unknown run command `once`\n");
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_run_daemon_max_ticks_one_executes_one_fake_runner_tick_and_run_views_inspect_artifacts() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap().to_owned();
    let config_path = paths.runtime_root.join("millrace.toml");
    write_mock_codex_runtime_config(&paths, temp_dir.path());

    let task_path = temp_dir.path().join("task-run-once-cli.md");
    fs::write(&task_path, runnable_task_markdown("task-run-once-cli")).unwrap();
    run_rust_millrace([
        "queue",
        "add-task",
        task_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue add-task for bounded daemon")
    .assert_success();

    let before_daemon = runtime_tree_snapshot(&root);
    let daemon = run_rust_millrace(vec![
        "run".to_owned(),
        "daemon".to_owned(),
        "--workspace".to_owned(),
        workspace.clone(),
        "--mode".to_owned(),
        "standard_plain".to_owned(),
        "--config".to_owned(),
        config_path.to_string_lossy().into_owned(),
        "--max-ticks".to_owned(),
        "1".to_owned(),
    ])
    .expect("run Rust millrace run daemon bounded tick");

    daemon.assert_success();
    assert_eq!(daemon.stderr, "");
    for expected in [
        "run_mode: daemon\n",
        "active_mode_id: default_codex\n",
        "mode_override: standard_plain\n",
        "exit_reason: max_ticks\n",
        "runtime_ticks: 1\n",
        "ticks: 1\n",
        "daemon_ownership_acquired: true\n",
        "runtime_ownership_release_ok: true\n",
        "runtime_ownership_released: true\n",
    ] {
        assert!(
            daemon.stdout.contains(expected),
            "missing expected daemon output fragment: {expected}\nstdout:\n{}",
            daemon.stdout
        );
    }
    assert!(!daemon.stdout.contains("non-executing Rust CLI placeholder"));
    assert_ne!(runtime_tree_snapshot(&root), before_daemon);
    assert!(
        paths
            .tasks_active_dir
            .join("task-run-once-cli.md")
            .is_file()
    );
    assert!(!paths.runtime_lock_file.exists());

    let list = run_rust_millrace(["runs", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace runs ls after bounded daemon");
    list.assert_success();
    let run_id = stdout_line_value(&list.stdout, "run_id: ");
    let before_views = runtime_tree_snapshot(&root);
    assert!(list.stdout.contains(&format!("run_id: {run_id}\n")));
    assert!(list.stdout.contains("status: valid\n"));
    assert!(list.stdout.contains("runtime_outcome: complete\n"));
    assert!(list.stdout.contains("work_item_kind: task\n"));

    let show = run_rust_millrace([
        "runs",
        "show",
        run_id,
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show after bounded daemon");
    show.assert_success();
    for expected in [
        "stage_result_count: 1\n",
        "stage: builder\n",
        "terminal_result: BUILDER_COMPLETE\n",
        "result_class: success\n",
        "primary_tail_artifact: runner_stdout.request-",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing expected runs show fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }

    let tail = run_rust_millrace([
        "runs",
        "tail",
        run_id,
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail after bounded daemon");
    tail.assert_success();
    assert_eq!(tail.stdout, "### BUILDER_COMPLETE\n\n");
    assert_eq!(runtime_tree_snapshot(&root), before_views);
}

#[test]
fn rust_run_daemon_max_ticks_one_reports_no_work_paused_and_stopped_outcomes() {
    for (setup, expected_exit_reason) in [
        ("none", "max_ticks"),
        ("pause", "max_ticks"),
        ("stop", "stop_requested"),
    ] {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("workspace");
        run_init_for(&root);
        let paths = workspace_paths(&root);

        match setup {
            "pause" => {
                run_rust_millrace(["control", "pause", "--workspace", root.to_str().unwrap()])
                    .expect("run Rust millrace control before bounded daemon")
                    .assert_success();
            }
            "stop" => {
                let mut snapshot = load_snapshot(&paths).unwrap();
                snapshot.stop_requested = true;
                save_snapshot(&paths, &snapshot).unwrap();
            }
            _ => {}
        }

        let output = run_rust_millrace([
            "run",
            "daemon",
            "--workspace",
            root.to_str().unwrap(),
            "--max-ticks",
            "1",
        ])
        .expect("run Rust millrace run daemon bounded non-dispatch outcome");
        output.assert_success();
        assert_eq!(output.stderr, "");
        for expected in [
            "run_mode: daemon\n".to_owned(),
            format!("exit_reason: {expected_exit_reason}\n"),
            "runtime_ticks: 1\n".to_owned(),
            "ticks: 1\n".to_owned(),
            "daemon_ownership_acquired: true\n".to_owned(),
            "runtime_ownership_release_ok: true\n".to_owned(),
            "runtime_ownership_released: true\n".to_owned(),
        ] {
            assert!(
                output.stdout.contains(&expected),
                "missing expected daemon output fragment: {expected}\nstdout:\n{}",
                output.stdout
            );
        }
        assert!(!paths.runtime_lock_file.exists());
    }
}

#[test]
fn rust_run_daemon_reports_startup_compile_failure_and_releases_ownership() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let output = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "missing_mode",
        "--max-ticks",
        "1",
    ])
    .expect("run Rust millrace run daemon invalid mode startup failure");

    assert_exit_code(&output, 1);
    assert_eq!(output.stderr, "");
    for expected in [
        "error: millrace run daemon startup failed: ",
        "run_mode: daemon\n",
        "mode_override: missing_mode\n",
        "startup_failed: true\n",
        "daemon_ownership_acquired: false\n",
        "runtime_ticks: 0\n",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected startup-failure output fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    assert!(!paths.runtime_lock_file.exists());
}

#[test]
fn rust_run_daemon_default_stdout_is_quiet_except_summary_lines() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let daemon = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--max-ticks",
        "1",
    ])
    .expect("run Rust millrace run daemon default monitor");

    daemon.assert_success();
    assert_eq!(daemon.stderr, "");
    assert!(daemon.stdout.contains("run_mode: daemon\n"));
    assert!(daemon.stdout.contains("exit_reason: max_ticks\n"));
    assert!(daemon.stdout.contains("runtime_ticks: 1\n"));
    assert!(daemon.stdout.contains("ticks: 1\n"));
    assert!(daemon.stdout.contains("runtime_ownership_released: true\n"));
    assert!(
        !daemon.stdout.contains("["),
        "default daemon stdout should not contain live monitor lines\nstdout:\n{}",
        daemon.stdout
    );
    let snapshot = load_snapshot(&paths).expect("load snapshot after daemon no-work tick");
    assert_eq!(
        snapshot.runtime_mode,
        millrace_ai::contracts::RuntimeMode::Daemon
    );
    assert!(!snapshot.process_running);
    assert!(
        fs::read_to_string(paths.logs_dir.join("runtime_events.jsonl"))
            .expect("read daemon runtime events")
            .contains("\"event_type\":\"runtime_tick_idle\"")
    );
    assert_eq!(
        millrace_ai::workspace::inspect_runtime_ownership_lock(&paths).state,
        millrace_ai::workspace::RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn rust_run_daemon_bounded_execution_uses_fake_runner_and_run_views() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    write_mock_codex_runtime_config(&paths, temp_dir.path());
    fs::write(
        paths.tasks_queue_dir.join("task-daemon-cli.md"),
        runnable_task_markdown("task-daemon-cli"),
    )
    .unwrap();

    let daemon = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "default_codex",
        "--config",
        paths.runtime_config_file.to_str().unwrap(),
        "--max-ticks",
        "1",
    ])
    .expect("run Rust millrace run daemon bounded fake-runner dispatch");

    daemon.assert_success();
    assert_eq!(daemon.stderr, "");
    for expected in [
        "run_mode: daemon\n",
        "active_mode_id: default_codex\n",
        "mode_override: default_codex\n",
        "exit_reason: max_ticks\n",
        "runtime_ticks: 1\n",
        "ticks: 1\n",
    ] {
        assert!(
            daemon.stdout.contains(expected),
            "missing expected daemon output fragment: {expected}\nstdout:\n{}",
            daemon.stdout
        );
    }
    assert!(
        daemon
            .stdout
            .contains("compiled_plan_id: plan-default_codex-")
    );
    assert!(!daemon.stdout.contains("non-executing Rust CLI placeholder"));

    let list = run_rust_millrace(["runs", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace runs ls after daemon");
    list.assert_success();
    assert!(list.stdout.contains("status: valid\n"));
    let run_id = stdout_line_value(&list.stdout, "run_id: ");

    let show = run_rust_millrace([
        "runs",
        "show",
        run_id,
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show after daemon");
    show.assert_success();
    assert!(show.stdout.contains("stage: builder\n"));

    let tail = run_rust_millrace([
        "runs",
        "tail",
        run_id,
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail after daemon");
    tail.assert_success();
    assert_eq!(tail.stdout, "### BUILDER_COMPLETE\n\n");
    assert!(!tail.stdout.to_ascii_lowercase().contains("codex"));
    assert!(!tail.stdout.to_ascii_lowercase().contains("pi runner"));
    assert_eq!(
        millrace_ai::workspace::inspect_runtime_ownership_lock(&paths).state,
        millrace_ai::workspace::RuntimeOwnershipLockState::Absent
    );
}

#[test]
fn rust_run_daemon_basic_monitor_prints_live_lines_to_stdout() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    write_mock_codex_runtime_config(&paths, temp_dir.path());
    fs::write(
        paths.tasks_queue_dir.join("task-daemon-monitor.md"),
        runnable_task_markdown("task-daemon-monitor"),
    )
    .unwrap();

    let daemon = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--max-ticks",
        "1",
        "--monitor",
        "BaSiC",
    ])
    .expect("run Rust millrace run daemon basic monitor");

    daemon.assert_success();
    assert_eq!(daemon.stderr, "");
    assert!(daemon.stdout.contains("runtime started mode="));
    assert!(daemon.stdout.contains("stage start execution/builder"));
    assert!(daemon.stdout.contains("stage done execution/builder"));
    assert!(daemon.stdout.contains("route execution"));
    assert!(daemon.stdout.contains("run_mode: daemon\n"));
    assert!(
        daemon.stdout.find("stage start execution/builder").unwrap()
            < daemon.stdout.find("stage done execution/builder").unwrap()
    );
    assert!(
        daemon.stdout.find("stage done execution/builder").unwrap()
            < daemon.stdout.find("route execution").unwrap()
    );
}

#[test]
fn rust_run_daemon_monitor_log_fanout_does_not_change_stdout_mode() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    write_mock_codex_runtime_config(&paths, temp_dir.path());
    fs::write(
        paths.tasks_queue_dir.join("task-daemon-log-monitor.md"),
        runnable_task_markdown("task-daemon-log-monitor"),
    )
    .unwrap();
    let monitor_log = temp_dir
        .path()
        .join("nested")
        .join("logs")
        .join("monitor.log");

    let daemon = run_rust_millrace(vec![
        "run".to_owned(),
        "daemon".to_owned(),
        "--workspace".to_owned(),
        root.to_string_lossy().into_owned(),
        "--max-ticks".to_owned(),
        "1".to_owned(),
        "--monitor-log".to_owned(),
        monitor_log.to_string_lossy().into_owned(),
    ])
    .expect("run Rust millrace run daemon monitor log");

    daemon.assert_success();
    assert_eq!(daemon.stderr, "");
    assert!(daemon.stdout.contains("run_mode: daemon\n"));
    assert!(
        !daemon.stdout.contains("stage start execution/builder"),
        "monitor-log alone must not enable stdout monitor mode\nstdout:\n{}",
        daemon.stdout
    );
    let log = fs::read_to_string(&monitor_log).expect("read daemon monitor log");
    assert!(log.contains("runtime started mode="));
    assert!(log.contains("stage start execution/builder"));
    assert!(log.contains("stage done execution/builder"));
    assert!(log.contains("route execution"));
}

#[test]
fn rust_run_daemon_reports_lock_contention_without_mutating_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("daemon-session"))
        .expect("acquire active runtime lock");
    let before = runtime_tree_snapshot(&root);
    let daemon = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--max-ticks",
        "1",
    ])
    .expect("run Rust millrace run daemon with active lock");

    assert_exit_code(&daemon, 1);
    assert_eq!(daemon.stderr, "");
    for expected in [
        "error: millrace run daemon startup failed: ",
        "run_mode: daemon\n",
        "mode_override: none\n",
        "startup_failed: true\n",
        "daemon_ownership_acquired: false\n",
        "runtime_ticks: 0\n",
    ] {
        assert!(
            daemon.stdout.contains(expected),
            "missing expected daemon lock-contention fragment: {expected}\nstdout:\n{}",
            daemon.stdout
        );
    }
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_upgrade_preview_is_read_only_and_python_shaped() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let workspace = root.to_str().unwrap();
    let before = runtime_tree_snapshot(&root);

    let output =
        run_rust_millrace(["upgrade", "--workspace", workspace]).expect("run Rust upgrade preview");

    output.assert_success();
    for expected in [
        "applied: false\n",
        "baseline_manifest_id: ",
        "candidate_manifest_id: ",
        "safe_package_update: 0\n",
        "local_only_modification: 0\n",
        "already_converged: 0\n",
        "localized_removed: 0\n",
        "conflict: 0\n",
        "missing: 0\n",
        "entry: entrypoints/execution/builder.md unchanged\n",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected upgrade output fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    assert_eq!(output.stderr, "");
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_upgrade_apply_updates_safe_and_missing_assets() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();
    let mut manifest = load_baseline_manifest(&paths).unwrap();
    let candidate_manifest = build_baseline_manifest();
    let safe_path = "entrypoints/execution/builder.md";
    let missing_path = "modes/default_codex.json";
    let candidate_safe_bytes = fs::read(paths.runtime_root.join(safe_path)).unwrap();
    let old_safe_bytes = b"old package builder\n";

    fs::write(paths.runtime_root.join(safe_path), old_safe_bytes).unwrap();
    manifest
        .entries
        .iter_mut()
        .find(|entry| entry.relative_path == safe_path)
        .unwrap()
        .original_sha256 = sha256_hex(old_safe_bytes);
    manifest
        .entries
        .retain(|entry| entry.relative_path != missing_path);
    fs::remove_file(paths.runtime_root.join(missing_path)).unwrap();
    manifest.manifest_id = "workspace-old-baseline".to_owned();
    write_baseline_manifest(&paths, &manifest).unwrap();

    let output = run_rust_millrace(["upgrade", "--workspace", workspace, "--apply"])
        .expect("run Rust upgrade apply");

    output.assert_success();
    for expected in [
        "applied: true\n",
        "result_manifest_id: ",
        "safe_package_update: 1\n",
        "missing: 1\n",
        "entry: entrypoints/execution/builder.md safe_package_update\n",
        "entry: modes/default_codex.json missing\n",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected apply output fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    assert_eq!(
        fs::read(paths.runtime_root.join(safe_path)).unwrap(),
        candidate_safe_bytes
    );
    assert!(paths.runtime_root.join(missing_path).is_file());
    assert_eq!(load_baseline_manifest(&paths).unwrap(), candidate_manifest);
}

#[test]
fn rust_upgrade_apply_refuses_conflicts_and_preserves_files() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();
    let mut manifest = load_baseline_manifest(&paths).unwrap();
    let managed_path = "entrypoints/execution/builder.md";
    let operator_bytes = b"operator edit\n";

    manifest
        .entries
        .iter_mut()
        .find(|entry| entry.relative_path == managed_path)
        .unwrap()
        .original_sha256 = sha256_hex(b"old package\n");
    manifest.manifest_id = "workspace-conflict-baseline".to_owned();
    write_baseline_manifest(&paths, &manifest).unwrap();
    fs::write(paths.runtime_root.join(managed_path), operator_bytes).unwrap();
    let manifest_before = fs::read(&paths.baseline_manifest_file).unwrap();

    let output = run_rust_millrace(["upgrade", "--workspace", workspace, "--apply"])
        .expect("run conflicting Rust upgrade apply");

    assert_exit_code(&output, 1);
    assert!(
        output
            .stdout
            .contains("error: upgrade conflict(s) detected: entrypoints/execution/builder.md\n")
    );
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read(paths.runtime_root.join(managed_path)).unwrap(),
        operator_bytes
    );
    assert_eq!(
        fs::read(&paths.baseline_manifest_file).unwrap(),
        manifest_before
    );
}

#[test]
fn rust_upgrade_localizes_removed_assets_and_validates_requests() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();
    let removed_path = "entrypoints/removed-package.md";
    let removed_bytes = b"operator keeps this removed package file\n";
    let mut manifest = load_baseline_manifest(&paths).unwrap();

    fs::write(paths.runtime_root.join(removed_path), removed_bytes).unwrap();
    manifest.entries.push(BaselineManifestEntry {
        relative_path: removed_path.to_owned(),
        asset_family: "entrypoints".to_owned(),
        original_sha256: sha256_hex(removed_bytes),
    });
    manifest.manifest_id = "workspace-removed-baseline".to_owned();
    write_baseline_manifest(&paths, &manifest).unwrap();
    let list_file = temp_dir.path().join("localize-removed.txt");
    fs::write(
        &list_file,
        "# keep local copy\nentrypoints/removed-package.md\n\n",
    )
    .unwrap();

    let blocked = run_rust_millrace(["upgrade", "--workspace", workspace, "--apply"])
        .expect("run removed-asset upgrade without localization");
    assert_exit_code(&blocked, 1);
    assert!(blocked.stdout.contains("upgrade conflict(s) detected"));

    let preview = run_rust_millrace([
        "upgrade",
        "--workspace",
        workspace,
        "--localize-removed-from",
        list_file.to_str().unwrap(),
    ])
    .expect("run removed-asset localization preview");
    preview.assert_success();
    assert!(preview.stdout.contains("localized_removed: 1\n"));
    assert!(
        preview
            .stdout
            .contains("entry: entrypoints/removed-package.md localized_removed\n")
    );

    let invalid = run_rust_millrace([
        "upgrade",
        "--workspace",
        workspace,
        "--localize-removed",
        "entrypoints/not-removed.md",
    ])
    .expect("run invalid removed-asset localization preview");
    assert_exit_code(&invalid, 1);
    assert!(
        invalid
            .stdout
            .contains("localize-removed path is not a removed managed asset")
    );

    let apply = run_rust_millrace([
        "upgrade",
        "--workspace",
        workspace,
        "--apply",
        "--localize-removed",
        removed_path,
    ])
    .expect("run removed-asset localization apply");
    apply.assert_success();
    assert!(apply.stdout.contains("applied: true\n"));
    assert_eq!(
        fs::read(paths.runtime_root.join(removed_path)).unwrap(),
        removed_bytes
    );
    assert!(
        load_baseline_manifest(&paths)
            .unwrap()
            .entry_for(removed_path)
            .is_none()
    );
}

#[test]
fn rust_upgrade_reports_manifest_failures_and_uninitialized_workspaces() {
    let temp_dir = TempDir::new().unwrap();
    let uninitialized = temp_dir.path().join("fresh");
    let uninitialized_output =
        run_rust_millrace(["upgrade", "--workspace", uninitialized.to_str().unwrap()])
            .expect("run Rust upgrade on uninitialized workspace");

    assert_exit_code(&uninitialized_output, 1);
    assert!(
        uninitialized_output
            .stdout
            .contains("workspace is not initialized")
    );
    assert!(!uninitialized.join("millrace-agents").exists());

    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    fs::write(&paths.baseline_manifest_file, "{not-json").unwrap();
    let malformed = run_rust_millrace(["upgrade", "--workspace", root.to_str().unwrap()])
        .expect("run Rust upgrade with malformed manifest");

    assert_exit_code(&malformed, 1);
    assert!(malformed.stdout.contains("baseline_manifest"));
    assert_eq!(malformed.stderr, "");

    fs::remove_file(&paths.baseline_manifest_file).unwrap();
    let missing = run_rust_millrace(["upgrade", "--workspace", root.to_str().unwrap()])
        .expect("run Rust upgrade with missing manifest");

    assert_exit_code(&missing, 1);
    assert!(missing.stdout.contains("workspace is not initialized"));
}

#[test]
fn rust_control_commands_apply_offline_state_and_render_python_shaped_results() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();

    let cases = [
        (vec!["pause"], "pause", "runtime paused directly"),
        (
            vec!["control", "resume"],
            "resume",
            "runtime resumed directly",
        ),
        (vec!["stop"], "stop", "runtime stopped directly"),
        (
            vec!["control", "retry-active"],
            "retry_active",
            "no active work item to retry",
        ),
        (
            vec!["clear-stale-state"],
            "clear_stale_state",
            "cleared stale runtime state; requeued=0; runtime_ownership_lock=missing",
        ),
        (
            vec!["reload-config"],
            "reload_config",
            "no daemon running; reload request not enqueued",
        ),
    ];

    for (mut args, action, detail) in cases {
        args.extend(["--workspace", workspace]);
        let output = run_rust_millrace(args).expect("run Rust millrace control command");

        output.assert_success();
        assert_eq!(output.stderr, "");
        assert!(output.stdout.contains(&format!("action: {action}\n")));
        assert!(output.stdout.contains("mode: direct\n"));
        assert!(output.stdout.contains(&format!("detail: {detail}\n")));
        assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());
    }
}

#[test]
fn rust_control_and_config_reload_route_to_mailbox_when_daemon_lock_is_active() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("daemon-session"))
        .unwrap();

    let pause = run_rust_millrace(["control", "pause", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace control pause with active lock");
    pause.assert_success();
    for expected in [
        "action: pause\n",
        "mode: mailbox\n",
        "applied: false\n",
        "detail: queued for daemon processing\n",
        "command_id: pause-",
        "mailbox_path: ",
    ] {
        assert!(
            pause.stdout.contains(expected),
            "missing expected pause output fragment: {expected}\nstdout:\n{}",
            pause.stdout
        );
    }

    let reload = run_rust_millrace(["config", "reload", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace config reload with active lock");
    reload.assert_success();
    for expected in [
        "action: reload_config\n",
        "mode: mailbox\n",
        "applied: false\n",
        "detail: queued for daemon processing on the next runtime tick\n",
        "command_id: reload_config-",
    ] {
        assert!(
            reload.stdout.contains(expected),
            "missing expected reload output fragment: {expected}\nstdout:\n{}",
            reload.stdout
        );
    }

    let commands: Vec<_> = mailbox_json_paths(&paths.mailbox_incoming_dir)
        .iter()
        .map(|path| {
            serde_json::from_str::<Value>(&fs::read_to_string(path).unwrap()).unwrap()["command"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(commands, vec!["pause", "reload_config"]);
    assert!(!load_snapshot(&paths).unwrap().paused);
}

#[test]
fn rust_approvals_cli_lists_shows_and_applies_direct_decisions() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let approval_id = create_pending_approval(&paths, "request-cli-approval");

    let list = run_rust_millrace(["approvals", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace approvals ls");
    list.assert_success();
    assert_eq!(list.stderr, "");
    for expected in [
        format!("approval_id: {approval_id}\n"),
        "status: pending\n".to_owned(),
        "capability_id: package.install\n".to_owned(),
        "grant_id: grant-package-install\n".to_owned(),
        "run_id: run-approval\n".to_owned(),
        "request_id: request-cli-approval\n".to_owned(),
        "work_item_id: task-approval\n".to_owned(),
    ] {
        assert!(
            list.stdout.contains(&expected),
            "missing approvals ls output fragment: {expected}\nstdout:\n{}",
            list.stdout
        );
    }

    let show = run_rust_millrace([
        "approvals",
        "show",
        &approval_id,
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace approvals show");
    show.assert_success();
    assert_eq!(show.stderr, "");
    let shown: Value = serde_json::from_str(&show.stdout).unwrap();
    assert_eq!(shown["approval_id"], approval_id);
    assert_eq!(shown["status"], "pending");
    assert_eq!(shown["grant"]["capability_id"], "package.install");

    let approve = run_rust_millrace([
        "approvals",
        "approve",
        &approval_id,
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator approved package install",
    ])
    .expect("run Rust millrace approvals approve");
    approve.assert_success();
    assert_eq!(approve.stderr, "");
    assert!(
        approve
            .stdout
            .contains("action: approve_execution_capability\n")
    );
    assert!(approve.stdout.contains("mode: direct\n"));
    assert!(approve.stdout.contains("applied: true\n"));
    assert!(approve.stdout.contains(&format!(
        "detail: approved execution capability request {approval_id}\n"
    )));
    let listing = list_execution_capability_approvals(&paths).unwrap();
    assert!(listing.pending.is_empty());
    assert_eq!(listing.resolved[0].status.as_str(), "approved");
}

#[test]
fn rust_approvals_cli_routes_daemon_owned_decisions_to_mailbox() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let approval_id = create_pending_approval(&paths, "request-cli-mailbox-approval");
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("approval-daemon"))
        .unwrap();

    let deny = run_rust_millrace([
        "approvals",
        "deny",
        &approval_id,
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "operator denied package install",
    ])
    .expect("run Rust millrace approvals deny with active lock");
    deny.assert_success();
    assert_eq!(deny.stderr, "");
    assert!(deny.stdout.contains("action: deny_execution_capability\n"));
    assert!(deny.stdout.contains("mode: mailbox\n"));
    assert!(deny.stdout.contains("applied: false\n"));
    assert!(
        deny.stdout
            .contains("detail: queued for daemon processing\n")
    );

    let mailbox_paths = mailbox_json_paths(&paths.mailbox_incoming_dir);
    assert_eq!(mailbox_paths.len(), 1);
    let envelope: Value = serde_json::from_str(&fs::read_to_string(&mailbox_paths[0]).unwrap())
        .expect("mailbox envelope JSON");
    assert_eq!(envelope["command"], "deny_execution_capability");
    assert_eq!(envelope["payload"]["approval_id"], approval_id);
    assert_eq!(
        envelope["payload"]["reason"],
        "operator denied package install"
    );
    let listing = list_execution_capability_approvals(&paths).unwrap();
    assert_eq!(listing.pending.len(), 1);
    assert!(listing.resolved.is_empty());
}

#[test]
fn rust_control_treats_invalid_and_stale_locks_as_offline() {
    let temp_dir = TempDir::new().unwrap();

    for (workspace_name, write_lock) in [
        ("invalid-lock-workspace", "invalid"),
        ("stale-lock-workspace", "stale"),
    ] {
        let root = temp_dir.path().join(workspace_name);
        run_init_for(&root);
        let paths = workspace_paths(&root);
        if write_lock == "invalid" {
            fs::write(&paths.runtime_lock_file, "{not-valid-json").unwrap();
        } else {
            acquire_runtime_ownership_lock_with_options(
                &paths,
                RuntimeOwnershipLockOptions::new(
                    u32::MAX,
                    "test-host",
                    "stale-daemon-session",
                    "2026-04-15T00:00:00Z",
                )
                .unwrap(),
            )
            .unwrap();
        }

        let output = run_rust_millrace(["control", "pause", "--workspace", root.to_str().unwrap()])
            .expect("run Rust millrace control pause with non-active lock");

        output.assert_success();
        assert!(output.stdout.contains("action: pause\n"));
        assert!(output.stdout.contains("mode: direct\n"));
        assert!(output.stdout.contains("detail: runtime paused directly\n"));
        assert!(mailbox_json_paths(&paths.mailbox_incoming_dir).is_empty());
    }
}

#[test]
fn rust_planning_retry_active_uses_plane_scoped_runtime_control_boundary() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    fs::write(
        paths.specs_active_dir.join("spec-planning-retry.md"),
        read_fixture("work_documents/spec.md")
            .unwrap()
            .replace("spec-fixture", "spec-planning-retry"),
    )
    .unwrap();

    let mut snapshot = load_snapshot(&paths).unwrap();
    let active_since = Timestamp::parse("active_since", "2026-04-15T00:00:00Z").unwrap();
    snapshot.active_runs_by_plane.insert(
        Plane::Planning,
        ActiveRunState {
            plane: Plane::Planning,
            lane_id: String::new(),
            stage: StageName::Manager,
            node_id: "manager".to_owned(),
            stage_kind_id: "manager".to_owned(),
            run_id: "run-planning-retry".to_owned(),
            compiled_plan_id: String::new(),
            compiled_plan_fingerprint: String::new(),
            request_kind: ActiveRunRequestKind::ActiveWorkItem,
            work_item_family_id: None,
            work_item_kind: Some(WorkItemKind::Spec),
            work_item_id: Some("spec-planning-retry".to_owned()),
            closure_target_root_spec_id: None,
            closure_target_root_idea_id: None,
            active_since: active_since.clone(),
            running_status_marker: None,
        },
    );
    snapshot.active_plane = Some(Plane::Planning);
    snapshot.active_stage = Some(StageName::Manager);
    snapshot.active_node_id = Some("manager".to_owned());
    snapshot.active_stage_kind_id = Some("manager".to_owned());
    snapshot.active_run_id = Some("run-planning-retry".to_owned());
    snapshot.active_work_item_kind = Some(WorkItemKind::Spec);
    snapshot.active_work_item_id = Some("spec-planning-retry".to_owned());
    snapshot.active_since = Some(active_since);
    save_snapshot(&paths, &snapshot).unwrap();

    let output = run_rust_millrace([
        "planning",
        "retry-active",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "planning retry",
    ])
    .expect("run Rust millrace planning retry-active");

    output.assert_success();
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("action: retry_active\n"));
    assert!(output.stdout.contains("mode: direct\n"));
    assert!(output.stdout.contains("applied: true\n"));
    assert!(
        output
            .stdout
            .contains("detail: active spec spec-planning-retry requeued\n")
    );
    assert!(
        paths
            .specs_queue_dir
            .join("spec-planning-retry.md")
            .is_file()
    );
    assert!(
        !paths
            .specs_active_dir
            .join("spec-planning-retry.md")
            .exists()
    );
}

#[test]
fn rust_config_validate_compiles_selected_or_explicit_config_modes() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let alias = run_rust_millrace([
        "config",
        "validate",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
        "standard_plain",
    ])
    .expect("run Rust millrace config validate alias");
    alias.assert_success();
    assert_eq!(alias.stderr, "");
    assert!(alias.stdout.contains("ok: true\n"));
    assert!(alias.stdout.contains("mode_id: default_codex\n"));
    assert!(alias.stdout.contains("used_last_known_good: false\n"));
    assert!(paths.compiled_plan_file.is_file());
    assert!(paths.compile_diagnostics_file.is_file());

    let config_root = temp_dir.path().join("config-workspace");
    run_init_for(&config_root);
    let config_paths = workspace_paths(&config_root);
    let custom_config = temp_dir.path().join("learning-config.toml");
    fs::write(
        &custom_config,
        [
            "[runtime]",
            "default_mode = \"learning_codex\"",
            "run_style = \"daemon\"",
            "",
        ]
        .join("\n"),
    )
    .unwrap();

    let explicit = run_rust_millrace([
        "config",
        "validate",
        "--workspace",
        config_root.to_str().unwrap(),
        "--config",
        custom_config.to_str().unwrap(),
    ])
    .expect("run Rust millrace config validate explicit config");
    explicit.assert_success();
    assert!(explicit.stdout.contains("ok: true\n"));
    assert!(explicit.stdout.contains("mode_id: learning_codex\n"));
    let plan: Value =
        serde_json::from_str(&fs::read_to_string(&config_paths.compiled_plan_file).unwrap())
            .unwrap();
    assert_eq!(plan["mode_id"], "learning_codex");

    let invalid = run_rust_millrace([
        "config",
        "validate",
        "--workspace",
        config_root.to_str().unwrap(),
        "--mode",
        "missing_mode",
    ])
    .expect("run Rust millrace config validate invalid mode");
    assert_exit_code(&invalid, 1);
    assert!(invalid.stdout.contains("ok: false\n"));
    assert!(invalid.stdout.contains("mode_id: missing_mode\n"));
    assert!(invalid.stdout.contains("error: "));

    let invalid_runner_config = temp_dir.path().join("invalid-runner-config.toml");
    fs::write(
        &invalid_runner_config,
        "[runners.codex]\npermission_default = \"root\"\n",
    )
    .unwrap();
    let invalid_config = run_rust_millrace([
        "config",
        "validate",
        "--workspace",
        config_root.to_str().unwrap(),
        "--config",
        invalid_runner_config.to_str().unwrap(),
    ])
    .expect("run Rust millrace config validate invalid runner config");
    assert_exit_code(&invalid_config, 1);
    assert!(
        invalid_config
            .stdout
            .contains("runners.codex.permission_default"),
        "stdout did not include invalid config field: {}",
        invalid_config.stdout
    );

    let invalid_auto_recovery_config = temp_dir.path().join("invalid-auto-recovery-config.toml");
    fs::write(
        &invalid_auto_recovery_config,
        "[auto_recovery]\ncooldown_seconds = []\n",
    )
    .unwrap();
    let invalid_auto_recovery = run_rust_millrace([
        "config",
        "validate",
        "--workspace",
        config_root.to_str().unwrap(),
        "--config",
        invalid_auto_recovery_config.to_str().unwrap(),
    ])
    .expect("run Rust millrace config validate invalid auto-recovery config");
    assert_exit_code(&invalid_auto_recovery, 1);
    assert!(
        invalid_auto_recovery
            .stdout
            .contains("auto_recovery.cooldown_seconds"),
        "stdout did not include invalid auto-recovery config field: {}",
        invalid_auto_recovery.stdout
    );
}

#[test]
fn rust_queue_intake_commands_write_canonical_artifacts_and_are_visible() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let task_path = temp_dir.path().join("task-cli-intake.md");
    let task_raw = read_fixture("work_documents/task.md")
        .unwrap()
        .replace("task-fixture", "task-cli-intake");
    fs::write(&task_path, task_raw).unwrap();

    let task = run_rust_millrace([
        "queue",
        "add-task",
        task_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue add-task");
    task.assert_success();
    assert_eq!(
        task.stdout,
        format!(
            "enqueued_task: {}\n",
            paths.tasks_queue_dir.join("task-cli-intake.md").display()
        )
    );
    assert_eq!(task.stderr, "");
    let queued_task = fs::read_to_string(paths.tasks_queue_dir.join("task-cli-intake.md")).unwrap();
    assert!(queued_task.contains("Task-ID: task-cli-intake\n"));
    assert!(queued_task.starts_with("# Fixture task\n"));

    let spec_path = temp_dir.path().join("spec-cli-intake.json");
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": "1.0",
            "kind": "spec",
            "spec_id": "spec-cli-intake",
            "title": "Spec CLI intake",
            "summary": "Import through Rust CLI JSON intake",
            "source_type": "manual",
            "root_idea_id": "idea-cli-intake",
            "root_spec_id": "spec-cli-intake",
            "goals": ["prove spec intake"],
            "constraints": ["stay deterministic"],
            "acceptance": ["queue add-spec imports JSON"],
            "references": ["tests/parity_cli.rs"],
            "created_at": "2026-04-15T00:00:00Z",
            "created_by": "tests"
        }))
        .unwrap(),
    )
    .unwrap();

    let spec = run_rust_millrace([
        "add-spec",
        spec_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-spec alias");
    spec.assert_success();
    assert_eq!(
        spec.stdout,
        format!(
            "enqueued_spec: {}\n",
            paths.specs_queue_dir.join("spec-cli-intake.md").display()
        )
    );
    assert_eq!(spec.stderr, "");
    let queued_spec = fs::read_to_string(paths.specs_queue_dir.join("spec-cli-intake.md")).unwrap();
    assert!(queued_spec.starts_with("# Spec CLI intake\n"));
    assert!(queued_spec.contains("Spec-ID: spec-cli-intake\n"));
    assert!(queued_spec.contains("Root-Idea-ID: idea-cli-intake\n"));

    let probe_path = temp_dir.path().join("probe-cli-intake.json");
    fs::write(
        &probe_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": "1.0",
            "kind": "probe",
            "probe_id": "probe-cli-intake",
            "title": "Probe CLI intake",
            "summary": "Import through Rust CLI JSON intake",
            "request": "Research the current codebase and route the smallest safe change.",
            "target_paths": ["src/cli/intake.rs"],
            "constraints": ["Do not implement during recon."],
            "acceptance": ["queue add-probe imports JSON"],
            "risk_notes": ["probe intake can drift from task/spec intake"],
            "references": ["tests/parity_cli.rs"],
            "created_at": "2026-04-15T00:00:00Z",
            "created_by": "tests"
        }))
        .unwrap(),
    )
    .unwrap();

    let probe = run_rust_millrace([
        "add-probe",
        probe_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-probe alias");
    probe.assert_success();
    assert_eq!(
        probe.stdout,
        format!(
            "enqueued_probe: {}\n",
            paths.probes_queue_dir.join("probe-cli-intake.md").display()
        )
    );
    assert_eq!(probe.stderr, "");
    let queued_probe =
        fs::read_to_string(paths.probes_queue_dir.join("probe-cli-intake.md")).unwrap();
    assert!(queued_probe.starts_with("# Probe CLI intake\n"));
    assert!(queued_probe.contains("Probe-ID: probe-cli-intake\n"));
    assert!(queued_probe.contains("Request: Research the current codebase"));

    let idea_path = temp_dir.path().join("idea-cli-intake.md");
    fs::write(
        &idea_path,
        "# CLI intake idea\n\nBuild the queue intake surface.\n",
    )
    .unwrap();
    let idea = run_rust_millrace([
        "add-idea",
        idea_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-idea alias");
    idea.assert_success();
    assert_eq!(
        idea.stdout,
        format!(
            "enqueued_idea: {}\n",
            root.join("ideas/inbox/idea-cli-intake.md").display()
        )
    );
    assert_eq!(idea.stderr, "");
    assert_eq!(
        fs::read_to_string(root.join("ideas/inbox/idea-cli-intake.md")).unwrap(),
        "# CLI intake idea\n\nBuild the queue intake surface.\n"
    );

    let list = run_rust_millrace(["queue", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace queue ls after intake");
    list.assert_success();
    for expected in [
        "execution_queue_depth: 1\n",
        "planning_queue_depth: 2\n",
        "probe_queue_depth: 1\n",
        "spec_queue_depth: 1\n",
        "incident_queue_depth: 0\n",
        "task_queue_count: 1\n",
        "probe_queue_count: 1\n",
        "spec_queue_count: 1\n",
        "work_item: kind=task state=queue id=task-cli-intake path=",
        "work_item: kind=probe state=queue id=probe-cli-intake path=",
        "work_item: kind=spec state=queue id=spec-cli-intake path=",
    ] {
        assert!(
            list.stdout.contains(expected),
            "missing expected queue output fragment: {expected}\nstdout:\n{}",
            list.stdout
        );
    }
}

#[test]
fn rust_queue_intake_routes_to_mailbox_when_daemon_lock_is_active() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("daemon-session"))
        .unwrap();

    let task_path = temp_dir.path().join("task-mailbox-cli.md");
    let task_raw = read_fixture("work_documents/task.md")
        .unwrap()
        .replace("task-fixture", "task-mailbox-cli");
    fs::write(&task_path, task_raw).unwrap();

    let output = run_rust_millrace([
        "queue",
        "add-task",
        task_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue add-task mailbox");

    output.assert_success();
    assert_eq!(output.stderr, "");
    for expected in [
        "action: add_task\n",
        "mode: mailbox\n",
        "applied: false\n",
        "detail: queued for daemon processing\n",
        "command_id: add_task-",
        "mailbox_path: ",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected mailbox output fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    assert!(!paths.tasks_queue_dir.join("task-mailbox-cli.md").exists());

    let probe_path = temp_dir.path().join("probe-mailbox-cli.md");
    let probe_raw = read_fixture("work_documents/probe.md")
        .unwrap()
        .replace("probe-fixture", "probe-mailbox-cli");
    fs::write(&probe_path, probe_raw).unwrap();

    let probe_output = run_rust_millrace([
        "queue",
        "add-probe",
        probe_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue add-probe mailbox");
    probe_output.assert_success();
    assert_eq!(probe_output.stderr, "");
    for expected in [
        "action: add_probe\n",
        "mode: mailbox\n",
        "applied: false\n",
        "detail: queued for daemon processing\n",
        "command_id: add_probe-",
        "mailbox_path: ",
    ] {
        assert!(
            probe_output.stdout.contains(expected),
            "missing expected mailbox output fragment: {expected}\nstdout:\n{}",
            probe_output.stdout
        );
    }
    assert!(!paths.probes_queue_dir.join("probe-mailbox-cli.md").exists());

    let mailbox_paths = mailbox_json_paths(&paths.mailbox_incoming_dir);
    assert_eq!(mailbox_paths.len(), 2);
    let envelopes: Vec<Value> = mailbox_paths
        .iter()
        .map(|path| serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    let add_task = envelopes
        .iter()
        .find(|envelope| envelope["command"] == "add_task")
        .unwrap();
    assert_eq!(add_task["kind"], "mailbox_command");
    assert_eq!(
        add_task["payload"]["document"]["task_id"],
        "task-mailbox-cli"
    );
    let add_probe = envelopes
        .iter()
        .find(|envelope| envelope["command"] == "add_probe")
        .unwrap();
    assert_eq!(add_probe["kind"], "mailbox_command");
    assert_eq!(
        add_probe["payload"]["document"]["probe_id"],
        "probe-mailbox-cli"
    );
}

#[test]
fn rust_queue_intake_rejects_invalid_duplicate_mismatched_and_uninitialized_inputs() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let mismatch_path = temp_dir.path().join("task-source-name.md");
    fs::write(
        &mismatch_path,
        read_fixture("work_documents/task.md")
            .unwrap()
            .replace("task-fixture", "task-document-id"),
    )
    .unwrap();
    let mismatch = run_rust_millrace([
        "queue",
        "add-task",
        mismatch_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-task mismatch");
    assert_exit_code(&mismatch, 1);
    assert!(mismatch.stdout.contains("failed to add task"));
    assert!(
        mismatch
            .stdout
            .contains("filename stem does not match task_id")
    );
    assert!(!paths.tasks_queue_dir.join("task-document-id.md").exists());

    let malformed_path = temp_dir.path().join("task-malformed.md");
    fs::write(&malformed_path, "# Malformed\n").unwrap();
    let malformed = run_rust_millrace([
        "queue",
        "add-task",
        malformed_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-task malformed");
    assert_exit_code(&malformed, 1);
    assert!(malformed.stdout.contains("failed to add task"));
    assert!(
        malformed
            .stdout
            .contains("must include one canonical document identifier")
    );

    let unsafe_path = temp_dir.path().join("bad-task.md");
    fs::write(
        &unsafe_path,
        read_fixture("work_documents/task.md")
            .unwrap()
            .replace("task-fixture", "../bad"),
    )
    .unwrap();
    let unsafe_id = run_rust_millrace([
        "queue",
        "add-task",
        unsafe_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-task unsafe");
    assert_exit_code(&unsafe_id, 1);
    assert!(unsafe_id.stdout.contains("task_id"));
    assert!(!paths.tasks_dir.join("bad.md").exists());

    let duplicate_path = temp_dir.path().join("task-duplicate-cli.md");
    fs::write(
        &duplicate_path,
        read_fixture("work_documents/task.md")
            .unwrap()
            .replace("task-fixture", "task-duplicate-cli"),
    )
    .unwrap();
    run_rust_millrace([
        "queue",
        "add-task",
        duplicate_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-task duplicate first")
    .assert_success();
    let duplicate = run_rust_millrace([
        "queue",
        "add-task",
        duplicate_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-task duplicate second");
    assert_exit_code(&duplicate, 1);
    assert!(
        duplicate
            .stdout
            .contains("task task-duplicate-cli already exists")
    );

    let duplicate_probe_path = temp_dir.path().join("probe-duplicate-cli.md");
    fs::write(
        &duplicate_probe_path,
        read_fixture("work_documents/probe.md")
            .unwrap()
            .replace("probe-fixture", "probe-duplicate-cli"),
    )
    .unwrap();
    run_rust_millrace([
        "queue",
        "add-probe",
        duplicate_probe_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-probe duplicate first")
    .assert_success();
    let duplicate_probe = run_rust_millrace([
        "queue",
        "add-probe",
        duplicate_probe_path.to_str().unwrap(),
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-probe duplicate second");
    assert_exit_code(&duplicate_probe, 1);
    assert!(duplicate_probe.stdout.contains("failed to add probe"));
    assert!(
        duplicate_probe
            .stdout
            .contains("probe probe-duplicate-cli already exists")
    );

    let uninitialized_root = temp_dir.path().join("uninitialized");
    let uninitialized_path = temp_dir.path().join("task-uninitialized-cli.md");
    fs::write(
        &uninitialized_path,
        read_fixture("work_documents/task.md")
            .unwrap()
            .replace("task-fixture", "task-uninitialized-cli"),
    )
    .unwrap();
    let uninitialized = run_rust_millrace([
        "queue",
        "add-task",
        uninitialized_path.to_str().unwrap(),
        "--workspace",
        uninitialized_root.to_str().unwrap(),
    ])
    .expect("run Rust millrace add-task uninitialized");
    assert_exit_code(&uninitialized, 1);
    assert!(
        uninitialized
            .stdout
            .starts_with("error: workspace is not initialized: ")
    );
    assert_eq!(uninitialized.stderr, "");
    assert!(!uninitialized_root.join("millrace-agents").exists());
}

#[test]
fn rust_read_only_queue_commands_render_documents_without_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let task = read_fixture("work_documents/task.md").unwrap();
    fs::write(paths.tasks_queue_dir.join("task-fixture.md"), &task).unwrap();
    fs::write(
        paths.tasks_active_dir.join("task-active.md"),
        task.replace("task-fixture", "task-active"),
    )
    .unwrap();
    fs::write(
        paths.tasks_done_dir.join("task-done.md"),
        task.replace("task-fixture", "task-done"),
    )
    .unwrap();
    fs::write(
        paths.tasks_blocked_dir.join("task-blocked.md"),
        task.replace("task-fixture", "task-blocked"),
    )
    .unwrap();

    let probe = read_fixture("work_documents/probe.md").unwrap();
    fs::write(paths.probes_queue_dir.join("probe-fixture.md"), &probe).unwrap();
    fs::write(
        paths.probes_active_dir.join("probe-active.md"),
        probe.replace("probe-fixture", "probe-active"),
    )
    .unwrap();
    fs::write(
        paths.probes_done_dir.join("probe-done.md"),
        probe.replace("probe-fixture", "probe-done"),
    )
    .unwrap();
    fs::write(
        paths.probes_blocked_dir.join("probe-blocked.md"),
        probe.replace("probe-fixture", "probe-blocked"),
    )
    .unwrap();

    let spec = read_fixture("work_documents/spec.md").unwrap();
    fs::write(paths.specs_queue_dir.join("spec-fixture.md"), &spec).unwrap();
    fs::write(
        paths.specs_blocked_dir.join("spec-blocked.md"),
        spec.replace("spec-fixture", "spec-blocked"),
    )
    .unwrap();

    let incident = read_fixture("work_documents/incident.md").unwrap();
    fs::write(
        paths.incidents_incoming_dir.join("inc-fixture.md"),
        &incident,
    )
    .unwrap();
    fs::write(
        paths.incidents_resolved_dir.join("inc-resolved.md"),
        incident.replace("inc-fixture", "inc-resolved"),
    )
    .unwrap();

    let before = runtime_tree_snapshot(&root);
    let output = run_rust_millrace(["queue", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace queue ls");

    output.assert_success();
    assert_eq!(output.stderr, "");
    for expected in [
        "execution_queue_depth: 1\n",
        "planning_queue_depth: 3\n",
        "probe_queue_depth: 1\n",
        "spec_queue_depth: 1\n",
        "incident_queue_depth: 1\n",
        "planning_active: 1\n",
        "active_task_count: 1\n",
        "active_probe_count: 1\n",
        "task_done_count: 1\n",
        "task_blocked_count: 1\n",
        "probe_done_count: 1\n",
        "probe_blocked_count: 1\n",
        "spec_blocked_count: 1\n",
        "incident_resolved_count: 1\n",
        "work_item: kind=task state=queue id=task-fixture path=",
        "work_item: kind=probe state=done id=probe-done path=",
    ] {
        assert!(
            output.stdout.contains(expected),
            "missing expected queue output fragment: {expected}\nstdout:\n{}",
            output.stdout
        );
    }
    assert_eq!(runtime_tree_snapshot(&root), before);

    let output = run_rust_millrace([
        "queue",
        "show",
        "task-fixture",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue show");

    output.assert_success();
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("work_item_id: task-fixture\n"));
    assert!(output.stdout.contains("work_item_kind: task\n"));
    assert!(output.stdout.contains("work_item_state: queue\n"));
    assert!(output.stdout.contains("title: Fixture task\n"));
    assert_eq!(runtime_tree_snapshot(&root), before);

    let output = run_rust_millrace([
        "queue",
        "show",
        "probe-done",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace queue show probe");

    output.assert_success();
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("work_item_id: probe-done\n"));
    assert!(output.stdout.contains("work_item_kind: probe\n"));
    assert!(output.stdout.contains("work_item_state: done\n"));
    assert!(output.stdout.contains("title: Fixture probe\n"));
    assert!(
        output
            .stdout
            .contains("request: Research the current codebase and route the smallest safe change.")
    );
    assert!(output.stdout.contains("status_hint: queued\n"));
    assert!(
        output
            .stdout
            .contains("target_paths: [\"src/example/parser.py\"]\n")
    );
    assert_eq!(runtime_tree_snapshot(&root), before);

    fs::write(
        paths.tasks_queue_dir.join("task-malformed.md"),
        "# Malformed\n",
    )
    .unwrap();
    let before_malformed_check = runtime_tree_snapshot(&root);
    let output = run_rust_millrace(["queue", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace queue ls malformed");

    assert_exit_code(&output, 1);
    assert!(output.stdout.contains("error: queue document error at "));
    assert!(output.stdout.contains("task-malformed.md"));
    assert_eq!(output.stderr, "");
    assert!(paths.tasks_queue_dir.join("task-malformed.md").is_file());
    assert!(
        !paths
            .tasks_queue_dir
            .join("task-malformed.md.invalid")
            .exists()
    );
    assert!(
        !paths
            .tasks_queue_dir
            .join("invalid-artifacts.jsonl")
            .exists()
    );
    assert_eq!(runtime_tree_snapshot(&root), before_malformed_check);
}

#[test]
fn rust_status_config_and_modes_read_only_commands_render_without_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let before = runtime_tree_snapshot(&root);

    let status = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status");
    status.assert_success();
    assert_eq!(status.stderr, "");
    for expected in [
        &format!("workspace: {}\n", paths.root.display()),
        "runtime_mode: daemon\n",
        "process_running: false\n",
        "runtime_ownership_lock: absent\n",
        "compiled_plan_currentness: missing\n",
        "execution_queue_depth: 0\n",
        "usage_governance_enabled: false\n",
        "blocked_idle: false\n",
        "latest_runtime_error_report_path: none\n",
        "latest_operator_intervention: none\n",
        "closure_target_root_spec_id: none\n",
    ] {
        assert!(
            status.stdout.contains(expected),
            "missing expected status output fragment: {expected}\nstdout:\n{}",
            status.stdout
        );
    }

    let status_json = run_rust_millrace([
        "status",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
    ])
    .expect("run Rust millrace status json");
    status_json.assert_success();
    assert_eq!(status_json.stderr, "");
    let status_payload: Value =
        serde_json::from_str(&status_json.stdout).expect("parse status JSON payload");
    assert_eq!(
        status_payload["workspace"],
        paths.root.display().to_string()
    );
    assert_eq!(status_payload["runtime_mode"], "daemon");
    assert_eq!(status_payload["process_running"], false);
    assert_eq!(status_payload["execution_queue_depth"], 0);
    assert_eq!(status_payload["planning_queue_depth"], 0);
    assert_eq!(status_payload["closure_target_root_spec_id"], Value::Null);
    assert_eq!(status_payload["blocked_idle"], false);
    assert_eq!(
        status_payload["latest_runtime_error_report_path"],
        Value::Null
    );
    assert_eq!(status_payload["latest_operator_intervention"], Value::Null);

    let watch = run_rust_millrace([
        "status",
        "watch",
        "--workspace",
        root.to_str().unwrap(),
        "--max-updates",
        "2",
        "--interval-seconds",
        "0",
    ])
    .expect("run Rust millrace status watch");
    watch.assert_success();
    assert_eq!(
        watch
            .stdout
            .matches(&format!("workspace: {}", paths.root.display()))
            .count(),
        2
    );

    let config = run_rust_millrace(["config", "show", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace config show");
    config.assert_success();
    assert_eq!(config.stderr, "");
    for expected in [
        "default_mode: default_codex\n",
        "run_style: daemon\n",
        "idle_sleep_seconds: 1.0\n",
        "runners.default_runner: codex_cli\n",
        "runners.codex.permission_default: maximum\n",
        "runners.pi.event_log_policy: failure_full\n",
        "watchers.enabled: true\n",
        "auto_recovery.enabled: true\n",
        "usage_governance.enabled: false\n",
        "execution_capabilities.enabled: true\n",
        "execution_capabilities.allow_advisory_grants: true\n",
        "execution_capabilities.fail_required_advisory: false\n",
        "config_version: bootstrap\n",
        "last_reload_outcome: none\n",
    ] {
        assert!(
            config.stdout.contains(expected),
            "missing expected config output fragment: {expected}\nstdout:\n{}",
            config.stdout
        );
    }

    let custom_config_path = temp_dir.path().join("runner-config.toml");
    fs::write(
        &custom_config_path,
        [
            "[runners]",
            "default_runner = \"pi_rpc\"",
            "",
            "[runners.codex]",
            "command = \"codex-dev\"",
            "args = [\"exec\", \"--trace\"]",
            "permission_by_stage = { builder = \"basic\" }",
            "",
            "[runners.pi]",
            "event_log_policy = \"full\"",
            "disable_skills = false",
            "",
            "[auto_recovery]",
            "enabled = false",
            "blocked_dependency_retry_enabled = false",
            "max_auto_requeues_per_work_item = 0",
            "cooldown_seconds = [0, 300]",
            "",
            "[execution_capabilities]",
            "enabled = false",
            "allow_advisory_grants = false",
            "fail_required_advisory = true",
            "",
            "[stages.builder]",
            "runner = \"pi_rpc\"",
            "timeout_seconds = 45",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let custom_config = run_rust_millrace([
        "config",
        "show",
        "--workspace",
        root.to_str().unwrap(),
        "--config",
        custom_config_path.to_str().unwrap(),
    ])
    .expect("run Rust millrace config show custom runner config");
    custom_config.assert_success();
    for expected in [
        "runners.default_runner: pi_rpc\n",
        "runners.codex.command: codex-dev\n",
        "runners.codex.args: [\"exec\",\"--trace\"]\n",
        "runners.codex.permission_by_stage: {\"builder\":\"basic\"}\n",
        "runners.pi.disable_skills: false\n",
        "runners.pi.event_log_policy: full\n",
        "auto_recovery.enabled: false\n",
        "execution_capabilities.enabled: false\n",
        "execution_capabilities.allow_advisory_grants: false\n",
        "execution_capabilities.fail_required_advisory: true\n",
        "stages.count: 1\n",
    ] {
        assert!(
            custom_config.stdout.contains(expected),
            "missing expected custom config output fragment: {expected}\nstdout:\n{}",
            custom_config.stdout
        );
    }
    for non_exposed_key in [
        "auto_recovery.blocked_dependency_retry_enabled:",
        "auto_recovery.max_auto_requeues_per_work_item:",
        "auto_recovery.cooldown_seconds:",
    ] {
        assert!(
            !custom_config.stdout.contains(non_exposed_key),
            "config show should not invent a Rust-only auto-recovery output key `{non_exposed_key}`\nstdout:\n{}",
            custom_config.stdout
        );
    }

    let modes = run_rust_millrace(["modes", "list"]).expect("run Rust millrace modes list");
    modes.assert_success();
    assert!(modes.stdout.contains(
        "default_codex: execution_loop=execution.standard planning_loop=planning.standard\n"
    ));
    assert!(modes.stdout.contains(
        "default_codex_integrated: execution_loop=execution.with_integrator planning_loop=planning.standard\n"
    ));
    assert!(modes.stdout.contains(
        "learning_codex_integrated: execution_loop=execution.with_integrator planning_loop=planning.standard\n"
    ));
    assert!(
        modes
            .stdout
            .contains("standard_plain -> default_codex (compatibility alias)\n")
    );

    let mode = run_rust_millrace(["modes", "show", "standard_plain"])
        .expect("run Rust millrace modes show");
    mode.assert_success();
    assert!(mode.stdout.contains("alias_of: default_codex\n"));
    assert!(mode.stdout.contains("mode_id: default_codex\n"));

    let integrated_mode = run_rust_millrace(["modes", "show", "learning_codex_integrated"])
        .expect("run Rust millrace integrated modes show");
    integrated_mode.assert_success();
    assert!(
        integrated_mode
            .stdout
            .contains("mode_id: learning_codex_integrated\n")
    );
    assert!(
        integrated_mode
            .stdout
            .contains("execution_loop_id: execution.with_integrator\n")
    );
    assert!(
        integrated_mode
            .stdout
            .contains("planning_loop_id: planning.standard\n")
    );
    assert!(
        integrated_mode
            .stdout
            .contains("learning_loop_id: learning.standard\n")
    );

    assert_eq!(runtime_tree_snapshot(&root), before);
    assert!(!paths.compiled_plan_file.exists());

    let governance_root = temp_dir.path().join("workspace-governance");
    run_init_for(&governance_root);
    let governance_paths = workspace_paths(&governance_root);
    fs::write(
        &governance_paths.usage_governance_state_file,
        serde_json::json!({
            "version": "1.0",
            "enabled": true,
            "auto_resume": true,
            "auto_resume_possible": true,
            "evaluation_boundary": "between_stages",
            "calendar_timezone": "UTC",
            "daemon_session_id": "daemon-session",
            "last_evaluated_at": "2026-04-28T20:00:00Z",
            "active_blockers": [{
                "source": "subscription_quota",
                "rule_id": "quota-five-hour-test",
                "window": "five_hour",
                "observed": 96.0,
                "threshold": 95.0,
                "metric": null,
                "auto_resume_possible": true,
                "next_auto_resume_at": "2026-04-28T21:00:00Z",
                "detail": ""
            }],
            "paused_by_governance": true,
            "next_auto_resume_at": "2026-04-28T21:00:00Z",
            "subscription_quota_status": {
                "enabled": true,
                "provider": "codex_chatgpt_oauth",
                "state": "healthy",
                "degraded_policy": "fail_open",
                "detail": null,
                "last_refreshed_at": "2026-04-28T20:00:00Z",
                "windows": {
                    "five_hour": {
                        "window": "five_hour",
                        "percent_used": 96.0,
                        "resets_at": "2026-04-28T21:00:00Z",
                        "read_at": "2026-04-28T20:00:00Z"
                    }
                }
            }
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    let before_governance_status = runtime_tree_snapshot(&governance_root);
    let governance_status =
        run_rust_millrace(["status", "--workspace", governance_root.to_str().unwrap()])
            .expect("run Rust millrace status with governance state");
    governance_status.assert_success();
    for expected in [
        "usage_governance_enabled: true\n",
        "usage_governance_paused: true\n",
        "usage_governance_blocker_count: 1\n",
        "usage_governance_subscription_status: healthy\n",
        "usage_governance_blocker: source=subscription_quota rule=quota-five-hour-test window=five_hour observed=96 threshold=95 auto_resume_possible=true next_resume=2026-04-28T21:00:00Z detail=none\n",
    ] {
        assert!(
            governance_status.stdout.contains(expected),
            "missing expected governance status fragment: {expected}\nstdout:\n{}",
            governance_status.stdout
        );
    }
    assert_eq!(
        runtime_tree_snapshot(&governance_root),
        before_governance_status
    );
}

#[test]
fn rust_status_prefers_actionable_closure_target_and_counts_deferred_roots() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let mut blocked = closure_target_state("spec-root-blocked", "idea-blocked");
    blocked.closure_blocked_by_lineage_work = true;
    blocked.blocking_work_ids = vec!["task-blocked".to_owned()];
    save_closure_target_state(&paths, &blocked).unwrap();
    save_closure_target_state(
        &paths,
        &closure_target_state("spec-root-actionable", "idea-actionable"),
    )
    .unwrap();
    fs::write(
        paths.specs_queue_dir.join("spec-root-deferred.md"),
        lineage_spec_markdown("spec-root-deferred", "spec-root-deferred"),
    )
    .unwrap();
    let before = runtime_tree_snapshot(&root);

    let status = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status with closure targets");

    status.assert_success();
    assert_eq!(status.stderr, "");
    assert!(!status.stdout.contains("invalid_multiple_open_targets"));
    assert!(
        status
            .stdout
            .contains("closure_target_root_spec_id: spec-root-actionable\n")
    );
    assert!(
        status
            .stdout
            .contains("closure_target_blocked_by_lineage_work: false\n")
    );
    assert!(
        status
            .stdout
            .contains("planning_root_specs_deferred_by_closure_target: 1\n")
    );
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_status_reports_multiple_actionable_closure_targets_as_invalid() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    save_closure_target_state(
        &paths,
        &closure_target_state("spec-root-actionable-a", "idea-actionable-a"),
    )
    .unwrap();
    save_closure_target_state(
        &paths,
        &closure_target_state("spec-root-actionable-b", "idea-actionable-b"),
    )
    .unwrap();

    let status = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status with ambiguous closure targets");

    status.assert_success();
    assert_eq!(status.stderr, "");
    assert!(
        status
            .stdout
            .contains("closure_target_root_spec_id: invalid_multiple_actionable_open_targets\n")
    );
    assert!(status.stdout.contains("closure_target_open: invalid\n"));
}

#[test]
fn rust_status_json_reports_blocked_idle_and_runtime_error_context_read_only() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);

    let mut target = closure_target_state("spec-root-blocked", "idea-blocked");
    target.closure_blocked_by_lineage_work = true;
    target.blocking_work_ids = vec!["task-blocked".to_owned()];
    save_closure_target_state(&paths, &target).unwrap();

    let report_path = paths
        .runs_dir
        .join("run-runtime-error")
        .join("runtime_error_report.md");
    fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    fs::write(&report_path, "# Runtime Error\n").unwrap();
    fs::write(
        &paths.runtime_error_context_file,
        serde_json::to_string_pretty(&json!({
            "schema_version": "1.0",
            "kind": "runtime_error_context",
            "error_code": "planning_post_stage_apply_failed",
            "plane": "planning",
            "failed_stage": "manager",
            "repair_stage": "mechanic",
            "work_item_kind": "spec",
            "work_item_id": "spec-root-blocked",
            "run_id": "run-runtime-error",
            "router_action": "route_to_mechanic",
            "terminal_result": "BLOCKED",
            "stage_result_path": "millrace-agents/runs/run-runtime-error/stage_results/request-001.json",
            "report_path": report_path.display().to_string(),
            "exception_type": "RuntimeError",
            "exception_message": "post-stage apply failed",
            "captured_at": "2026-04-15T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();

    let mut snapshot = load_snapshot(&paths).unwrap();
    snapshot.runtime_mode = RuntimeMode::Daemon;
    snapshot.process_running = true;
    snapshot.planning_status_marker = "### BLOCKED".to_owned();
    snapshot.current_failure_class = Some("recon_handoff_invalid".to_owned());
    snapshot.updated_at = timestamp("2026-04-15T00:00:00Z");
    save_snapshot(&paths, &snapshot).unwrap();
    acquire_runtime_ownership_lock_with_options(&paths, active_lock_options("status-json-tests"))
        .unwrap();
    let before = runtime_tree_snapshot(&root);

    let status_json = run_rust_millrace([
        "status",
        "show",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
    ])
    .expect("run Rust millrace status show json");

    status_json.assert_success();
    assert_eq!(status_json.stderr, "");
    let payload: Value =
        serde_json::from_str(&status_json.stdout).expect("parse blocked-idle status JSON");
    assert_eq!(payload["process_running"], true);
    assert_eq!(payload["active_run_count"], 0);
    assert_eq!(payload["execution_queue_depth"], 0);
    assert_eq!(payload["planning_queue_depth"], 0);
    assert_eq!(payload["learning_queue_depth"], 0);
    assert_eq!(payload["closure_target_open"], true);
    assert_eq!(payload["closure_target_blocked_by_lineage_work"], true);
    assert_eq!(payload["blocked_idle"], true);
    assert_eq!(payload["current_failure_class"], "recon_handoff_invalid");
    assert_eq!(
        payload["latest_runtime_error_report_path"],
        report_path.display().to_string()
    );

    let status_text = run_rust_millrace(["status", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace status text");
    status_text.assert_success();
    assert!(status_text.stdout.contains("blocked_idle: true\n"));
    assert!(status_text.stdout.contains(&format!(
        "latest_runtime_error_report_path: {}\n",
        report_path.display()
    )));
    assert!(
        status_text
            .stdout
            .contains("current_failure_class: recon_handoff_invalid\n")
    );
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_status_format_rejections_are_deterministic_and_read_only() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let before = runtime_tree_snapshot(&root);

    let invalid_format = run_rust_millrace([
        "status",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "yaml",
    ])
    .expect("run Rust millrace status invalid format");
    assert_exit_code(&invalid_format, 1);
    assert_eq!(
        invalid_format.stdout,
        "error: --format must be text or json\n"
    );
    assert_eq!(invalid_format.stderr, "");

    let watch_json = run_rust_millrace([
        "status",
        "watch",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
        "--max-updates",
        "1",
        "--interval-seconds",
        "0",
    ])
    .expect("run Rust millrace status watch json");
    assert_exit_code(&watch_json, 1);
    assert_eq!(
        watch_json.stdout,
        "error: status watch only supports text format\n"
    );
    assert_eq!(watch_json.stderr, "");
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_runs_read_only_commands_inspect_and_tail_artifacts_without_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let run_dir = paths.runs_dir.join("run-001");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    fs::write(
        stage_results_dir.join("request-001.json"),
        read_fixture("runtime_json/stage_result_envelope.json").unwrap(),
    )
    .unwrap();
    fs::write(run_dir.join("builder_summary.md"), "builder summary\n").unwrap();
    fs::write(run_dir.join("stdout.txt"), "stdout body\n").unwrap();
    fs::write(run_dir.join("stderr.txt"), "stderr body\n").unwrap();
    let before = runtime_tree_snapshot(&root);

    let list = run_rust_millrace(["runs", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace runs ls");
    list.assert_success();
    assert_eq!(list.stderr, "");
    assert!(list.stdout.contains("run_id: run-001\n"));
    assert!(list.stdout.contains("status: valid\n"));
    assert!(list.stdout.contains("work_item_kind: task\n"));

    let show = run_rust_millrace([
        "runs",
        "show",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show");
    show.assert_success();
    assert_eq!(show.stderr, "");
    for expected in [
        "stage_result_count: 1\n",
        "primary_tail_artifact: builder_summary.md\n",
        "stage_result_path: stage_results/request-001.json\n",
        "request_id: request-001\n",
        "terminal_result: BUILDER_COMPLETE\n",
        "thinking_level: none\n",
        "total_tokens: 135\n",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing expected runs show output fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }

    let tail = run_rust_millrace([
        "runs",
        "tail",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail");
    tail.assert_success();
    assert_eq!(tail.stdout, "builder summary\n\n");
    assert_eq!(runtime_tree_snapshot(&root), before);

    fs::create_dir_all(paths.runs_dir.join("run-empty")).unwrap();
    let before_missing_artifact = runtime_tree_snapshot(&root);
    let missing = run_rust_millrace([
        "runs",
        "tail",
        "run-empty",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail missing artifact");
    assert_exit_code(&missing, 1);
    assert_eq!(
        missing.stdout,
        "error: no tailable artifact found for run: run-empty\n"
    );
    assert_eq!(runtime_tree_snapshot(&root), before_missing_artifact);
}

#[test]
fn rust_runs_trace_renders_text_json_output_and_fallbacks_without_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let run_dir = paths.runs_dir.join("run-001");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    fs::write(
        stage_results_dir.join("request-001.json"),
        read_fixture("runtime_json/stage_result_envelope.json").unwrap(),
    )
    .unwrap();
    let before = runtime_tree_snapshot(&root);

    let text = run_rust_millrace([
        "runs",
        "trace",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs trace text");
    text.assert_success();
    assert_eq!(text.stderr, "");
    for expected in [
        "run_id: run-001\n",
        "status: incomplete\n",
        "compiled_plan_id: none\n",
        "work_item_kind: task\n",
        "work_item_id: task-001\n",
        "node_count: 1\n",
        "edge_count: 0\n",
        "note: derived from stage result artifacts\n",
        "builder BUILDER_COMPLETE\n",
    ] {
        assert!(
            text.stdout.contains(expected),
            "missing expected runs trace text fragment: {expected}\nstdout:\n{}",
            text.stdout
        );
    }

    let json_output = run_rust_millrace([
        "runs",
        "trace",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
    ])
    .expect("run Rust millrace runs trace json");
    json_output.assert_success();
    assert_eq!(json_output.stderr, "");
    let trace: Value =
        serde_json::from_str(&json_output.stdout).expect("parse runs trace JSON output");
    assert_eq!(trace["kind"], "run_trace_graph");
    assert_eq!(trace["run_id"], "run-001");
    assert_eq!(trace["status"], "incomplete");
    assert_eq!(trace["nodes"][0]["trace_node_id"], "request-001");
    assert_eq!(trace["nodes"][0]["terminal_result"], "BUILDER_COMPLETE");

    let output_path = temp_dir.path().join("run-trace.json");
    let file_output = run_rust_millrace([
        "runs",
        "trace",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
        "--output",
        output_path.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs trace output file");
    file_output.assert_success();
    assert_eq!(file_output.stdout, "");
    assert_eq!(file_output.stderr, "");
    let file_trace: Value =
        serde_json::from_str(&fs::read_to_string(output_path).unwrap()).unwrap();
    assert_eq!(file_trace["run_id"], "run-001");
    assert!(!run_dir.join("run_trace.json").exists());
    assert_eq!(runtime_tree_snapshot(&root), before);

    fs::write(run_dir.join("run_trace.json"), "{bad\n").unwrap();
    let before_malformed = runtime_tree_snapshot(&root);
    let malformed = run_rust_millrace([
        "runs",
        "trace",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs trace malformed fallback");
    malformed.assert_success();
    assert!(malformed.stdout.contains("status: malformed\n"));
    assert!(malformed.stdout.contains("note: run_trace.json malformed:"));
    assert!(malformed.stdout.contains("builder BUILDER_COMPLETE\n"));
    assert_eq!(
        fs::read_to_string(run_dir.join("run_trace.json")).unwrap(),
        "{bad\n"
    );
    assert_eq!(runtime_tree_snapshot(&root), before_malformed);

    let invalid_format = run_rust_millrace([
        "runs",
        "trace",
        "run-001",
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "yaml",
    ])
    .expect("run Rust millrace runs trace invalid format");
    assert_exit_code(&invalid_format, 1);
    assert_eq!(
        invalid_format.stdout,
        "error: --format must be text or json\n"
    );
    assert_eq!(invalid_format.stderr, "");
    assert_eq!(runtime_tree_snapshot(&root), before_malformed);

    let missing = run_rust_millrace([
        "runs",
        "trace",
        "missing",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs trace missing run");
    assert_exit_code(&missing, 1);
    assert_eq!(missing.stdout, "error: run not found: missing\n");
    assert_eq!(missing.stderr, "");
    assert_eq!(runtime_tree_snapshot(&root), before_malformed);
}

#[test]
fn rust_runs_show_displays_learning_noop_result_class_without_mutation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let run_dir = paths.runs_dir.join("run-learning-noop");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();
    fs::write(
        run_dir.join("analyst_summary.md"),
        "analyst no-op summary\n",
    )
    .unwrap();
    fs::write(run_dir.join("stdout.txt"), "### ANALYST_NOOP\n").unwrap();
    fs::write(run_dir.join("stderr.txt"), "").unwrap();

    let mut stage_result: Value =
        serde_json::from_str(&read_fixture("runtime_json/stage_result_envelope.json").unwrap())
            .unwrap();
    stage_result["run_id"] = json!("run-learning-noop");
    stage_result["plane"] = json!("learning");
    stage_result["stage"] = json!("analyst");
    stage_result["node_id"] = json!("analyst");
    stage_result["stage_kind_id"] = json!("analyst");
    stage_result["work_item_kind"] = json!("learning_request");
    stage_result["work_item_id"] = json!("learn-noop");
    stage_result["terminal_result"] = json!("ANALYST_NOOP");
    stage_result["result_class"] = json!("no_op");
    stage_result["summary_status_marker"] = json!("### ANALYST_NOOP");
    stage_result["success"] = json!(false);
    stage_result["detected_marker"] = json!("### ANALYST_NOOP");
    stage_result["report_artifact"] = json!("analyst_summary.md");
    stage_result["artifact_paths"] = json!(["analyst_summary.md"]);
    stage_result["notes"] = json!(["learning request required no changes"]);
    stage_result["metadata"]["request_id"] = json!("request-learning-noop");
    stage_result["metadata"]["request_kind"] = json!("learning_request");
    fs::write(
        stage_results_dir.join("request-learning-noop.json"),
        serde_json::to_string_pretty(&stage_result).unwrap() + "\n",
    )
    .unwrap();
    let before = runtime_tree_snapshot(&root);

    let show = run_rust_millrace([
        "runs",
        "show",
        "run-learning-noop",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show for learning no-op");

    show.assert_success();
    for expected in [
        "status: valid\n",
        "work_item_kind: learning_request\n",
        "work_item_id: learn-noop\n",
        "stage: analyst\n",
        "terminal_result: ANALYST_NOOP\n",
        "result_class: no_op\n",
        "primary_tail_artifact: analyst_summary.md\n",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing expected runs show output fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_runs_read_only_commands_surface_advanced_inspection_evidence() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let run_dir = paths.runs_dir.join("run-advanced");
    let stage_results_dir = run_dir.join("stage_results");
    fs::create_dir_all(&stage_results_dir).unwrap();

    fs::write(run_dir.join("arbiter_report.md"), "arbiter report\n").unwrap();
    fs::write(
        run_dir.join("runner_prompt.request-advanced.md"),
        "prompt\n",
    )
    .unwrap();
    fs::write(
        run_dir.join("runner_stdout.request-advanced.txt"),
        "stdout body\n",
    )
    .unwrap();
    fs::write(
        run_dir.join("runner_stderr.request-advanced.txt"),
        "stderr body\n",
    )
    .unwrap();
    fs::write(
        run_dir.join("runner_events.request-advanced.jsonl"),
        "{\"type\":\"event\"}\n",
    )
    .unwrap();
    fs::write(
        run_dir.join("runner_invocation.request-advanced.json"),
        "{}\n",
    )
    .unwrap();
    fs::write(
        run_dir.join("runner_completion.request-advanced.json"),
        "{}\n",
    )
    .unwrap();
    fs::write(
        run_dir.join("skill_revision_evidence.request-advanced.json"),
        "{\"kind\":\"skill_revision_evidence\"}\n",
    )
    .unwrap();

    let mut stage_result: Value =
        serde_json::from_str(&read_fixture("runtime_json/stage_result_envelope.json").unwrap())
            .unwrap();
    stage_result["run_id"] = json!("run-advanced");
    stage_result["work_item_id"] = json!("spec-root-001");
    stage_result["work_item_kind"] = json!("spec");
    stage_result["prompt_artifact"] = json!("runner_prompt.request-advanced.md");
    stage_result["report_artifact"] = json!("arbiter_report.md");
    stage_result["stdout_path"] = json!("runner_stdout.request-advanced.txt");
    stage_result["stderr_path"] = json!("runner_stderr.request-advanced.txt");
    stage_result["artifact_paths"] =
        json!(["arbiter_report.md", "runner_events.request-advanced.jsonl"]);
    stage_result["token_usage"] = json!({
        "input_tokens": 7,
        "cached_input_tokens": 2,
        "output_tokens": 3,
        "thinking_tokens": 1,
        "total_tokens": 11
    });
    stage_result["metadata"] = json!({
        "request_id": "request-advanced",
        "mode_id": "learning_codex",
        "compiled_plan_id": "plan-advanced",
        "request_kind": "closure_target",
        "closure_target_root_spec_id": "spec-root-001",
        "closure_target_root_idea_id": "idea-root-001",
        "preferred_rubric_path": "millrace-agents/arbiter/rubrics/spec-root-001.md",
        "preferred_verdict_path": "millrace-agents/arbiter/verdicts/spec-root-001.json",
        "preferred_report_path": "arbiter_report.md",
        "skill_revision_evidence_path": "skill_revision_evidence.request-advanced.json",
        "raw_exit_kind": "completed",
        "raw_exit_code": 0,
        "execution_capability_grants": [
            {
                "grant_id": "grant-runner",
                "request_id": "request-runner",
                "capability_id": "runner.invoke",
                "access": "execute",
                "scope": {"kind": "runner", "value": "fake", "metadata": {}},
                "required": true,
                "decision_state": "granted",
                "enforcement_mode": "runtime_enforced",
                "evidence_requirements": [],
                "evidence_status": "not_required",
                "decision_reason": "default runner grant",
                "resolved_by": "compiler",
                "fingerprint": "grant-111111111111"
            }
        ],
        "capability_support_decisions": [
            {
                "runner_id": "fake",
                "invocation_context_ref": "builder",
                "grant_id": "grant-runner",
                "support_state": "supported",
                "enforcement_mode": "runtime_enforced",
                "limitations": [],
                "evidence_available": ["runner_invocation", "runner_completion"],
                "reason": "fake runner provides deterministic capability evidence"
            }
        ]
    });
    fs::write(
        stage_results_dir.join("request-advanced.json"),
        serde_json::to_string_pretty(&stage_result).unwrap(),
    )
    .unwrap();
    fs::write(stage_results_dir.join("request-malformed.json"), "{bad\n").unwrap();
    fs::write(
        &paths.usage_governance_ledger_file,
        serde_json::to_string(&json!({
            "dedupe_key": "millrace-agents/runs/run-advanced/stage_results/request-advanced.json",
            "counted_at": "2026-04-15T00:00:01Z",
            "stage_completed_at": "2026-04-15T00:00:00Z",
            "plane": "execution",
            "run_id": "run-advanced",
            "stage_id": "builder",
            "work_item_kind": "spec",
            "work_item_id": "spec-root-001",
            "token_usage": {
                "input_tokens": 7,
                "cached_input_tokens": 2,
                "output_tokens": 3,
                "thinking_tokens": 1,
                "total_tokens": 11
            },
            "stage_result_path": "millrace-agents/runs/run-advanced/stage_results/request-advanced.json",
            "daemon_session_id": "daemon-test"
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();

    let runner_only_dir = paths.runs_dir.join("run-runner-only");
    fs::create_dir_all(&runner_only_dir).unwrap();
    fs::write(
        runner_only_dir.join("runner_stdout.request-runner-only.txt"),
        "runner-only stdout\n",
    )
    .unwrap();
    let stderr_only_dir = paths.runs_dir.join("run-stderr-only");
    fs::create_dir_all(&stderr_only_dir).unwrap();
    fs::write(
        stderr_only_dir.join("runner_stderr.request-stderr-only.txt"),
        "runner-only stderr\n",
    )
    .unwrap();
    let event_only_dir = paths.runs_dir.join("run-event-only");
    fs::create_dir_all(&event_only_dir).unwrap();
    fs::write(
        event_only_dir.join("runner_events.request-event-only.jsonl"),
        "{\"type\":\"event-only\"}\n",
    )
    .unwrap();

    let stage_only_dir = paths.runs_dir.join("run-stage-only");
    let stage_only_results_dir = stage_only_dir.join("stage_results");
    fs::create_dir_all(&stage_only_results_dir).unwrap();
    let mut stage_only: Value =
        serde_json::from_str(&read_fixture("runtime_json/stage_result_envelope.json").unwrap())
            .unwrap();
    stage_only["run_id"] = json!("run-stage-only");
    stage_only["report_artifact"] = Value::Null;
    stage_only["stdout_path"] = Value::Null;
    stage_only["stderr_path"] = Value::Null;
    stage_only["artifact_paths"] = json!([]);
    stage_only["metadata"] = json!({"request_id": "request-stage-only"});
    fs::write(
        stage_only_results_dir.join("request-stage-only.json"),
        serde_json::to_string_pretty(&stage_only).unwrap(),
    )
    .unwrap();

    let missing_selected_dir = paths.runs_dir.join("run-missing-selected");
    let missing_selected_results_dir = missing_selected_dir.join("stage_results");
    fs::create_dir_all(&missing_selected_results_dir).unwrap();
    let mut missing_selected: Value =
        serde_json::from_str(&read_fixture("runtime_json/stage_result_envelope.json").unwrap())
            .unwrap();
    missing_selected["run_id"] = json!("run-missing-selected");
    missing_selected["report_artifact"] = Value::Null;
    missing_selected["stdout_path"] = json!("missing-stdout.txt");
    missing_selected["stderr_path"] = Value::Null;
    missing_selected["artifact_paths"] = json!([]);
    missing_selected["metadata"] = json!({"request_id": "request-missing-selected"});
    fs::write(
        missing_selected_results_dir.join("request-missing-selected.json"),
        serde_json::to_string_pretty(&missing_selected).unwrap(),
    )
    .unwrap();

    let before = runtime_tree_snapshot(&root);
    let list = run_rust_millrace(["runs", "ls", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace runs ls advanced");
    list.assert_success();
    assert!(list.stdout.contains("run_id: run-advanced\n"));
    assert!(list.stdout.contains("status: malformed\n"));
    assert!(list.stdout.contains("run_id: run-runner-only\n"));
    assert!(list.stdout.contains("status: incomplete\n"));
    assert!(list.stdout.contains("runner_artifact_count: 1\n"));

    let show = run_rust_millrace([
        "runs",
        "show",
        "run-advanced",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs show advanced");
    show.assert_success();
    for expected in [
        "status: malformed\n",
        "compiled_plan_id: plan-advanced\n",
        "mode_id: learning_codex\n",
        "request_kind: closure_target\n",
        "closure_target_root_spec_id: spec-root-001\n",
        "closure_target_root_idea_id: idea-root-001\n",
        "primary_prompt_artifact_path: runner_prompt.request-advanced.md\n",
        "primary_event_log_path: runner_events.request-advanced.jsonl\n",
        "primary_runner_invocation_path: runner_invocation.request-advanced.json\n",
        "primary_runner_completion_path: runner_completion.request-advanced.json\n",
        "primary_skill_revision_evidence_path: skill_revision_evidence.request-advanced.json\n",
        "malformed_stage_result_path: stage_results/request-malformed.json\n",
        "governance_ledger_stage_result_path: millrace-agents/runs/run-advanced/stage_results/request-advanced.json\n",
        "runner_artifact: kind=runner_completion request_id=request-advanced path=runner_completion.request-advanced.json thinking_level=none model_reasoning_effort=none\n",
        "thinking_level: none\n",
        "skill_revision_evidence_path: skill_revision_evidence.request-advanced.json\n",
        "preferred_verdict_path: millrace-agents/arbiter/verdicts/spec-root-001.json\n",
        "remediation_reference_path: millrace-agents/arbiter/verdicts/spec-root-001.json\n",
        "raw_exit_kind: completed\n",
        "raw_exit_code: 0\n",
        "capability_grant: grant_id=grant-runner capability_id=runner.invoke decision=granted enforcement=runtime_enforced evidence_status=not_required fingerprint=grant-111111111111\n",
        "capability_support: grant_id=grant-runner runner_id=fake support=supported enforcement=runtime_enforced evidence=runner_invocation,runner_completion\n",
        "total_tokens: 11\n",
        "note: request-malformed.json: invalid stage result payload:",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing expected advanced runs show output fragment: {expected}\nstdout:\n{}",
            show.stdout
        );
    }

    let report_tail = run_rust_millrace([
        "runs",
        "tail",
        "run-advanced",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail advanced report");
    report_tail.assert_success();
    assert_eq!(report_tail.stdout, "arbiter report\n\n");

    let runner_only_tail = run_rust_millrace([
        "runs",
        "tail",
        "run-runner-only",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail runner-only");
    runner_only_tail.assert_success();
    assert_eq!(runner_only_tail.stdout, "runner-only stdout\n\n");

    let stderr_only_tail = run_rust_millrace([
        "runs",
        "tail",
        "run-stderr-only",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail stderr-only");
    stderr_only_tail.assert_success();
    assert_eq!(stderr_only_tail.stdout, "runner-only stderr\n\n");

    let event_only_tail = run_rust_millrace([
        "runs",
        "tail",
        "run-event-only",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail event-only");
    event_only_tail.assert_success();
    assert_eq!(event_only_tail.stdout, "{\"type\":\"event-only\"}\n\n");

    let stage_only_tail = run_rust_millrace([
        "runs",
        "tail",
        "run-stage-only",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail stage-only");
    stage_only_tail.assert_success();
    assert!(
        stage_only_tail
            .stdout
            .contains("\"kind\": \"stage_result\"")
    );

    let missing_selected_tail = run_rust_millrace([
        "runs",
        "tail",
        "run-missing-selected",
        "--workspace",
        root.to_str().unwrap(),
    ])
    .expect("run Rust millrace runs tail missing selected artifact");
    assert_exit_code(&missing_selected_tail, 1);
    assert!(
        missing_selected_tail
            .stdout
            .contains("error: failed to read tailable artifact missing-stdout.txt: "),
        "missing selected-artifact failure detail\nstdout:\n{}",
        missing_selected_tail.stdout
    );
    assert_eq!(runtime_tree_snapshot(&root), before);
}

#[test]
fn rust_skills_list_show_and_search_file_backed_indexes() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let workspace = root.to_str().unwrap();

    let list = run_rust_millrace(["skills", "ls", "--workspace", workspace])
        .expect("run Rust millrace skills ls");
    list.assert_success();
    assert_eq!(list.stderr, "");
    for expected in [
        "builder-core\n",
        "millrace-skill-creator\n",
        "skills-readme\n",
    ] {
        assert!(
            list.stdout.contains(expected),
            "missing skill list entry: {expected}\nstdout:\n{}",
            list.stdout
        );
    }

    let show = run_rust_millrace(["skills", "show", "builder-core", "--workspace", workspace])
        .expect("run Rust millrace skills show");
    show.assert_success();
    assert_eq!(show.stderr, "");
    assert!(show.stdout.contains("skill_id: builder-core\n"));
    assert!(show.stdout.contains("path: "));
    assert!(show.stdout.contains("builder-core/SKILL.md\n"));
    assert!(show.stdout.contains("title: Builder Core\n"));

    let search = run_rust_millrace(["skills", "search", "Builder Core", "--workspace", workspace])
        .expect("run Rust millrace skills search");
    search.assert_success();
    assert_eq!(search.stderr, "");
    assert_eq!(search.stdout, "builder-core\n");

    let source_list = run_rust_millrace([
        "skills",
        "ls",
        "--target",
        "source",
        "--workspace",
        workspace,
    ])
    .expect("run Rust millrace skills ls source");
    source_list.assert_success();
    assert!(source_list.stdout.contains("builder-core\n"));

    let unsafe_show = run_rust_millrace(["skills", "show", "../bad", "--workspace", workspace])
        .expect("run Rust millrace skills unsafe show");
    assert_exit_code(&unsafe_show, 1);
    assert_eq!(unsafe_show.stderr, "");
    assert!(unsafe_show.stdout.contains("unsafe skill id"));
}

#[test]
fn rust_skills_install_export_and_promote_file_backed_packages() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();

    let source_skill = temp_dir.path().join("source-skill");
    fs::create_dir_all(source_skill.join("references")).unwrap();
    fs::write(
        source_skill.join("SKILL.md"),
        "---\nasset_type: skill\nasset_id: source-skill\n---\n\n# Source Skill\n",
    )
    .unwrap();
    fs::write(source_skill.join("references/ref.md"), "# Reference\n").unwrap();

    let install = run_rust_millrace([
        "skills",
        "install",
        source_skill.to_str().unwrap(),
        "--workspace",
        workspace,
    ])
    .expect("run Rust millrace skills install local");
    install.assert_success();
    assert_eq!(install.stderr, "");
    assert!(install.stdout.contains("installed_skill: source-skill\n"));
    assert!(paths.skills_dir.join("source-skill/SKILL.md").is_file());
    assert!(
        fs::read_to_string(paths.skills_dir.join("skills_index.md"))
            .unwrap()
            .contains("- source-skill: source-skill/SKILL.md")
    );
    assert!(
        fs::read_to_string(paths.skills_dir.join("skill_operations.jsonl"))
            .unwrap()
            .contains("\"operation\":\"install\"")
    );

    let duplicate = run_rust_millrace([
        "skills",
        "install",
        source_skill.to_str().unwrap(),
        "--workspace",
        workspace,
    ])
    .expect("run Rust millrace skills install duplicate");
    assert_exit_code(&duplicate, 1);
    assert!(
        duplicate
            .stdout
            .contains("skill already exists: source-skill")
    );

    let archive = temp_dir.path().join("source-skill-bundle.zip");
    let export = run_rust_millrace([
        "skills",
        "export",
        "source-skill",
        "--workspace",
        workspace,
        "--output",
        archive.to_str().unwrap(),
    ])
    .expect("run Rust millrace skills export");
    export.assert_success();
    assert_eq!(
        export.stdout,
        format!("exported_skill: {}\n", archive.display())
    );
    let archive_bytes = fs::read(&archive).unwrap();
    assert_eq!(&archive_bytes[..4], b"PK\x03\x04");

    let source_assets = temp_dir.path().join("source-assets");
    fs::create_dir_all(&source_assets).unwrap();
    fs::write(source_assets.join("skills_index.md"), "# Skills Index\n").unwrap();
    let promote = run_rust_millrace_with_env(
        [
            "skills",
            "promote",
            "source-skill",
            "--workspace",
            workspace,
        ],
        [(
            "MILLRACE_SOURCE_SKILLS_DIR",
            source_assets.to_str().unwrap(),
        )],
    )
    .expect("run Rust millrace skills promote");
    promote.assert_success();
    assert_eq!(promote.stderr, "");
    assert!(promote.stdout.contains("promoted_skill: source-skill\n"));
    assert!(source_assets.join("source-skill/SKILL.md").is_file());
    assert!(
        fs::read_to_string(source_assets.join("skills_index.md"))
            .unwrap()
            .contains("- source-skill: source-skill/SKILL.md")
    );
    let promote_log = fs::read_to_string(source_assets.join("skill_operations.jsonl")).unwrap();
    let promote_event: Value = serde_json::from_str(promote_log.lines().last().unwrap()).unwrap();
    assert_eq!(promote_event["operation"], "promote");
    assert_eq!(promote_event["skill_id"], "source-skill");
    assert_eq!(promote_event["operator_controlled"], true);
    assert_eq!(promote_event["promotion_source"], "workspace");
    assert_eq!(promote_event["promotion_destination"], "source");
    assert_eq!(
        promote_event["promoted_files"],
        json!(["SKILL.md", "references/ref.md"])
    );
    assert_eq!(
        promote_event["file_sha256"]["SKILL.md"],
        sha256_hex(&fs::read(source_assets.join("source-skill/SKILL.md")).unwrap())
    );
}

#[test]
fn rust_skills_remote_refresh_and_install_use_fixture_backed_index() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();
    let index = fixture_path("skills/remote_index.md");
    let remote_root = fixture_path("skills/remote");

    let refresh = run_rust_millrace_with_env(
        ["skills", "refresh-remote-index", "--workspace", workspace],
        [("MILLRACE_REMOTE_SKILLS_INDEX_PATH", index.to_str().unwrap())],
    )
    .expect("run Rust millrace skills refresh remote index");
    refresh.assert_success();
    assert_eq!(refresh.stderr, "");
    assert!(refresh.stdout.contains("remote_skills_index: "));
    assert!(paths.skills_dir.join("remote_skills_index.md").is_file());

    let install = run_rust_millrace_with_env(
        [
            "skills",
            "install",
            "browser-local-qa",
            "--workspace",
            workspace,
        ],
        [
            ("MILLRACE_REMOTE_SKILLS_INDEX_PATH", index.to_str().unwrap()),
            ("MILLRACE_REMOTE_SKILLS_ROOT", remote_root.to_str().unwrap()),
        ],
    )
    .expect("run Rust millrace skills install remote");
    install.assert_success();
    assert_eq!(install.stderr, "");
    assert!(
        install
            .stdout
            .contains("installed_skill: browser-local-qa\n")
    );
    assert!(install.stdout.contains("source: remote\n"));
    assert!(paths.skills_dir.join("browser-local-qa/SKILL.md").is_file());
    assert!(
        paths
            .skills_dir
            .join("browser-local-qa/references/evidence.md")
            .is_file()
    );
    assert!(
        paths
            .skills_dir
            .join("browser-local-qa/remote_source.json")
            .is_file()
    );
    assert!(
        fs::read_to_string(paths.skills_dir.join("skills_index.md"))
            .unwrap()
            .contains("- browser-local-qa: browser-local-qa/SKILL.md")
    );

    let draft = run_rust_millrace_with_env(
        ["skills", "install", "draft-skill", "--workspace", workspace],
        [
            ("MILLRACE_REMOTE_SKILLS_INDEX_PATH", index.to_str().unwrap()),
            ("MILLRACE_REMOTE_SKILLS_ROOT", remote_root.to_str().unwrap()),
        ],
    )
    .expect("run Rust millrace skills install unavailable remote");
    assert_exit_code(&draft, 1);
    assert!(draft.stdout.contains("remote skill is not available"));
}

#[test]
fn rust_skills_create_and_improve_queue_learning_requests_when_mode_allows() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    run_init_for(&root);
    let paths = workspace_paths(&root);
    let workspace = root.to_str().unwrap();

    let blocked = run_rust_millrace([
        "skills",
        "create",
        "write a checker skill",
        "--workspace",
        workspace,
    ])
    .expect("run Rust millrace skills create default mode");
    assert_exit_code(&blocked, 1);
    assert!(
        blocked
            .stdout
            .contains("current mode does not enable the learning plane")
    );
    assert!(
        fs::read_dir(&paths.learning_requests_queue_dir)
            .unwrap()
            .next()
            .is_none()
    );

    let create = run_rust_millrace([
        "skills",
        "create",
        "write a checker skill",
        "--workspace",
        workspace,
        "--mode",
        "learning_codex",
    ])
    .expect("run Rust millrace skills create learning mode");
    create.assert_success();
    assert!(create.stdout.contains("queued_learning_request: "));

    let improve = run_rust_millrace([
        "skills",
        "improve",
        "builder-core",
        "--workspace",
        workspace,
        "--mode",
        "learning_codex",
    ])
    .expect("run Rust millrace skills improve learning mode");
    improve.assert_success();
    assert!(improve.stdout.contains("queued_learning_request: "));

    let mut documents: Vec<_> = fs::read_dir(&paths.learning_requests_queue_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    documents.sort();
    assert_eq!(documents.len(), 2);
    let combined = documents
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(combined.contains("Requested-Action: create\n"));
    assert!(combined.contains("Summary: write a checker skill\n"));
    assert!(combined.contains("Requested-Action: improve\n"));
    assert!(combined.contains("Target-Skill-ID: builder-core\n"));
}

#[test]
fn rust_read_only_commands_require_initialized_workspace_without_creating_it() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let cases = [
        vec!["queue", "ls", "--workspace", root.to_str().unwrap()],
        vec!["status", "--workspace", root.to_str().unwrap()],
        vec!["runs", "ls", "--workspace", root.to_str().unwrap()],
        vec![
            "runs",
            "trace",
            "run-001",
            "--workspace",
            root.to_str().unwrap(),
        ],
        vec!["config", "show", "--workspace", root.to_str().unwrap()],
        vec!["skills", "ls", "--workspace", root.to_str().unwrap()],
    ];

    for args in cases {
        let output = run_rust_millrace(args).expect("run Rust millrace read-only uninitialized");

        assert_exit_code(&output, 1);
        assert!(
            output
                .stdout
                .starts_with("error: workspace is not initialized: ")
        );
        assert_eq!(output.stderr, "");
        assert!(!root.join("millrace-agents").exists());
    }
}

#[test]
fn rust_run_daemon_requires_initialized_workspace_without_creating_it() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let output = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--max-ticks",
        "1",
    ])
    .expect("run Rust millrace run daemon uninitialized");

    assert_exit_code(&output, 1);
    assert!(
        output
            .stdout
            .starts_with("error: workspace is not initialized: ")
    );
    assert_eq!(output.stderr, "");
    assert!(!root.join("millrace-agents").exists());
}

#[test]
fn rust_run_daemon_preserves_parse_and_execution_error_classes() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let max_ticks = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--max-ticks",
        "0",
    ])
    .expect("run Rust millrace run daemon invalid max ticks");
    assert_exit_code(&max_ticks, 2);
    assert_eq!(max_ticks.stdout, "");
    assert_eq!(
        max_ticks.stderr,
        "error: `--max-ticks` value must be an integer greater than or equal to 1\n"
    );
    assert!(!root.join("millrace-agents").exists());

    run_init_for(&root);
    let invalid_monitor = run_rust_millrace([
        "run",
        "daemon",
        "--workspace",
        root.to_str().unwrap(),
        "--monitor",
        "verbose",
    ])
    .expect("run Rust millrace run daemon invalid monitor");
    assert_exit_code(&invalid_monitor, 1);
    assert_eq!(
        invalid_monitor.stdout,
        "error: unknown monitor mode: verbose\n"
    );
    assert_eq!(invalid_monitor.stderr, "");
}

#[test]
fn rust_control_and_config_parse_failures_do_not_create_or_mutate_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");

    let control_parse = run_rust_millrace([
        "control",
        "pause",
        "--workspace",
        root.to_str().unwrap(),
        "--reason",
        "not-accepted",
    ])
    .expect("run Rust millrace control parse failure");
    assert_exit_code(&control_parse, 2);
    assert_eq!(control_parse.stdout, "");
    assert_eq!(control_parse.stderr, "error: unknown option `--reason`\n");
    assert!(!root.join("millrace-agents").exists());

    let config_parse = run_rust_millrace([
        "config",
        "validate",
        "--workspace",
        root.to_str().unwrap(),
        "--mode",
    ])
    .expect("run Rust millrace config parse failure");
    assert_exit_code(&config_parse, 2);
    assert_eq!(config_parse.stdout, "");
    assert_eq!(config_parse.stderr, "error: missing value for `--mode`\n");
    assert!(!root.join("millrace-agents").exists());
}

#[test]
fn rust_control_planning_and_config_commands_require_initialized_workspace() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let cases = [
        vec!["control", "pause", "--workspace", root.to_str().unwrap()],
        vec![
            "planning",
            "retry-active",
            "--workspace",
            root.to_str().unwrap(),
        ],
        vec!["config", "validate", "--workspace", root.to_str().unwrap()],
        vec!["config", "reload", "--workspace", root.to_str().unwrap()],
    ];

    for args in cases {
        let output = run_rust_millrace(args).expect("run Rust millrace uninitialized command");

        assert_exit_code(&output, 1);
        assert!(
            output
                .stdout
                .starts_with("error: workspace is not initialized: ")
        );
        assert_eq!(output.stderr, "");
        assert!(!root.join("millrace-agents").exists());
    }
}

#[test]
fn rust_cli_framework_rejects_representative_shared_parse_failures() {
    let cases = [
        (
            vec!["queue", "ls", "--workspace="],
            "error: `--workspace` value must not be empty\n",
        ),
        (
            vec![
                "control",
                "pause",
                "--workspace=/tmp/first",
                "--workspace=/tmp/second",
            ],
            "error: duplicate `--workspace` option\n",
        ),
        (
            vec!["run", "daemon", "--mode"],
            "error: missing value for `--mode`\n",
        ),
        (
            vec!["status", "watch", "--unknown"],
            "error: unknown option `--unknown`\n",
        ),
        (
            vec!["approvals", "approve", "approval-001", "--reason"],
            "error: missing value for `--reason`\n",
        ),
        (
            vec!["approvals", "deny", "approval-001", "--reason="],
            "error: `--reason` value must not be empty\n",
        ),
        (
            vec!["approvals", "show"],
            "error: missing required argument `APPROVAL_ID`\n",
        ),
        (
            vec!["modes", "show", "default_codex", "extra"],
            "error: unexpected argument `extra`\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_rust_millrace(args).expect("run Rust millrace parse failure");

        assert_exit_code(&output, 2);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, expected_stderr);
    }
}

#[test]
fn rust_init_cli_does_not_create_legacy_runtime_dirs_in_workspace_or_repo_root() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().join("workspace");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let legacy_dirs = [
        "state",
        "runs",
        "tasks",
        "specs",
        "incidents",
        "loops",
        "graphs",
        "registry",
        "logs",
        "entrypoints",
        "skills",
        "roles",
    ];
    let repo_before: Vec<_> = legacy_dirs
        .iter()
        .map(|name| (name, repo_root.join(name).exists()))
        .collect();

    run_rust_millrace(["init", "--workspace", root.to_str().unwrap()])
        .expect("run Rust millrace init")
        .assert_success();

    for name in legacy_dirs {
        assert!(
            !root.join(name).exists(),
            "unexpected root-level runtime artifact: {name}"
        );
    }
    assert!(!root.join("millrace-agents/roles").exists());

    for (name, existed_before) in repo_before {
        assert_eq!(
            repo_root.join(name).exists(),
            existed_before,
            "repository root artifact changed after init: {name}"
        );
    }
}

#[test]
fn rust_init_cli_rejects_missing_or_malformed_workspace_input() {
    let cases = [
        (
            vec!["init"],
            "error: missing required option `--workspace <path>`\n",
        ),
        (
            vec!["init", "--workspace"],
            "error: missing value for `--workspace`\n",
        ),
        (
            vec!["init", "--workspace", ""],
            "error: `--workspace` value must not be empty\n",
        ),
        (
            vec!["init", "--workspace="],
            "error: `--workspace` value must not be empty\n",
        ),
        (
            vec!["init", "--workspace=/tmp/first", "--workspace=/tmp/second"],
            "error: duplicate `--workspace` option\n",
        ),
        (
            vec!["init", "--workspace", "/tmp/workspace", "extra"],
            "error: unexpected argument `extra`\n",
        ),
        (
            vec!["init", "--unknown", "/tmp/workspace"],
            "error: unknown option `--unknown`\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_rust_millrace(args).expect("run Rust millrace init parse failure");

        assert_exit_code(&output, 2);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, expected_stderr);
    }
}

#[test]
fn python_reference_version_probe_is_pinned_to_0_17_3() {
    let output = run_python_reference_version_probe().expect("run Python reference version probe");

    output.assert_success();

    let version_line =
        parse_version_line(output.stdout_trimmed()).expect("parse Python version line");
    assert_eq!(version_line.binary_name, "millrace");
    assert_eq!(version_line.version, "0.17.3");
}

#[test]
fn version_shape_matches_python_reference_even_when_versions_differ() {
    let rust = run_rust_millrace(["--version"]).expect("run Rust millrace --version");
    let python = run_python_reference_version_probe().expect("run Python reference version probe");

    rust.assert_success();
    python.assert_success();

    let rust_line = parse_version_line(rust.stdout_trimmed()).expect("parse Rust version line");
    let python_line =
        parse_version_line(python.stdout_trimmed()).expect("parse Python version line");

    assert_eq!(rust_line.binary_name, python_line.binary_name);
    assert_ne!(rust_line.version, python_line.version);
}

#[test]
fn parity_workspace_fixture_does_not_initialize_millrace() {
    let workspace = ParityWorkspace::new().expect("create parity workspace fixture");

    assert!(
        !workspace
            .python_workspace()
            .join("millrace-agents")
            .exists()
    );
    assert!(!workspace.rust_workspace().join("millrace-agents").exists());
}

#[test]
#[ignore = "requires a Python environment with millrace-ai CLI dependencies installed"]
fn python_reference_cli_probe() {
    let output = run_python_reference_cli(["--version"]).expect("run Python reference CLI");

    output.assert_success();
    assert_eq!(output.stdout_trimmed(), "millrace 0.17.3");
}
