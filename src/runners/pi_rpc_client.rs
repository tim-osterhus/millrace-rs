//! Pi JSONL RPC client and subprocess transport.

use std::{
    fmt, io,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::contracts::{Timestamp, TokenUsage};

use super::{RunnerEnvironmentDelta, RunnerExitKind};

/// Result returned by one Pi RPC prompt lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub struct PiRpcSessionResult {
    pub exit_kind: RunnerExitKind,
    pub exit_code: Option<i32>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    pub event_lines: Vec<String>,
    pub assistant_text: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub failure_class: Option<String>,
    pub notes: Vec<String>,
    pub stderr_text: String,
    pub observed_exit_kind: Option<RunnerExitKind>,
    pub observed_exit_code: Option<i32>,
}

/// Request used to construct a Pi RPC prompt client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiRpcClientCreateRequest {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub environment_delta: RunnerEnvironmentDelta,
}

/// Errors surfaced by the Pi RPC client before an adapter result exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiRpcClientError {
    /// The configured executable was not found.
    BinaryNotFound { binary: String },
    /// The JSONL transport failed.
    Transport { message: String },
    /// The JSONL stream contained invalid records.
    InvalidJson { message: String },
}

impl fmt::Display for PiRpcClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BinaryNotFound { binary } => write!(f, "runner binary not found: {binary}"),
            Self::Transport { message } | Self::InvalidJson { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for PiRpcClientError {}

/// Prompt-level Pi RPC client used by the adapter.
pub trait PiRpcPromptClient {
    /// Runs one prompt and returns the raw Pi RPC session evidence.
    fn run_prompt(
        &mut self,
        prompt: &str,
        timeout_seconds: u64,
    ) -> Result<PiRpcSessionResult, PiRpcClientError>;
}

/// Factory seam used by the Pi adapter for deterministic tests.
pub trait PiRpcClientFactory {
    /// Creates one prompt client for a Pi command invocation.
    fn create(
        &self,
        request: PiRpcClientCreateRequest,
    ) -> Result<Box<dyn PiRpcPromptClient>, PiRpcClientError>;
}

/// Subprocess-backed Pi RPC client factory.
#[derive(Debug, Clone, Copy)]
pub struct SubprocessPiRpcClientFactory {
    abort_grace_seconds: f64,
}

impl Default for SubprocessPiRpcClientFactory {
    fn default() -> Self {
        Self {
            abort_grace_seconds: 2.0,
        }
    }
}

impl SubprocessPiRpcClientFactory {
    /// Builds a factory with the Python-compatible abort grace period.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            abort_grace_seconds: 2.0,
        }
    }

    /// Builds a factory with a custom abort grace period.
    #[must_use]
    pub const fn with_abort_grace_seconds(abort_grace_seconds: f64) -> Self {
        Self {
            abort_grace_seconds,
        }
    }
}

impl PiRpcClientFactory for SubprocessPiRpcClientFactory {
    fn create(
        &self,
        request: PiRpcClientCreateRequest,
    ) -> Result<Box<dyn PiRpcPromptClient>, PiRpcClientError> {
        let transport = SubprocessPiRpcTransport::spawn(
            request.command,
            request.cwd,
            request.environment_delta,
        )?;
        Ok(Box::new(PiRpcJsonlClient::with_abort_grace_seconds(
            transport,
            self.abort_grace_seconds,
        )))
    }
}

/// One stdout read result from a Pi RPC transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiRpcStreamEvent {
    /// One raw JSONL stdout line, without a trailing newline.
    Line(String),
    /// The stdout stream ended.
    Eof,
    /// No stdout line arrived before the requested timeout.
    Timeout,
}

