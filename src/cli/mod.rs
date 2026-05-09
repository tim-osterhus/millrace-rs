//! Command-line dispatch, parsing, workspace checks, and rendering.

use std::{collections::BTreeSet, fs, io::BufWriter, path::PathBuf, process::ExitCode};

use crate::{
    compiler::{
        CompileOutcome, CompileWorkspaceOptions, CompiledRunPlan, FrozenGraphPlanePlan,
        MaterializedGraphNodePlan,
    },
    contracts::{Plane, ResultClass, RuntimeMode},
    runtime::{
        BasicTerminalMonitor, RuntimeDaemonLoopExitReason, RuntimeDaemonLoopOptions,
        RuntimeMonitorEvent, RuntimeMonitorFanout, RuntimeMonitorSink, RuntimeStartupOptions,
        RuntimeTickDispatchOutcome, RuntimeTickOptions, RuntimeTickOutcomeKind,
        build_runtime_runner_dispatcher, load_runtime_startup_config,
    },
    workspace::{
        BaselineUpgradePreview, ClosureLineageRepairOutcome, LineageRepairError, RuntimeControl,
        RuntimeControlActionResult, WorkspacePaths, apply_baseline_upgrade, load_baseline_manifest,
        preview_baseline_upgrade, repair_closure_lineage,
    },
};
use serde_json::{Map, Value};

mod intake;
mod parser;
mod read_only;
mod render;
mod skills;

use parser::{EmptyValue, OptionSpec, ParsedArgs, parse_args};
use render::CliOutput;

pub const PRIMARY_COMMAND_GROUPS: &[&str] = &[
    "run", "status", "runs", "queue", "planning", "config", "control", "compile", "modes",
    "skills", "doctor", "upgrade", "init", "version",
];

pub const COMPATIBILITY_ALIASES: &[&str] = &[
    "add-task",
    "add-probe",
    "add-spec",
    "add-idea",
    "pause",
    "resume",
    "stop",
    "retry-active",
    "clear-stale-state",
    "reload-config",
    "about",
];

pub fn run<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).skip(1).collect::<Vec<_>>();
    render::render_output(dispatch(args))
}

fn dispatch(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return status_overview_output();
    }
    let command = args.remove(0);

    match command.as_str() {
        "--version" | "-V" | "version" => version_output(),
        "init" => run_init(args),
        "doctor" => run_doctor(args),
        "compile" => run_compile(args),
        "--status" | "about" => status_overview_output(),
        "run" => run_runtime_group(args),
        "status" => run_status_group(args),
        "runs" => run_runs_group(args),
        "queue" => run_queue_group(args),
        "planning" => run_planning_group(args),
        "config" => run_config_group(args),
        "control" => run_control_group(args),
        "modes" => run_modes_group(args),
        "skills" => run_skills_group(args),
        "upgrade" => run_upgrade_group(args),
        "add-task" => run_queue_alias("add-task", "add-task", args),
        "add-probe" => run_queue_alias("add-probe", "add-probe", args),
        "add-spec" => run_queue_alias("add-spec", "add-spec", args),
        "add-idea" => run_queue_alias("add-idea", "add-idea", args),
        "pause" => run_control_alias("pause", args),
        "resume" => run_control_alias("resume", args),
        "stop" => run_control_alias("stop", args),
        "retry-active" => run_control_alias("retry-active", args),
        "clear-stale-state" => run_control_alias("clear-stale-state", args),
        "reload-config" => run_control_alias("reload-config", args),
        _ => CliOutput::parse_error(format!("unknown command `{command}`")),
    }
}

fn run_compile(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error("missing compile command `validate`, `show`, or `graph`");
    }
    let command = args.remove(0);
    match command.as_str() {
        "validate" => run_compile_command(CompileCliCommand::Validate, args),
        "show" => run_compile_command(CompileCliCommand::Show, args),
        "graph" => run_compile_graph(args),
        _ => CliOutput::parse_error(format!("unknown compile command `{command}`")),
    }
}

fn run_compile_command(command: CompileCliCommand, args: Vec<String>) -> CliOutput {
    let parsed = match parse_or_output(args, &[workspace_spec(), mode_spec(), config_spec()]) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let options = CompileCliOptions {
        workspace: parsed.value("--workspace").unwrap_or(".").to_owned(),
        mode: parsed.value("--mode").map(ToOwned::to_owned),
        config_path: parsed.value("--config").map(PathBuf::from),
    };
    let paths = match require_workspace(&options.workspace) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    let compiler_options = CompileWorkspaceOptions {
        requested_mode_id: options.mode,
        config_path: options.config_path,
        ..CompileWorkspaceOptions::default()
    };

    match crate::compiler::compile_and_persist_workspace_plan_for_paths(&paths, compiler_options) {
        Ok(outcome) => {
            let mut lines = render_compile_diagnostics_lines(&outcome);
            if command == CompileCliCommand::Show {
                lines.extend(render_compile_show_lines(&paths, &outcome));
            }
            CliOutput::with_exit_code(lines, compile_exit_code(&outcome))
        }
        Err(error) => CliOutput::stdout_failure(error.to_string()),
    }
}

fn run_compile_graph(args: Vec<String>) -> CliOutput {
    let parsed = match parse_or_output(
        args,
        &[
            workspace_spec(),
            mode_spec(),
            config_spec(),
            value_spec("--plane", EmptyValue::NonBlank),
            value_spec("--format", EmptyValue::NonBlank),
            value_spec("--output", EmptyValue::NonEmpty),
        ],
    ) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }

    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    let compiler_options = CompileWorkspaceOptions {
        requested_mode_id: parsed.value("--mode").map(ToOwned::to_owned),
        config_path: parsed.value("--config").map(PathBuf::from),
        ..CompileWorkspaceOptions::default()
    };
    let outcome = match crate::compiler::compile_and_persist_workspace_plan_for_paths(
        &paths,
        compiler_options,
    ) {
        Ok(outcome) => outcome,
        Err(error) => return CliOutput::stdout_failure(error.to_string()),
    };
    let Some(plan) = outcome.active_plan.as_ref() else {
        return CliOutput::with_exit_code(
            render_compile_diagnostics_lines(&outcome),
            compile_exit_code(&outcome),
        );
    };

    let selected_graphs = if let Some(plane) = parsed.value("--plane") {
        let plane = match Plane::from_value(plane) {
            Ok(plane) => plane,
            Err(error) => return CliOutput::stdout_failure(error.to_string()),
        };
        match crate::compiler::export_compiled_stage_graph(plan, plane) {
            Ok(graph) => vec![graph],
            Err(error) => return CliOutput::stdout_failure(error.to_string()),
        }
    } else {
        match crate::compiler::export_compiled_stage_graphs(plan) {
            Ok(graphs) => graphs,
            Err(error) => return CliOutput::stdout_failure(error.to_string()),
        }
    };

    match render_format(parsed.value("--format").unwrap_or("text")) {
        Ok(GraphTraceOutputFormat::Text) => write_or_print_rendered(
            render::compiled_graph_lines(&selected_graphs).join("\n"),
            &parsed,
        ),
        Ok(GraphTraceOutputFormat::Json) => match serde_json::to_string_pretty(&selected_graphs) {
            Ok(rendered) => write_or_print_rendered(rendered, &parsed),
            Err(error) => {
                CliOutput::stdout_failure(format!("failed to render graph JSON: {error}"))
            }
        },
        Err(output) => output,
    }
}

fn run_doctor(args: Vec<String>) -> CliOutput {
    let parsed = match parse_or_output(args, &[workspace_spec()]) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let workspace = parsed.value("--workspace").unwrap_or(".");

    let report = crate::run_workspace_doctor(workspace);
    let mut lines = vec![
        format!("ok: {}", bool_text(report.ok)),
        format!("errors: {}", report.errors.len()),
        format!("warnings: {}", report.warnings.len()),
    ];
    for issue in &report.errors {
        lines.push(format!(
            "error: {} {} {}",
            issue.code,
            issue_location(issue),
            issue.message
        ));
    }
    for issue in &report.warnings {
        lines.push(format!(
            "warning: {} {} {}",
            issue.code,
            issue_location(issue),
            issue.message
        ));
    }

    CliOutput::with_exit_code(lines, if report.ok { 0 } else { 1 })
}