/// Transport operations needed by the JSONL lifecycle.
pub trait PiRpcTransport {
    /// Sends one JSON payload to Pi stdin.
    fn send_json(&mut self, payload: &Value) -> Result<(), PiRpcClientError>;
    /// Reads one stdout line or stream event.
    fn read_stdout_event(
        &mut self,
        timeout: Duration,
    ) -> Result<PiRpcStreamEvent, PiRpcClientError>;
    /// Returns stderr captured so far.
    fn stderr_text(&self) -> String;
    /// Returns the process exit code if it is already known.
    fn poll_exit_code(&mut self) -> Result<Option<i32>, PiRpcClientError>;
    /// Closes stdin if possible.
    fn close_stdin(&mut self);
    /// Requests graceful process termination.
    fn terminate(&mut self) -> Result<(), PiRpcClientError>;
    /// Forces process termination.
    fn kill(&mut self) -> Result<(), PiRpcClientError>;
    /// Waits for process exit until the timeout expires.
    fn wait(&mut self, timeout: Duration) -> Result<Option<i32>, PiRpcClientError>;
}

/// JSONL client that owns one prompt lifecycle over a Pi RPC transport.
pub struct PiRpcJsonlClient<T>
where
    T: PiRpcTransport,
{
    transport: T,
    abort_grace_seconds: f64,
}

impl<T> fmt::Debug for PiRpcJsonlClient<T>
where
    T: PiRpcTransport,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PiRpcJsonlClient")
            .field("abort_grace_seconds", &self.abort_grace_seconds)
            .finish_non_exhaustive()
    }
}