fn run_init(args: Vec<String>) -> CliOutput {
    let parsed = match parse_or_output(args, &[workspace_spec()]) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let Some(workspace) = parsed.value("--workspace") else {
        return CliOutput::parse_error("missing required option `--workspace <path>`");
    };

    match crate::initialize_workspace(workspace) {
        Ok(paths) => CliOutput::success(vec![
            format!("workspace: {}", paths.root.display()),
            "initialized: true".to_owned(),
        ]),
        Err(error) => CliOutput::stderr_failure(error.to_string()),
    }
}

fn run_runtime_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error("missing run command `once` or `daemon`");
    }
    let command = args.remove(0);
    let specs = match command.as_str() {
        "once" => vec![workspace_spec(), mode_spec(), config_spec()],
        "daemon" => vec![
            workspace_spec(),
            mode_spec(),
            config_spec(),
            value_spec("--max-ticks", EmptyValue::NonBlank),
            value_spec("--monitor", EmptyValue::NonBlank),
            value_spec("--monitor-log", EmptyValue::NonEmpty),
        ],
        _ => return CliOutput::parse_error(format!("unknown run command `{command}`")),
    };
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    if command == "daemon" {
        if let Err(output) = validate_daemon_max_ticks(&parsed) {
            return output;
        }
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    if command == "daemon" {
        if let Err(output) = validate_daemon_monitor(&parsed) {
            return output;
        }
        return run_daemon(parsed, paths);
    }
    run_once(parsed, paths)
}

fn run_daemon(parsed: ParsedArgs, paths: WorkspacePaths) -> CliOutput {
    let mode_override = parsed.value("--mode").map(ToOwned::to_owned);
    let startup_options = RuntimeStartupOptions {
        requested_mode_id: mode_override.clone(),
        config_path: parsed.value("--config").map(PathBuf::from),
        runtime_mode: RuntimeMode::Daemon,
        ..RuntimeStartupOptions::default()
    };

    let session = match crate::runtime::startup_runtime_daemon_for_paths(&paths, startup_options) {
        Ok(session) => session,
        Err(error) => return run_daemon_startup_failure(mode_override.as_deref(), error),
    };

    let mut session = session;
    let mut monitor = match build_daemon_monitor(&parsed) {
        Ok(monitor) => monitor,
        Err(error) => {
            let release_result = session.close().map_err(|error| error.to_string());
            return run_daemon_internal_failure(
                mode_override.as_deref(),
                &session,
                error.to_string(),
                0,
                release_result,
            );
        }
    };
    if let Some(monitor) = monitor.as_mut() {
        let startup_event = daemon_startup_monitor_event(&session);
        if let Err(error) = monitor.emit(&startup_event) {
            let release_result = session.close().map_err(|error| error.to_string());
            return run_daemon_internal_failure(
                mode_override.as_deref(),
                &session,
                format!("failed to emit daemon monitor startup event: {error}"),
                0,
                release_result,
            );
        }
    }

    let runner = match build_runtime_runner_dispatcher(&session) {
        Ok(runner) => runner,
        Err(error) => {
            let release_result = session.close().map_err(|error| error.to_string());
            return run_daemon_internal_failure(
                mode_override.as_deref(),
                &session,
                error.to_string(),
                0,
                release_result,
            );
        }
    };

    let loop_options = daemon_loop_options(&parsed);
    let outcome = if let Some(monitor) = monitor.as_mut() {
        crate::runtime::run_runtime_daemon_loop_with_monitor(session, runner, loop_options, monitor)
    } else {
        crate::runtime::run_runtime_daemon_loop(session, runner, loop_options)
    };

    match outcome {
        Ok(outcome) => render_run_daemon_outcome(mode_override.as_deref(), outcome),
        Err(error) => run_daemon_loop_failure(mode_override.as_deref(), error.to_string()),
    }
}

fn run_once(parsed: ParsedArgs, paths: WorkspacePaths) -> CliOutput {
    let mode_override = parsed.value("--mode").map(ToOwned::to_owned);
    let startup_options = RuntimeStartupOptions {
        requested_mode_id: mode_override.clone(),
        config_path: parsed.value("--config").map(PathBuf::from),
        runtime_mode: RuntimeMode::Once,
        ..RuntimeStartupOptions::default()
    };

    let mut session = match crate::runtime::startup_runtime_once_for_paths(&paths, startup_options)
    {
        Ok(session) => session,
        Err(error) => return run_once_startup_failure(mode_override.as_deref(), error),
    };

    let runner = match build_runtime_runner_dispatcher(&session) {
        Ok(runner) => runner,
        Err(error) => {
            let release_result = session.close().map_err(|error| error.to_string());
            return run_once_internal_failure(
                mode_override.as_deref(),
                &session,
                error.to_string(),
                release_result,
            );
        }
    };

    match crate::runtime::run_serial_runtime_tick_with_runner(
        &mut session,
        RuntimeTickOptions::default(),
        &runner,
    ) {
        Ok(outcome) => {
            let release_result = session.finish().map_err(|error| error.to_string());
            render_run_once_outcome(mode_override.as_deref(), outcome, release_result)
        }
        Err(error) => {
            let message = error.to_string();
            let release_result = session.close().map_err(|error| error.to_string());
            run_once_tick_failure(mode_override.as_deref(), &session, message, release_result)
        }
    }
}

fn daemon_loop_options(parsed: &ParsedArgs) -> RuntimeDaemonLoopOptions {
    RuntimeDaemonLoopOptions {
        max_ticks: parsed.value("--max-ticks").map(|value| {
            value
                .parse::<u64>()
                .expect("daemon max ticks were validated before execution")
        }),
        ..RuntimeDaemonLoopOptions::default()
    }
}

fn build_daemon_monitor(parsed: &ParsedArgs) -> Result<Option<RuntimeMonitorFanout>, String> {
    let mut sinks: Vec<Box<dyn RuntimeMonitorSink>> = Vec::new();
    if daemon_stdout_monitor_enabled(parsed) {
        sinks.push(Box::new(BasicTerminalMonitor::new(std::io::stdout())));
    }
    if let Some(path) = parsed.value("--monitor-log") {
        let path = PathBuf::from(path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create monitor log directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("failed to create monitor log {}: {error}", path.display()))?;
        sinks.push(Box::new(BasicTerminalMonitor::new(BufWriter::new(file))));
    }

    if sinks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(RuntimeMonitorFanout::new(sinks)))
    }
}

fn daemon_stdout_monitor_enabled(parsed: &ParsedArgs) -> bool {
    parsed
        .value("--monitor")
        .is_some_and(|value| value.eq_ignore_ascii_case("basic"))
}

fn daemon_startup_monitor_event(
    session: &crate::runtime::RuntimeStartupSession,
) -> RuntimeMonitorEvent {
    let mut payload = Map::new();
    payload.insert(
        "mode_id".to_owned(),
        Value::String(session.snapshot.active_mode_id.clone()),
    );
    payload.insert(
        "compiled_plan_id".to_owned(),
        Value::String(session.snapshot.compiled_plan_id.clone()),
    );
    payload.insert(
        "compiled_plan_currentness".to_owned(),
        Value::String("current".to_owned()),
    );
    match load_baseline_manifest(&session.paths) {
        Ok(manifest) => {
            payload.insert(
                "baseline_manifest_id".to_owned(),
                Value::String(manifest.manifest_id),
            );
            payload.insert(
                "baseline_seed_package_version".to_owned(),
                Value::String(manifest.seed_package_version),
            );
        }
        Err(_) => {
            payload.insert(
                "baseline_manifest_id".to_owned(),
                Value::String("unknown".to_owned()),
            );
            payload.insert(
                "baseline_seed_package_version".to_owned(),
                Value::String("unknown".to_owned()),
            );
        }
    }
    payload.insert(
        "loop_ids_by_plane".to_owned(),
        string_plane_map_value(&session.snapshot.loop_ids_by_plane),
    );
    if let Some(policy) = &session.compiled_plan.concurrency_policy {
        if let Ok(value) = serde_json::to_value(policy) {
            payload.insert("concurrency_policy".to_owned(), value);
        }
    }
    payload.insert(
        "scheduler_mode".to_owned(),
        Value::String(
            if session.compiled_plan.concurrency_policy.is_some() {
                "plane-concurrent"
            } else {
                "serial"
            }
            .to_owned(),
        ),
    );
    payload.insert(
        "status_markers_by_plane".to_owned(),
        string_plane_map_value(&session.snapshot.status_markers_by_plane),
    );
    payload.insert(
        "queue_depths_by_plane".to_owned(),
        number_plane_map_value(&session.snapshot.queue_depths_by_plane),
    );

    RuntimeMonitorEvent::new(
        "runtime_started",
        session.snapshot.updated_at.clone(),
        payload,
    )
}

fn string_plane_map_value(map: &std::collections::HashMap<Plane, String>) -> Value {
    let mut object = Map::new();
    for plane in [Plane::Execution, Plane::Planning, Plane::Learning] {
        if let Some(value) = map.get(&plane) {
            object.insert(plane.as_str().to_owned(), Value::String(value.clone()));
        }
    }
    Value::Object(object)
}

fn number_plane_map_value(map: &std::collections::HashMap<Plane, u64>) -> Value {
    let mut object = Map::new();
    for plane in [Plane::Execution, Plane::Planning, Plane::Learning] {
        if let Some(value) = map.get(&plane) {
            object.insert(plane.as_str().to_owned(), Value::Number((*value).into()));
        }
    }
    Value::Object(object)
}

fn run_once_startup_failure(
    mode_override: Option<&str>,
    error: crate::runtime::RuntimeStartupError,
) -> CliOutput {
    CliOutput::with_exit_code(
        vec![
            format!("error: millrace run once startup failed: {error}"),
            "run_mode: once".to_owned(),
            format!("mode_override: {}", option_text(mode_override)),
            "startup_failed: true".to_owned(),
            "execution_started: false".to_owned(),
            "stage_dispatched: false".to_owned(),
            "runtime_ticks: 0".to_owned(),
            "runtime_session_started: false".to_owned(),
        ],
        1,
    )
}

fn run_daemon_startup_failure(
    mode_override: Option<&str>,
    error: crate::runtime::RuntimeStartupError,
) -> CliOutput {
    CliOutput::with_exit_code(
        vec![
            format!("error: millrace run daemon startup failed: {error}"),
            "run_mode: daemon".to_owned(),
            format!("mode_override: {}", option_text(mode_override)),
            "startup_failed: true".to_owned(),
            "daemon_ownership_acquired: false".to_owned(),
            "runtime_ticks: 0".to_owned(),
        ],
        1,
    )
}

fn run_daemon_internal_failure(
    mode_override: Option<&str>,
    session: &crate::runtime::RuntimeStartupSession,
    error: String,
    runtime_ticks: u64,
    release_result: Result<bool, String>,
) -> CliOutput {
    let mut lines = vec![
        format!("error: millrace run daemon failed before loop execution: {error}"),
        "run_mode: daemon".to_owned(),
        format!("active_mode_id: {}", session.snapshot.active_mode_id),
        format!("mode_override: {}", option_text(mode_override)),
        format!("compiled_plan_id: {}", session.snapshot.compiled_plan_id),
        "daemon_ownership_acquired: true".to_owned(),
        format!("runtime_ticks: {runtime_ticks}"),
    ];
    append_release_lines(&mut lines, release_result.as_ref());
    CliOutput::with_exit_code(lines, 1)
}

fn run_daemon_loop_failure(mode_override: Option<&str>, error: String) -> CliOutput {
    CliOutput::with_exit_code(
        vec![
            format!("error: millrace run daemon loop failed: {error}"),
            "run_mode: daemon".to_owned(),
            format!("mode_override: {}", option_text(mode_override)),
            "daemon_ownership_acquired: true".to_owned(),
            "runtime_ticks: unknown".to_owned(),
        ],
        1,
    )
}

fn run_once_internal_failure(
    mode_override: Option<&str>,
    session: &crate::runtime::RuntimeStartupSession,
    error: String,
    release_result: Result<bool, String>,
) -> CliOutput {
    let mut lines = run_once_failure_context_lines(mode_override, session);
    lines[0] = format!("error: millrace run once failed before tick dispatch: {error}");
    append_release_lines(&mut lines, release_result.as_ref());
    CliOutput::with_exit_code(lines, 1)
}

fn run_once_tick_failure(
    mode_override: Option<&str>,
    session: &crate::runtime::RuntimeStartupSession,
    error: String,
    release_result: Result<bool, String>,
) -> CliOutput {
    let mut lines = run_once_failure_context_lines(mode_override, session);
    lines[0] = format!("error: millrace run once tick failed: {error}");
    append_release_lines(&mut lines, release_result.as_ref());
    CliOutput::with_exit_code(lines, 1)
}

fn run_once_failure_context_lines(
    mode_override: Option<&str>,
    session: &crate::runtime::RuntimeStartupSession,
) -> Vec<String> {
    vec![
        String::new(),
        "run_mode: once".to_owned(),
        format!("active_mode_id: {}", session.snapshot.active_mode_id),
        format!("mode_override: {}", option_text(mode_override)),
        format!("compiled_plan_id: {}", session.snapshot.compiled_plan_id),
        "execution_started: false".to_owned(),
        "stage_dispatched: false".to_owned(),
        "runtime_ticks: 0".to_owned(),
    ]
}

fn render_run_once_outcome(
    mode_override: Option<&str>,
    outcome: RuntimeTickDispatchOutcome,
    release_result: Result<bool, String>,
) -> CliOutput {
    let mut lines = vec![
        "run_mode: once".to_owned(),
        format!("active_mode_id: {}", outcome.snapshot.active_mode_id),
        format!("mode_override: {}", option_text(mode_override)),
        format!("compiled_plan_id: {}", outcome.snapshot.compiled_plan_id),
        format!("tick_outcome: {}", outcome.kind.as_str()),
        format!("tick_reason: {}", run_once_tick_reason(&outcome)),
        "runtime_ticks: 1".to_owned(),
        format!(
            "execution_started: {}",
            bool_text(outcome.stage_request.is_some())
        ),
        format!(
            "stage_dispatched: {}",
            bool_text(outcome.runner_raw_result.is_some())
        ),
    ];

    if let Some(request) = &outcome.stage_request {
        lines.push(format!("run_id: {}", request.run_id));
        lines.push(format!("request_id: {}", request.request_id));
        lines.push(format!("plane: {}", request.plane.as_str()));
        lines.push(format!("stage: {}", request.stage.as_str()));
        lines.push(format!("node_id: {}", request.node_id));
        lines.push(format!("stage_kind_id: {}", request.stage_kind_id));
        lines.push(format!("request_kind: {}", request.request_kind.as_str()));
        lines.push(format!(
            "work_item_kind: {}",
            request
                .active_work_item_kind
                .map(|kind| kind.as_str())
                .unwrap_or("none")
        ));
        lines.push(format!(
            "work_item_id: {}",
            option_text(request.active_work_item_id.as_deref())
        ));
    }

    if let Some(raw_result) = &outcome.runner_raw_result {
        lines.push("runner_adapter: stage_runner_dispatcher".to_owned());
        lines.push(format!("runner_name: {}", raw_result.runner_name));
        lines.push(format!(
            "runner_exit_kind: {}",
            raw_result.exit_kind.as_str()
        ));
    }
    if let Some(stage_result) = &outcome.stage_result {
        lines.push(format!(
            "terminal_result: {}",
            stage_result.terminal_result.as_str()
        ));
        lines.push(format!(
            "result_class: {}",
            stage_result.result_class.as_str()
        ));
    }
    if let Some(decision) = &outcome.router_decision {
        lines.push(format!("router_action: {}", decision.action.as_str()));
        lines.push(format!("router_reason: {}", decision.reason));
        lines.push(format!(
            "next_stage: {}",
            decision
                .next_stage
                .map(|stage| stage.as_str())
                .unwrap_or("none")
        ));
    }

    append_path_line(
        &mut lines,
        "stage_request_path",
        outcome.stage_request_path.as_ref(),
    );
    append_path_line(
        &mut lines,
        "runner_raw_result_path",
        outcome.runner_raw_result_path.as_ref(),
    );
    append_path_line(
        &mut lines,
        "stage_result_path",
        outcome.stage_result_path.as_ref(),
    );
    append_path_line(
        &mut lines,
        "router_decision_path",
        outcome.router_decision_path.as_ref(),
    );
    append_path_line(
        &mut lines,
        "runtime_error_context_path",
        outcome.runtime_error_context_path.as_ref(),
    );
    append_path_line(
        &mut lines,
        "event_log_path",
        outcome.event_log_path.as_ref(),
    );
    let release_result_for_output = release_result_for_outcome(&outcome, &release_result);
    append_release_lines(&mut lines, release_result_for_output.as_ref());

    let exit_code = run_once_exit_code(&outcome, release_result.as_ref());
    CliOutput::with_exit_code(lines, exit_code)
}