impl<T> PiRpcJsonlClient<T>
where
    T: PiRpcTransport,
{
    /// Builds a JSONL client with the default abort grace period.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self::with_abort_grace_seconds(transport, 2.0)
    }

    /// Builds a JSONL client with a caller-supplied abort grace period.
    #[must_use]
    pub fn with_abort_grace_seconds(transport: T, abort_grace_seconds: f64) -> Self {
        Self {
            transport,
            abort_grace_seconds,
        }
    }

    /// Returns the owned transport after test execution.
    #[must_use]
    pub fn into_transport(self) -> T {
        self.transport
    }

    fn run_prompt_inner(
        &mut self,
        prompt: &str,
        timeout_seconds: u64,
    ) -> Result<PiRpcSessionResult, PiRpcClientError> {
        let started_at = now_timestamp("started_at")?;
        let mut event_lines = Vec::new();
        let mut notes = Vec::new();
        let mut provider_error_detected = false;
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(1));

        self.send_command(json!({
            "id": "prompt-1",
            "type": "prompt",
            "message": prompt,
        }))?;
        let prompt_response =
            match self.wait_for_response("prompt-1", &mut event_lines, deadline)? {
                Some(response) => response,
                None => return self.timeout_result(started_at, event_lines),
            };
        if !response_success(&prompt_response) {
            return Ok(PiRpcSessionResult {
                exit_kind: RunnerExitKind::RunnerError,
                exit_code: Some(1),
                started_at,
                ended_at: now_timestamp("ended_at")?,
                event_lines,
                assistant_text: None,
                token_usage: None,
                failure_class: Some("runner_rpc_rejected".to_owned()),
                notes: vec![
                    response_error(&prompt_response)
                        .unwrap_or("prompt rejected")
                        .to_owned(),
                ],
                stderr_text: self.transport.stderr_text(),
                observed_exit_kind: None,
                observed_exit_code: None,
            });
        }

        loop {
            match self.read_record(deadline)? {
                PiRpcRecord::Timeout => return self.timeout_result(started_at, event_lines),
                PiRpcRecord::Eof => {
                    return Ok(PiRpcSessionResult {
                        exit_kind: RunnerExitKind::RunnerError,
                        exit_code: Some(1),
                        started_at,
                        ended_at: now_timestamp("ended_at")?,
                        event_lines,
                        assistant_text: None,
                        token_usage: None,
                        failure_class: Some("runner_incomplete_rpc_stream".to_owned()),
                        notes: vec!["pi rpc stream ended before agent_end".to_owned()],
                        stderr_text: self.transport.stderr_text(),
                        observed_exit_kind: None,
                        observed_exit_code: None,
                    });
                }
                PiRpcRecord::Payload { raw_line, payload } => {
                    if payload.get("type").and_then(Value::as_str) == Some("response") {
                        continue;
                    }
                    if event_stop_reason(&payload).as_deref() == Some("error") {
                        provider_error_detected = true;
                    }
                    event_lines.push(raw_line);
                    if payload.get("type").and_then(Value::as_str) == Some("agent_end") {
                        break;
                    }
                }
            }
        }

        let assistant_text = self.request_last_assistant_text(&mut notes)?;
        let token_usage = self.request_session_stats(&mut notes)?;
        let mut exit_kind = RunnerExitKind::Completed;
        let mut exit_code = Some(0);
        let mut failure_class = None;

        if provider_error_detected && !has_nonempty_text(assistant_text.as_deref()) {
            exit_kind = RunnerExitKind::ProviderError;
            exit_code = Some(1);
            failure_class = Some("runner_provider_failure".to_owned());
        } else if !has_nonempty_text(assistant_text.as_deref()) {
            exit_kind = RunnerExitKind::RunnerError;
            exit_code = Some(1);
            failure_class = Some("runner_empty_assistant_text".to_owned());
        }

        Ok(PiRpcSessionResult {
            exit_kind,
            exit_code,
            started_at,
            ended_at: now_timestamp("ended_at")?,
            event_lines,
            assistant_text,
            token_usage,
            failure_class,
            notes,
            stderr_text: self.transport.stderr_text(),
            observed_exit_kind: None,
            observed_exit_code: None,
        })
    }

    fn send_command(&mut self, payload: Value) -> Result<(), PiRpcClientError> {
        self.transport.send_json(&payload)
    }

    fn wait_for_response(
        &mut self,
        response_id: &str,
        event_lines: &mut Vec<String>,
        deadline: Instant,
    ) -> Result<Option<Map<String, Value>>, PiRpcClientError> {
        loop {
            match self.read_record(deadline)? {
                PiRpcRecord::Timeout => return Ok(None),
                PiRpcRecord::Eof => {
                    return Err(PiRpcClientError::Transport {
                        message: "pi rpc stream ended before response".to_owned(),
                    });
                }
                PiRpcRecord::Payload { raw_line, payload } => {
                    if payload.get("type").and_then(Value::as_str) != Some("response") {
                        event_lines.push(raw_line);
                        continue;
                    }
                    if payload.get("id").and_then(Value::as_str) == Some(response_id) {
                        return payload.as_object().cloned().map(Some).ok_or_else(|| {
                            PiRpcClientError::InvalidJson {
                                message: "pi rpc response must be a JSON object".to_owned(),
                            }
                        });
                    }
                }
            }
        }
    }

    fn request_last_assistant_text(
        &mut self,
        notes: &mut Vec<String>,
    ) -> Result<Option<String>, PiRpcClientError> {
        let response =
            self.request_response("last-assistant-1", "get_last_assistant_text", notes)?;
        let Some(response) = response.filter(response_success) else {
            notes.push("pi rpc get_last_assistant_text failed".to_owned());
            return Ok(None);
        };
        let text = response
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("text"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(text)
    }

    fn request_session_stats(
        &mut self,
        notes: &mut Vec<String>,
    ) -> Result<Option<TokenUsage>, PiRpcClientError> {
        let response = self.request_response("session-stats-1", "get_session_stats", notes)?;
        let Some(response) = response.filter(response_success) else {
            notes.push("pi rpc get_session_stats unavailable".to_owned());
            return Ok(None);
        };
        Ok(token_usage_from_stats_payload(response.get("data")))
    }

    fn request_response(
        &mut self,
        response_id: &str,
        command_type: &str,
        _notes: &mut Vec<String>,
    ) -> Result<Option<Map<String, Value>>, PiRpcClientError> {
        if self
            .send_command(json!({
                "id": response_id,
                "type": command_type,
            }))
            .is_err()
        {
            return Ok(None);
        }
        let mut ignored_events = Vec::new();
        self.wait_for_response(
            response_id,
            &mut ignored_events,
            Instant::now() + Duration::from_secs(5),
        )
    }

    fn timeout_result(
        &mut self,
        started_at: Timestamp,
        mut event_lines: Vec<String>,
    ) -> Result<PiRpcSessionResult, PiRpcClientError> {
        let mut notes = vec!["runner process exceeded timeout".to_owned()];
        match self.send_command(json!({"id": "abort-1", "type": "abort"})) {
            Ok(()) => notes.push("sent pi rpc abort command".to_owned()),
            Err(_) => notes.push("failed to send pi rpc abort command".to_owned()),
        }

        let grace_deadline =
            Instant::now() + Duration::from_secs_f64(self.abort_grace_seconds.max(0.0));
        while Instant::now() < grace_deadline {
            match self.read_record(grace_deadline)? {
                PiRpcRecord::Payload { raw_line, payload } => {
                    if payload.get("type").and_then(Value::as_str) != Some("response") {
                        event_lines.push(raw_line);
                    }
                    if self.transport.poll_exit_code()?.is_some() {
                        break;
                    }
                }
                PiRpcRecord::Eof | PiRpcRecord::Timeout => break,
            }
        }

        if self.transport.poll_exit_code()?.is_none() {
            notes.push("sent pi rpc process terminate after abort grace period".to_owned());
            self.transport.terminate()?;
            if self.transport.wait(Duration::from_secs(1))?.is_none() {
                notes.push(
                    "pi rpc process required hard kill after terminate grace period".to_owned(),
                );
                self.transport.kill()?;
                let _ = self.transport.wait(Duration::from_secs(1))?;
            }
        }

        Ok(PiRpcSessionResult {
            exit_kind: RunnerExitKind::Timeout,
            exit_code: Some(124),
            started_at,
            ended_at: now_timestamp("ended_at")?,
            event_lines,
            assistant_text: None,
            token_usage: None,
            failure_class: Some("runner_timeout".to_owned()),
            notes,
            stderr_text: self.transport.stderr_text(),
            observed_exit_kind: None,
            observed_exit_code: None,
        })
    }

    fn read_record(&mut self, deadline: Instant) -> Result<PiRpcRecord, PiRpcClientError> {
        let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(PiRpcRecord::Timeout);
        };
        match self.transport.read_stdout_event(timeout)? {
            PiRpcStreamEvent::Timeout => Ok(PiRpcRecord::Timeout),
            PiRpcStreamEvent::Eof => Ok(PiRpcRecord::Eof),
            PiRpcStreamEvent::Line(raw_line) => {
                let payload = serde_json::from_str::<Value>(&raw_line).map_err(|error| {
                    PiRpcClientError::InvalidJson {
                        message: format!("invalid JSON in pi rpc stream: {error}"),
                    }
                })?;
                if !payload.is_object() {
                    return Err(PiRpcClientError::InvalidJson {
                        message: "pi rpc stream record must be a JSON object".to_owned(),
                    });
                }
                Ok(PiRpcRecord::Payload { raw_line, payload })
            }
        }
    }

    fn shutdown_process(&mut self) {
        self.transport.close_stdin();
        if self
            .transport
            .wait(Duration::from_secs(1))
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let _ = self.transport.terminate();
        if self
            .transport
            .wait(Duration::from_secs(1))
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let _ = self.transport.kill();
        let _ = self.transport.wait(Duration::from_secs(1));
    }
}