fn render_run_daemon_outcome(
    mode_override: Option<&str>,
    outcome: crate::runtime::RuntimeDaemonLoopOutcome,
) -> CliOutput {
    let mut lines = vec![
        "run_mode: daemon".to_owned(),
        format!("active_mode_id: {}", outcome.final_snapshot.active_mode_id),
        format!("mode_override: {}", option_text(mode_override)),
        format!(
            "compiled_plan_id: {}",
            outcome.final_snapshot.compiled_plan_id
        ),
        format!("exit_reason: {}", outcome.exit_reason.as_str()),
        format!("runtime_ticks: {}", outcome.completed_tick_count),
        format!("ticks: {}", outcome.completed_tick_count),
        format!("idle_sleep_count: {}", outcome.idle_sleep_count),
        format!("cycle_count: {}", outcome.cycle_outcomes.len()),
        format!(
            "post_cycle_completion_count: {}",
            outcome.post_cycle_completions.len()
        ),
        format!(
            "shutdown_completion_count: {}",
            outcome.shutdown_completions.len()
        ),
        "daemon_ownership_acquired: true".to_owned(),
        "runtime_ownership_release_ok: true".to_owned(),
        format!(
            "runtime_ownership_released: {}",
            bool_text(outcome.runtime_ownership_released)
        ),
        format!(
            "process_running: {}",
            bool_text(outcome.final_snapshot.process_running)
        ),
    ];
    if let Some(active_plane) = outcome.final_snapshot.active_plane {
        lines.push(format!("active_plane: {}", active_plane.as_str()));
    }
    let exit_code = if outcome.exit_reason == RuntimeDaemonLoopExitReason::Blocked
        || !outcome.runtime_ownership_released
    {
        1
    } else {
        0
    };
    CliOutput::with_exit_code(lines, exit_code)
}

fn run_once_tick_reason(outcome: &RuntimeTickDispatchOutcome) -> &str {
    outcome
        .router_decision
        .as_ref()
        .map(|decision| decision.reason.as_str())
        .unwrap_or(&outcome.reason)
}

fn run_once_exit_code(
    outcome: &RuntimeTickDispatchOutcome,
    release_result: Result<&bool, &String>,
) -> u8 {
    if release_result.is_err() {
        return 1;
    }
    if outcome.kind == RuntimeTickOutcomeKind::Blocked {
        return 1;
    }
    if let Some(stage_result) = &outcome.stage_result {
        if matches!(
            stage_result.result_class,
            ResultClass::RecoverableFailure | ResultClass::Blocked
        ) {
            return 1;
        }
    }
    0
}

fn append_path_line(lines: &mut Vec<String>, label: &str, path: Option<&PathBuf>) {
    if let Some(path) = path {
        lines.push(format!("{label}: {}", path.display()));
    }
}

fn release_result_for_outcome(
    outcome: &RuntimeTickDispatchOutcome,
    release_result: &Result<bool, String>,
) -> Result<bool, String> {
    match release_result {
        Ok(released) => Ok(*released || outcome.kind == RuntimeTickOutcomeKind::Stopped),
        Err(error) => Err(error.clone()),
    }
}

fn append_release_lines(lines: &mut Vec<String>, release_result: Result<&bool, &String>) {
    match release_result {
        Ok(released) => {
            lines.push("runtime_ownership_release_ok: true".to_owned());
            lines.push(format!(
                "runtime_ownership_released: {}",
                bool_text(*released)
            ));
        }
        Err(error) => {
            lines.push("runtime_ownership_release_ok: false".to_owned());
            lines.push(format!("runtime_ownership_release_error: {error}"));
        }
    }
}

fn run_status_group(args: Vec<String>) -> CliOutput {
    let specs = [
        repeatable_workspace_spec(),
        value_spec("--max-updates", EmptyValue::NonBlank),
        value_spec("--interval-seconds", EmptyValue::NonBlank),
    ];
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    let command = match optional_command(&parsed, &["show", "watch"]) {
        Ok(command) => command.unwrap_or("show"),
        Err(output) => return output,
    };
    if let Err(output) = reject_extra_positionals(&parsed, 1) {
        return output;
    }
    let paths_list = match require_status_workspaces(&parsed) {
        Ok(paths_list) => paths_list,
        Err(output) => return output,
    };
    let lines = if command == "watch" {
        read_only::status_watch_lines(
            &paths_list,
            parsed.value("--max-updates"),
            parsed.value("--interval-seconds"),
        )
    } else if paths_list.len() == 1 {
        read_only::status_lines(&paths_list[0])
    } else {
        read_only::status_watch_lines(&paths_list, Some("1"), Some("0"))
    };
    lines
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure)
}

fn run_runs_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error("missing runs command `ls`, `show`, `tail`, or `trace`");
    }
    let command = args.remove(0);
    if !matches!(command.as_str(), "ls" | "show" | "tail" | "trace") {
        return CliOutput::parse_error(format!("unknown runs command `{command}`"));
    }
    let specs = if command == "trace" {
        vec![
            workspace_spec(),
            value_spec("--format", EmptyValue::NonBlank),
            value_spec("--output", EmptyValue::NonEmpty),
        ]
    } else {
        vec![workspace_spec()]
    };
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if matches!(command.as_str(), "show" | "tail" | "trace") {
        if let Err(output) = require_one_positional(&parsed, "RUN_ID") {
            return output;
        }
    } else if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    if let Err(output) = reject_extra_positionals(&parsed, (command != "ls") as usize) {
        return output;
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    match command.as_str() {
        "ls" => read_only::runs_ls_lines(&paths)
            .map(CliOutput::success)
            .unwrap_or_else(CliOutput::stdout_failure),
        "show" => read_only::runs_show_lines(&paths, &parsed.positionals[0])
            .map(CliOutput::success)
            .unwrap_or_else(CliOutput::stdout_failure),
        "tail" => read_only::runs_tail_payload(&paths, &parsed.positionals[0])
            .map(|payload| CliOutput::success(vec![payload]))
            .unwrap_or_else(CliOutput::stdout_failure),
        "trace" => run_runs_trace(&paths, &parsed),
        _ => unreachable!("runs command validated above"),
    }
}

fn run_runs_trace(paths: &WorkspacePaths, parsed: &ParsedArgs) -> CliOutput {
    let format = match render_format(parsed.value("--format").unwrap_or("text")) {
        Ok(format) => format,
        Err(output) => return output,
    };
    let trace = match read_only::runs_trace_graph(paths, &parsed.positionals[0]) {
        Ok(trace) => trace,
        Err(error) => return CliOutput::stdout_failure(error),
    };
    match format {
        GraphTraceOutputFormat::Text => {
            write_or_print_rendered(render::run_trace_lines(&trace).join("\n"), parsed)
        }
        GraphTraceOutputFormat::Json => match serde_json::to_string_pretty(&trace) {
            Ok(rendered) => write_or_print_rendered(rendered, parsed),
            Err(error) => {
                CliOutput::stdout_failure(format!("failed to render trace JSON: {error}"))
            }
        },
    }
}

fn run_queue_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error(
            "missing queue command `ls`, `show`, `add-task`, `add-probe`, `add-spec`, `add-idea`, or `repair-lineage`",
        );
    }
    let command = args.remove(0);
    run_queue_alias("queue", &command, args)
}

fn run_queue_alias(alias_context: &str, command: &str, args: Vec<String>) -> CliOutput {
    if !matches!(
        command,
        "ls" | "show" | "add-task" | "add-probe" | "add-spec" | "add-idea" | "repair-lineage"
    ) {
        return CliOutput::parse_error(format!("unknown queue command `{command}`"));
    }
    let specs = if command == "repair-lineage" {
        vec![
            workspace_spec(),
            value_spec("--root-spec-id", EmptyValue::NonBlank),
            flag_spec("--apply"),
        ]
    } else {
        vec![workspace_spec()]
    };
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };

    match command {
        "ls" => {
            if let Err(output) = reject_positionals(&parsed) {
                return output;
            }
        }
        "show" => {
            if let Err(output) = require_one_positional(&parsed, "WORK_ITEM_ID") {
                return output;
            }
        }
        "add-task" => {
            if let Err(output) = require_one_positional(&parsed, "TASK_PATH") {
                return output;
            }
        }
        "add-probe" => {
            if let Err(output) = require_one_positional(&parsed, "PROBE_PATH") {
                return output;
            }
        }
        "add-spec" => {
            if let Err(output) = require_one_positional(&parsed, "SPEC_PATH") {
                return output;
            }
        }
        "add-idea" => {
            if let Err(output) = require_one_positional(&parsed, "IDEA_PATH") {
                return output;
            }
        }
        "repair-lineage" => {
            if let Err(output) = reject_positionals(&parsed) {
                return output;
            }
            if parsed.value("--root-spec-id").is_none() {
                return CliOutput::parse_error(
                    "missing required option `--root-spec-id <ROOT_SPEC_ID>`",
                );
            }
        }
        _ => unreachable!("queue command validated above"),
    }
    if let Err(output) = reject_extra_positionals(
        &parsed,
        (command != "ls" && command != "repair-lineage") as usize,
    ) {
        return output;
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    match command {
        "ls" if alias_context == "queue" => read_only::queue_ls_lines(&paths)
            .map(CliOutput::success)
            .unwrap_or_else(CliOutput::stdout_failure),
        "show" if alias_context == "queue" => read_only::queue_show_lines(
            &paths,
            parsed
                .positionals
                .first()
                .expect("queue show has one positional"),
        )
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure),
        "add-task" => intake::add_task_lines(
            &paths,
            parsed
                .positionals
                .first()
                .expect("queue add-task has one positional"),
        )
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure),
        "add-probe" => intake::add_probe_lines(
            &paths,
            parsed
                .positionals
                .first()
                .expect("queue add-probe has one positional"),
        )
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure),
        "add-spec" => intake::add_spec_lines(
            &paths,
            parsed
                .positionals
                .first()
                .expect("queue add-spec has one positional"),
        )
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure),
        "add-idea" => intake::add_idea_lines(
            &paths,
            parsed
                .positionals
                .first()
                .expect("queue add-idea has one positional"),
        )
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure),
        "repair-lineage" if alias_context == "queue" => run_queue_repair_lineage(
            &paths,
            parsed
                .value("--root-spec-id")
                .expect("queue repair-lineage has root spec id"),
            parsed.has("--apply"),
        ),
        _ => unreachable!("queue command validated above"),
    }
}

fn run_queue_repair_lineage(paths: &WorkspacePaths, root_spec_id: &str, apply: bool) -> CliOutput {
    match repair_closure_lineage(paths, root_spec_id, apply) {
        Ok(outcome) => CliOutput::success(render_lineage_repair_lines(&outcome, apply)),
        Err(error) => lineage_repair_failure_output(error),
    }
}

fn render_lineage_repair_lines(outcome: &ClosureLineageRepairOutcome, apply: bool) -> Vec<String> {
    let mut repaired_items = BTreeSet::new();
    for change in &outcome.plan.changes {
        repaired_items.insert(format!(
            "{}\0{}\0{}",
            change.work_item_kind, change.work_item_id, change.path
        ));
    }
    let repair_report_path = outcome
        .applied_report_path
        .as_ref()
        .unwrap_or(&outcome.preview_report_path);
    let mut lines = vec![
        format!("root_spec_id: {}", outcome.target.root_spec_id),
        format!("apply: {}", bool_text(apply)),
        format!("repair_count: {}", repaired_items.len()),
        format!("change_count: {}", outcome.plan.changes.len()),
        format!("repaired_count: {}", outcome.repaired_count),
        format!("skipped_count: {}", outcome.plan.skipped_findings.len()),
        format!("repair_report: {}", repair_report_path.display()),
    ];
    for change in &outcome.plan.changes {
        lines.push(format!(
            "change: {} {} {} {} -> {}",
            change.work_item_kind,
            change.work_item_id,
            change.field_name,
            change.old_value.as_deref().unwrap_or("None"),
            change.new_value
        ));
    }
    for finding in &outcome.plan.skipped_findings {
        lines.push(format!(
            "skipped: {} {} state={}",
            finding.work_item_kind, finding.work_item_id, finding.state
        ));
    }
    lines
}

fn lineage_repair_failure_output(error: LineageRepairError) -> CliOutput {
    match error {
        LineageRepairError::ActiveRuntimeOwnershipLock { .. } => {
            CliOutput::stdout_failure("active runtime ownership lock prevents lineage repair")
        }
        LineageRepairError::ActiveRuntimeStage { .. } => {
            CliOutput::stdout_failure("active runtime stage prevents lineage repair")
        }
        error @ (LineageRepairError::MissingClosureTarget { .. }
        | LineageRepairError::JsonSyntax { .. }
        | LineageRepairError::NonObjectPayload { .. }
        | LineageRepairError::ClosureTargetContract { .. }) => {
            CliOutput::stdout_failure(format!("failed to load closure target: {error}"))
        }
        error => CliOutput::stdout_failure(error.to_string()),
    }
}

fn run_planning_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error("missing planning command `retry-active`");
    }
    let command = args.remove(0);
    if command != "retry-active" {
        return CliOutput::parse_error(format!("unknown planning command `{command}`"));
    }
    let parsed = match parse_or_output(
        args,
        &[
            workspace_spec(),
            value_spec("--reason", EmptyValue::NonBlank),
        ],
    ) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    let reason = parsed
        .value("--reason")
        .unwrap_or("operator requested planning retry");
    match RuntimeControl::from_paths(paths)
        .and_then(|control| control.retry_active_planning(reason))
    {
        Ok(result) => CliOutput::success(render_control_result(&result)),
        Err(error) => CliOutput::stdout_failure(error.to_string()),
    }
}