impl<T> PiRpcPromptClient for PiRpcJsonlClient<T>
where
    T: PiRpcTransport,
{
    fn run_prompt(
        &mut self,
        prompt: &str,
        timeout_seconds: u64,
    ) -> Result<PiRpcSessionResult, PiRpcClientError> {
        let result = self.run_prompt_inner(prompt, timeout_seconds);
        self.shutdown_process();
        result
    }
}

#[derive(Debug)]
enum PiRpcRecord {
    Payload { raw_line: String, payload: Value },
    Eof,
    Timeout,
}

/// Real subprocess transport for Pi RPC.
pub struct SubprocessPiRpcTransport {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_rx: mpsc::Receiver<PiRpcStreamEvent>,
    stderr_text: Arc<Mutex<String>>,
    stdout_thread: Option<thread::JoinHandle<()>>,
    stderr_thread: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for SubprocessPiRpcTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubprocessPiRpcTransport")
            .field("process_id", &self.child.id())
            .finish_non_exhaustive()
    }
}

impl SubprocessPiRpcTransport {
    /// Spawns a Pi RPC subprocess with piped stdin/stdout/stderr.
    pub fn spawn(
        command: Vec<String>,
        cwd: PathBuf,
        environment_delta: RunnerEnvironmentDelta,
    ) -> Result<Self, PiRpcClientError> {
        let Some((binary, args)) = command.split_first() else {
            return Err(PiRpcClientError::Transport {
                message: "pi rpc command cannot be empty".to_owned(),
            });
        };
        let mut process = Command::new(binary);
        process
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in environment_delta.set {
            process.env(key, value);
        }
        for key in environment_delta.unset {
            process.env_remove(key);
        }

        let mut child = process.spawn().map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                PiRpcClientError::BinaryNotFound {
                    binary: binary.clone(),
                }
            } else {
                PiRpcClientError::Transport {
                    message: error.to_string(),
                }
            }
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PiRpcClientError::Transport {
                message: "pi rpc stdin is unavailable".to_owned(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PiRpcClientError::Transport {
                message: "pi rpc stdout is unavailable".to_owned(),
            })?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PiRpcClientError::Transport {
                message: "pi rpc stderr is unavailable".to_owned(),
            })?;