fn run_config_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error("missing config command `show`, `validate`, or `reload`");
    }
    let command = args.remove(0);
    let specs = match command.as_str() {
        "show" => vec![workspace_spec(), config_spec()],
        "validate" => vec![workspace_spec(), config_spec(), mode_spec()],
        "reload" => vec![workspace_spec()],
        _ => return CliOutput::parse_error(format!("unknown config command `{command}`")),
    };
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    match command.as_str() {
        "show" => read_only::config_show_lines(&paths, parsed.value("--config"))
            .map(CliOutput::success)
            .unwrap_or_else(CliOutput::stdout_failure),
        "validate" => {
            let resolved_config_path = parsed
                .value("--config")
                .map(PathBuf::from)
                .unwrap_or_else(|| paths.runtime_config_file.clone());
            if let Err(error) = load_runtime_startup_config(&resolved_config_path) {
                return CliOutput::stdout_failure(error.to_string());
            }
            let compiler_options = CompileWorkspaceOptions {
                requested_mode_id: parsed.value("--mode").map(ToOwned::to_owned),
                config_path: parsed.value("--config").map(PathBuf::from),
                ..CompileWorkspaceOptions::default()
            };
            match crate::compiler::compile_and_persist_workspace_plan_for_paths(
                &paths,
                compiler_options,
            ) {
                Ok(outcome) => CliOutput::with_exit_code(
                    render_compile_diagnostics_lines(&outcome),
                    compile_exit_code(&outcome),
                ),
                Err(error) => CliOutput::stdout_failure(error.to_string()),
            }
        }
        "reload" => {
            match RuntimeControl::from_paths(paths).and_then(|control| control.reload_config()) {
                Ok(result) => CliOutput::success(render_control_result(&result)),
                Err(error) => CliOutput::stdout_failure(error.to_string()),
            }
        }
        _ => unreachable!("config command validated above"),
    }
}

fn run_control_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error(
            "missing control command `pause`, `resume`, `stop`, `retry-active`, `clear-stale-state`, or `reload-config`",
        );
    }
    let command = args.remove(0);
    run_control_command(&command, args, false)
}

fn run_control_alias(command: &str, args: Vec<String>) -> CliOutput {
    run_control_command(command, args, true)
}

fn run_control_command(command: &str, args: Vec<String>, alias: bool) -> CliOutput {
    if !matches!(
        command,
        "pause" | "resume" | "stop" | "retry-active" | "clear-stale-state" | "reload-config"
    ) {
        return CliOutput::parse_error(format!("unknown control command `{command}`"));
    }
    let specs = if matches!(command, "retry-active" | "clear-stale-state") {
        vec![
            workspace_spec(),
            value_spec("--reason", EmptyValue::NonBlank),
        ]
    } else {
        vec![workspace_spec()]
    };
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    let reason = parsed.value("--reason");
    let result = RuntimeControl::from_paths(paths).and_then(|control| match command {
        "pause" => control.pause_runtime(),
        "resume" => control.resume_runtime(),
        "stop" => control.stop_runtime(),
        "retry-active" => control.retry_active(reason.unwrap_or("operator requested retry")),
        "clear-stale-state" => {
            control.clear_stale_state(reason.unwrap_or("operator requested stale-state clear"))
        }
        "reload-config" => control.reload_config(),
        _ => unreachable!("control command validated above"),
    });
    match result {
        Ok(result) => CliOutput::success(render_control_result(&result)),
        Err(error) => {
            let command_path = if alias {
                command.to_owned()
            } else {
                format!("control {command}")
            };
            CliOutput::stdout_failure(format!("{command_path}: {error}"))
        }
    }
}

fn run_modes_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error("missing modes command `list` or `show`");
    }
    let command = args.remove(0);
    let parsed = match parse_or_output(args, &[]) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    match command.as_str() {
        "list" => {
            if let Err(output) = reject_positionals(&parsed) {
                return output;
            }
        }
        "show" => {
            if let Err(output) = require_one_positional(&parsed, "MODE_ID") {
                return output;
            }
            if let Err(output) = reject_extra_positionals(&parsed, 1) {
                return output;
            }
        }
        _ => return CliOutput::parse_error(format!("unknown modes command `{command}`")),
    }
    match command.as_str() {
        "list" => read_only::modes_list_lines()
            .map(CliOutput::success)
            .unwrap_or_else(CliOutput::stdout_failure),
        "show" => read_only::modes_show_lines(&parsed.positionals[0])
            .map(CliOutput::success)
            .unwrap_or_else(CliOutput::stdout_failure),
        _ => unreachable!("modes command validated above"),
    }
}

fn run_skills_group(mut args: Vec<String>) -> CliOutput {
    if args.is_empty() {
        return CliOutput::parse_error(
            "missing skills command `ls`, `show`, `search`, `install`, `refresh-remote-index`, `create`, `improve`, `promote`, or `export`",
        );
    }
    let command = args.remove(0);
    let specs = [
        workspace_spec(),
        value_spec("--target", EmptyValue::NonBlank),
        flag_spec("--force"),
        flag_spec("--update"),
        mode_spec(),
        value_spec("--output", EmptyValue::NonEmpty),
        flag_spec("--foreground"),
    ];
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    let required_positionals = match command.as_str() {
        "ls" | "refresh-remote-index" => 0,
        "show" | "search" | "install" | "create" | "improve" | "promote" | "export" => 1,
        _ => return CliOutput::parse_error(format!("unknown skills command `{command}`")),
    };
    if required_positionals == 0 {
        if let Err(output) = reject_positionals(&parsed) {
            return output;
        }
    } else {
        if let Err(output) = require_one_positional(&parsed, "SKILL_ARG") {
            return output;
        }
        if let Err(output) = reject_extra_positionals(&parsed, 1) {
            return output;
        }
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    skills::skills_command_lines(&command, &parsed, &paths)
        .map(CliOutput::success)
        .unwrap_or_else(CliOutput::stdout_failure)
}

fn run_upgrade_group(args: Vec<String>) -> CliOutput {
    let specs = [
        workspace_spec(),
        flag_spec("--apply"),
        OptionSpec::repeatable_value("--localize-removed", EmptyValue::NonEmpty),
        value_spec("--localize-removed-from", EmptyValue::NonEmpty),
    ];
    let parsed = match parse_or_output(args, &specs) {
        Ok(parsed) => parsed,
        Err(output) => return output,
    };
    if let Err(output) = reject_positionals(&parsed) {
        return output;
    }
    let paths = match require_default_workspace(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    let localize_removed_paths = match collect_localize_removed_paths(&parsed) {
        Ok(paths) => paths,
        Err(output) => return output,
    };
    let result = if parsed.has("--apply") {
        apply_baseline_upgrade(&paths, &localize_removed_paths)
    } else {
        preview_baseline_upgrade(&paths, &localize_removed_paths)
    };
    result
        .map(|preview| CliOutput::success(render_upgrade_lines(&preview)))
        .unwrap_or_else(|error| CliOutput::stdout_failure(error.to_string()))
}

fn collect_localize_removed_paths(parsed: &ParsedArgs) -> Result<Vec<String>, CliOutput> {
    let mut paths: Vec<String> = parsed
        .values("--localize-removed")
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    let Some(path) = parsed.value("--localize-removed-from") else {
        return Ok(paths);
    };
    let raw = fs::read_to_string(path)
        .map_err(|error| CliOutput::stdout_failure(format!("failed to read {path}: {error}")))?;
    paths.extend(raw.lines().filter_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }));
    Ok(paths)
}

fn render_upgrade_lines(preview: &BaselineUpgradePreview) -> Vec<String> {
    let mut lines = vec![
        format!("applied: {}", bool_text(preview.applied)),
        format!("baseline_manifest_id: {}", preview.baseline_manifest_id),
        format!("candidate_manifest_id: {}", preview.candidate_manifest_id),
    ];
    if preview.applied {
        lines.push(format!(
            "result_manifest_id: {}",
            preview.candidate_manifest_id
        ));
    }
    for (disposition, count) in preview.counts_by_disposition() {
        lines.push(format!("{}: {count}", disposition.as_str()));
    }
    for entry in &preview.entries {
        lines.push(format!(
            "entry: {} {}",
            entry.relative_path,
            entry.disposition.as_str()
        ));
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompileCliCommand {
    Validate,
    Show,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompileCliOptions {
    workspace: String,
    mode: Option<String>,
    config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphTraceOutputFormat {
    Text,
    Json,
}

fn render_format(value: &str) -> Result<GraphTraceOutputFormat, CliOutput> {
    match value {
        "text" => Ok(GraphTraceOutputFormat::Text),
        "json" => Ok(GraphTraceOutputFormat::Json),
        _ => Err(CliOutput::stdout_failure("--format must be text or json")),
    }
}

fn write_or_print_rendered(rendered: String, parsed: &ParsedArgs) -> CliOutput {
    let Some(output) = parsed.value("--output") else {
        return CliOutput::success(vec![rendered]);
    };
    match fs::write(output, format!("{rendered}\n")) {
        Ok(()) => CliOutput::success(Vec::new()),
        Err(error) => {
            CliOutput::stdout_failure(format!("failed to write output {output}: {error}"))
        }
    }
}

fn parse_or_output(args: Vec<String>, specs: &[OptionSpec]) -> Result<ParsedArgs, CliOutput> {
    parse_args(args, specs).map_err(CliOutput::parse_error)
}

fn reject_positionals(parsed: &ParsedArgs) -> Result<(), CliOutput> {
    reject_extra_positionals(parsed, 0)
}

fn reject_extra_positionals(parsed: &ParsedArgs, allowed: usize) -> Result<(), CliOutput> {
    if parsed.positionals.len() > allowed {
        return Err(CliOutput::parse_error(format!(
            "unexpected argument `{}`",
            parsed.positionals[allowed]
        )));
    }
    Ok(())
}

fn require_one_positional(parsed: &ParsedArgs, name: &str) -> Result<(), CliOutput> {
    if parsed.positionals.is_empty() {
        return Err(CliOutput::parse_error(format!(
            "missing required argument `{name}`"
        )));
    }
    Ok(())
}

fn optional_command<'a>(
    parsed: &'a ParsedArgs,
    allowed: &[&'static str],
) -> Result<Option<&'a str>, CliOutput> {
    let Some(command) = parsed.positionals.first() else {
        return Ok(None);
    };
    if allowed.contains(&command.as_str()) {
        Ok(Some(command))
    } else {
        Err(CliOutput::parse_error(format!(
            "unknown status command `{command}`"
        )))
    }
}

fn require_default_workspace(parsed: &ParsedArgs) -> Result<WorkspacePaths, CliOutput> {
    require_workspace(parsed.value("--workspace").unwrap_or("."))
}

fn require_status_workspaces(parsed: &ParsedArgs) -> Result<Vec<WorkspacePaths>, CliOutput> {
    let workspaces = parsed.values("--workspace");
    if workspaces.is_empty() {
        return require_workspace(".").map(|paths| vec![paths]);
    }
    let mut paths_list = Vec::new();
    for workspace in workspaces {
        let paths = require_workspace(workspace)?;
        if !paths_list
            .iter()
            .any(|existing: &WorkspacePaths| existing.root == paths.root)
        {
            paths_list.push(paths);
        }
    }
    Ok(paths_list)
}

fn require_workspace(workspace: &str) -> Result<WorkspacePaths, CliOutput> {
    crate::require_initialized_workspace(workspace)
        .map_err(|error| CliOutput::stdout_failure(error.to_string()))
}

fn validate_daemon_max_ticks(parsed: &ParsedArgs) -> Result<(), CliOutput> {
    let Some(value) = parsed.value("--max-ticks") else {
        return Ok(());
    };
    match value.parse::<u64>() {
        Ok(value) if value >= 1 => Ok(()),
        _ => Err(CliOutput::parse_error(
            "`--max-ticks` value must be an integer greater than or equal to 1",
        )),
    }
}

fn validate_daemon_monitor(parsed: &ParsedArgs) -> Result<(), CliOutput> {
    let Some(value) = parsed.value("--monitor") else {
        return Ok(());
    };
    if matches!(value.to_ascii_lowercase().as_str(), "none" | "basic") {
        Ok(())
    } else {
        Err(CliOutput::stdout_failure(format!(
            "unknown monitor mode: {value}"
        )))
    }
}

fn render_control_result(result: &RuntimeControlActionResult) -> Vec<String> {
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
    lines
}

fn workspace_spec() -> OptionSpec {
    value_spec("--workspace", EmptyValue::NonEmpty)
}

fn repeatable_workspace_spec() -> OptionSpec {
    OptionSpec::repeatable_value("--workspace", EmptyValue::NonEmpty)
}

fn mode_spec() -> OptionSpec {
    value_spec("--mode", EmptyValue::NonBlank)
}

fn config_spec() -> OptionSpec {
    value_spec("--config", EmptyValue::NonEmpty)
}

fn value_spec(name: &'static str, empty_value: EmptyValue) -> OptionSpec {
    OptionSpec::value(name, empty_value)
}

fn flag_spec(name: &'static str) -> OptionSpec {
    OptionSpec::flag(name)
}

fn issue_location(issue: &crate::DoctorIssue) -> String {
    issue
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_owned())
}

fn render_compile_diagnostics_lines(outcome: &CompileOutcome) -> Vec<String> {
    let mut lines = vec![
        format!("ok: {}", bool_text(outcome.diagnostics.ok)),
        format!("mode_id: {}", outcome.diagnostics.mode_id),
        format!(
            "used_last_known_good: {}",
            bool_text(outcome.used_last_known_good)
        ),
    ];
    if let Some(fingerprint) = &outcome.compile_input_fingerprint {
        lines.extend([
            format!("compile_input.mode_id: {}", fingerprint.mode_id),
            format!(
                "compile_input.config_fingerprint: {}",
                fingerprint.config_fingerprint
            ),
            format!(
                "compile_input.assets_fingerprint: {}",
                fingerprint.assets_fingerprint
            ),
        ]);
    }
    for warning in &outcome.diagnostics.warnings {
        lines.push(format!("warning: {warning}"));
    }
    for error in &outcome.diagnostics.errors {
        lines.push(format!("error: {error}"));
    }
    lines
}

fn compile_exit_code(outcome: &CompileOutcome) -> u8 {
    if outcome.diagnostics.ok { 0 } else { 1 }
}

fn render_compile_show_lines(paths: &WorkspacePaths, outcome: &CompileOutcome) -> Vec<String> {
    let Some(plan) = outcome.active_plan.as_ref() else {
        return Vec::new();
    };
    let currentness = crate::compiler::inspect_workspace_plan_currentness_for_paths(
        paths,
        Some(&outcome.diagnostics.mode_id),
    )
    .ok();
    let currentness_state = currentness
        .as_ref()
        .map(|currentness| currentness.state.as_str())
        .unwrap_or_else(|| fallback_currentness_state(plan, outcome));
    let expected_fingerprint = currentness
        .as_ref()
        .map(|currentness| &currentness.expected_fingerprint)
        .or(outcome.compile_input_fingerprint.as_ref())
        .unwrap_or(&plan.compile_input_fingerprint);
    let persisted_fingerprint = currentness
        .as_ref()
        .and_then(|currentness| currentness.persisted_fingerprint.as_ref())
        .unwrap_or(&plan.compile_input_fingerprint);

    let mut lines = vec![
        format!("compiled_plan_currentness: {currentness_state}"),
        format!("compiled_plan_id: {}", plan.compiled_plan_id),
        format!("execution_loop_id: {}", plan.execution_loop_id),
        format!("planning_loop_id: {}", plan.planning_loop_id),
    ];
    if let Some(learning_loop_id) = &plan.learning_loop_id {
        lines.push(format!("learning_loop_id: {learning_loop_id}"));
    }
    for plane in [Plane::Execution, Plane::Planning, Plane::Learning] {
        if let Some(loop_id) = plan.loop_ids_by_plane.get(&plane) {
            lines.push(format!("loop_id: {} -> {loop_id}", plane.as_str()));
        }
    }

    lines.extend(render_baseline_manifest_lines(paths));
    lines.push(format!(
        "compile_input.mode_id: {}",
        expected_fingerprint.mode_id
    ));
    lines.push(format!(
        "compile_input.config_fingerprint: {}",
        expected_fingerprint.config_fingerprint
    ));
    lines.push(format!(
        "compile_input.assets_fingerprint: {}",
        expected_fingerprint.assets_fingerprint
    ));
    lines.push(format!(
        "persisted_compile_input.mode_id: {}",
        persisted_fingerprint.mode_id
    ));
    lines.push(format!(
        "persisted_compile_input.config_fingerprint: {}",
        persisted_fingerprint.config_fingerprint
    ));
    lines.push(format!(
        "persisted_compile_input.assets_fingerprint: {}",
        persisted_fingerprint.assets_fingerprint
    ));

    for graph in ordered_graphs(plan) {
        for entry in &graph.compiled_entries {
            lines.push(format!(
                "entry: {}.{} -> {}",
                entry.plane.as_str(),
                entry.entry_key.as_str(),
                entry.node_id
            ));
        }
    }
    if let Some(completion_entry) = &plan.planning_graph.compiled_completion_entry {
        lines.push(format!(
            "completion: {} -> {}",
            completion_entry.entry_key.as_str(),
            completion_entry.node_id
        ));
    }
    if let Some(completion_behavior) = &plan.planning_graph.completion_behavior {
        lines.extend([
            format!(
                "completion_behavior.trigger: {}",
                completion_behavior.trigger
            ),
            format!(
                "completion_behavior.readiness_rule: {}",
                completion_behavior.readiness_rule
            ),
            format!(
                "completion_behavior.request_kind: {}",
                completion_behavior.request_kind
            ),
            format!(
                "completion_behavior.target_selector: {}",
                completion_behavior.target_selector
            ),
            format!(
                "completion_behavior.rubric_policy: {}",
                completion_behavior.rubric_policy
            ),
            format!(
                "completion_behavior.blocked_work_policy: {}",
                completion_behavior.blocked_work_policy
            ),
            format!(
                "completion_behavior.skip_if_already_closed: {}",
                bool_text(completion_behavior.skip_if_already_closed)
            ),
            format!(
                "completion_behavior.on_pass_terminal_state_id: {}",
                completion_behavior.on_pass_terminal_state_id
            ),
            format!(
                "completion_behavior.on_gap_terminal_state_id: {}",
                completion_behavior.on_gap_terminal_state_id
            ),
            format!(
                "completion_behavior.create_incident_on_gap: {}",
                bool_text(completion_behavior.create_incident_on_gap)
            ),
        ]);
    }

    lines.extend(render_learning_trigger_lines(plan));
    lines.extend(render_concurrency_policy_lines(plan));

    for graph in ordered_graphs(plan) {
        for (index, node) in graph.nodes.iter().enumerate() {
            lines.push(format!(
                "node_order: {}.{index} -> {}",
                graph.plane.as_str(),
                node.node_id
            ));
            lines.extend(render_stage_lines(node));
        }
    }

    lines
}

fn fallback_currentness_state(plan: &CompiledRunPlan, outcome: &CompileOutcome) -> &'static str {
    match &outcome.compile_input_fingerprint {
        Some(fingerprint) if fingerprint != &plan.compile_input_fingerprint => "stale",
        _ => "current",
    }
}

fn render_baseline_manifest_lines(paths: &WorkspacePaths) -> Vec<String> {
    match load_baseline_manifest(paths) {
        Ok(manifest) => vec![
            format!("baseline_manifest_id: {}", manifest.manifest_id),
            format!(
                "baseline_seed_package_version: {}",
                manifest.seed_package_version
            ),
        ],
        Err(_) => vec![
            "baseline_manifest_id: none".to_owned(),
            "baseline_seed_package_version: none".to_owned(),
        ],
    }
}

fn render_learning_trigger_lines(plan: &CompiledRunPlan) -> Vec<String> {
    if plan.learning_trigger_rules.is_empty() {
        return vec!["learning_triggers: none".to_owned()];
    }

    let mut lines = vec![format!(
        "learning_triggers: {}",
        plan.learning_trigger_rules.len()
    )];
    for rule in &plan.learning_trigger_rules {
        lines.extend([
            format!("learning_trigger: {}", rule.rule_id),
            format!(
                "learning_trigger.source: {}.{}",
                rule.source_plane.as_str(),
                rule.source_stage.as_str()
            ),
            format!(
                "learning_trigger.on_terminal_results: {}",
                rule.on_terminal_results.join(", ")
            ),
            format!(
                "learning_trigger.target_stage: {}",
                rule.target_stage.as_str()
            ),
            format!(
                "learning_trigger.requested_action: {}",
                rule.requested_action.as_str()
            ),
        ]);
    }
    lines
}

fn render_concurrency_policy_lines(plan: &CompiledRunPlan) -> Vec<String> {
    let Some(policy) = &plan.concurrency_policy else {
        return vec!["concurrency_policy: none".to_owned()];
    };

    let mut lines = vec!["concurrency_policy: present".to_owned()];
    for group in &policy.mutually_exclusive_planes {
        lines.push(format!(
            "concurrency_policy.mutually_exclusive_planes: {}",
            render_plane_group(group)
        ));
    }
    for group in &policy.may_run_concurrently {
        lines.push(format!(
            "concurrency_policy.may_run_concurrently: {}",
            render_plane_group(group)
        ));
    }
    lines
}

fn render_stage_lines(stage_plan: &MaterializedGraphNodePlan) -> Vec<String> {
    vec![
        format!(
            "stage: {}.{}",
            stage_plan.plane.as_str(),
            stage_plan.node_id
        ),
        format!("stage_kind_id: {}", stage_plan.stage_kind_id),
        format!(
            "running_status_marker: {}",
            stage_plan.running_status_marker
        ),
        format!("entrypoint_path: {}", stage_plan.entrypoint_path),
        format!(
            "entrypoint_contract_id: {}",
            option_text(stage_plan.entrypoint_contract_id.as_deref())
        ),
        format!(
            "required_skills: {}",
            join_or_none(&stage_plan.required_skill_paths)
        ),
        format!(
            "attached_skills: {}",
            join_or_none(&stage_plan.attached_skill_additions)
        ),
        format!(
            "runner_name: {}",
            option_text(stage_plan.runner_name.as_deref())
        ),
        format!(
            "model_name: {}",
            option_text(stage_plan.model_name.as_deref())
        ),
        format!(
            "thinking_level: {}",
            option_text(stage_plan.thinking_level.as_deref())
        ),
        format!(
            "model_reasoning_effort: {}",
            option_text(stage_plan.model_reasoning_effort.as_deref())
        ),
        format!("timeout_seconds: {}", stage_plan.timeout_seconds),
    ]
}

fn ordered_graphs(plan: &CompiledRunPlan) -> Vec<&FrozenGraphPlanePlan> {
    let mut graphs = vec![&plan.execution_graph, &plan.planning_graph];
    if let Some(learning_graph) = &plan.learning_graph {
        graphs.push(learning_graph);
    }
    graphs
}

fn render_plane_group(group: &[Plane]) -> String {
    group
        .iter()
        .map(|plane| plane.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}

fn option_text(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn version_output() -> CliOutput {
    CliOutput::success(vec![format!("millrace {}", env!("CARGO_PKG_VERSION"))])
}

fn status_overview_output() -> CliOutput {
    let status = crate::runtime_status();

    CliOutput::success(vec![
        format!("Millrace Rust runtime {}", status.version),
        format!("package: {}", status.package_name),
        format!("crate: {}", status.crate_name),
        format!("binary: {}", status.cli_name),
        format!("status: {}", status.stability),
        "production runtime: Python package millrace-ai".to_owned(),
    ])
}