        let (stdout_tx, stdout_rx) = mpsc::channel();
        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.split(b'\n') {
                let Ok(mut bytes) = line else {
                    break;
                };
                if bytes.ends_with(b"\r") {
                    bytes.pop();
                }
                let text = String::from_utf8_lossy(&bytes).to_string();
                if stdout_tx.send(PiRpcStreamEvent::Line(text)).is_err() {
                    return;
                }
            }
            let _ = stdout_tx.send(PiRpcStreamEvent::Eof);
        });

        let stderr_text = Arc::new(Mutex::new(String::new()));
        let stderr_text_for_thread = Arc::clone(&stderr_text);
        let stderr_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let text = String::from_utf8_lossy(&buffer[..count]);
                        if let Ok(mut chunks) = stderr_text_for_thread.lock() {
                            chunks.push_str(&text);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout_rx,
            stderr_text,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
        })
    }
}

impl PiRpcTransport for SubprocessPiRpcTransport {
    fn send_json(&mut self, payload: &Value) -> Result<(), PiRpcClientError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| PiRpcClientError::Transport {
                message: "pi rpc stdin is unavailable".to_owned(),
            })?;
        let mut line =
            serde_json::to_vec(payload).map_err(|error| PiRpcClientError::Transport {
                message: error.to_string(),
            })?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| PiRpcClientError::Transport {
                message: error.to_string(),
            })
    }

    fn read_stdout_event(
        &mut self,
        timeout: Duration,
    ) -> Result<PiRpcStreamEvent, PiRpcClientError> {
        match self.stdout_rx.recv_timeout(timeout) {
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(PiRpcStreamEvent::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(PiRpcStreamEvent::Eof),
        }
    }

    fn stderr_text(&self) -> String {
        self.stderr_text
            .lock()
            .map(|chunks| chunks.clone())
            .unwrap_or_default()
    }

    fn poll_exit_code(&mut self) -> Result<Option<i32>, PiRpcClientError> {
        self.child
            .try_wait()
            .map(|status| status.map(|status| status.code().unwrap_or(-1)))
            .map_err(|error| PiRpcClientError::Transport {
                message: error.to_string(),
            })
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn terminate(&mut self) -> Result<(), PiRpcClientError> {
        terminate_child(&mut self.child)
    }

    fn kill(&mut self) -> Result<(), PiRpcClientError> {
        self.child
            .kill()
            .map_err(|error| PiRpcClientError::Transport {
                message: error.to_string(),
            })
    }

    fn wait(&mut self, timeout: Duration) -> Result<Option<i32>, PiRpcClientError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(code) = self.poll_exit_code()? {
                return Ok(Some(code));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for SubprocessPiRpcTransport {
    fn drop(&mut self) {
        self.close_stdin();
        let _ = self.wait(Duration::from_millis(100));
        if self.poll_exit_code().ok().flatten().is_none() {
            let _ = self.kill();
            let _ = self.wait(Duration::from_millis(100));
        }
        if let Some(handle) = self.stdout_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> Result<(), PiRpcClientError> {
    let pid = child.id() as libc::pid_t;
    // SAFETY: `pid` comes from a live `std::process::Child`; sending SIGTERM is
    // the platform termination request before the hard-kill fallback.
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(PiRpcClientError::Transport {
            message: io::Error::last_os_error().to_string(),
        })
    }
}

#[cfg(not(unix))]
fn terminate_child(child: &mut Child) -> Result<(), PiRpcClientError> {
    child.kill().map_err(|error| PiRpcClientError::Transport {
        message: error.to_string(),
    })
}

fn response_success(response: &Map<String, Value>) -> bool {
    response
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn response_error(response: &Map<String, Value>) -> Option<&str> {
    response.get("error").and_then(Value::as_str)
}

fn event_stop_reason(payload: &Value) -> Option<String> {
    if let Some(stop_reason) = payload.get("stopReason").and_then(Value::as_str) {
        return Some(stop_reason.to_owned());
    }
    if let Some(stop_reason) = payload
        .get("message")
        .and_then(Value::as_object)
        .and_then(|message| message.get("stopReason"))
        .and_then(Value::as_str)
    {
        return Some(stop_reason.to_owned());
    }
    payload
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().rev().find_map(|item| {
                let item = item.as_object()?;
                (item.get("role").and_then(Value::as_str) == Some("assistant"))
                    .then(|| item.get("stopReason").and_then(Value::as_str))
                    .flatten()
            })
        })
        .map(str::to_owned)
}

fn has_nonempty_text(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty())
}

/// Extracts Millrace token usage from a Pi session stats payload.
#[must_use]
pub fn token_usage_from_stats_payload(payload: Option<&Value>) -> Option<TokenUsage> {
    let tokens = payload?
        .as_object()?
        .get("tokens")
        .and_then(Value::as_object)?;
    let input_tokens = coerce_non_negative_int(tokens.get("input"));
    let output_tokens = coerce_non_negative_int(tokens.get("output"));
    let cached_input_tokens = coerce_non_negative_int(tokens.get("cacheRead"));
    let mut total_tokens = coerce_non_negative_int(tokens.get("total"));
    if total_tokens == 0 {
        total_tokens = input_tokens + output_tokens;
    }
    Some(TokenUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        thinking_tokens: 0,
        total_tokens,
    })
}

fn coerce_non_negative_int(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => {
            if let Some(value) = number.as_u64() {
                value
            } else if let Some(value) = number.as_i64() {
                value.max(0) as u64
            } else {
                number.as_f64().map_or(0, |value| value.max(0.0) as u64)
            }
        }
        _ => 0,
    }
}

fn now_timestamp(field_name: &'static str) -> Result<Timestamp, PiRpcClientError> {
    let rendered = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| PiRpcClientError::Transport {
            message: format!("{field_name} could not be formatted: {error}"),
        })?;
    Timestamp::parse(field_name, &rendered).map_err(|error| PiRpcClientError::Transport {
        message: error.to_string(),
    })
}

impl From<PiRpcClientError> for super::RunnerError {
    fn from(error: PiRpcClientError) -> Self {
        match error {
            PiRpcClientError::BinaryNotFound { binary } => Self::RunnerBinaryNotFound { binary },
            PiRpcClientError::Transport { message } | PiRpcClientError::InvalidJson { message } => {
                Self::RunnerTransport { message }
            }
        }
    }
}
