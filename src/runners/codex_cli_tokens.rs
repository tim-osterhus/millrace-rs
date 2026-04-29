//! Codex CLI JSONL token-usage extraction.

use std::{fs, path::Path};

use serde_json::Value;

use crate::contracts::TokenUsage;

/// Extracts the largest total token usage event from a Codex JSONL event log.
#[must_use]
pub fn extract_token_usage(event_log_path: Option<&Path>) -> Option<TokenUsage> {
    let path = event_log_path?;
    let raw = fs::read_to_string(path).ok()?;
    raw.lines()
        .filter_map(token_usage_from_line)
        .max_by_key(|usage| usage.total_tokens)
}

/// Extracts token usage from one Codex JSONL line.
#[must_use]
pub fn token_usage_from_line(line: &str) -> Option<TokenUsage> {
    let stripped = line.trim();
    if !stripped.starts_with('{') {
        return None;
    }
    let payload = serde_json::from_str::<Value>(stripped).ok()?;
    token_usage_from_payload(&payload)
}

/// Extracts token usage from one decoded Codex event payload.
#[must_use]
pub fn token_usage_from_payload(payload: &Value) -> Option<TokenUsage> {
    let object = payload.as_object()?;
    let payload_type = object.get("type").and_then(Value::as_str)?;
    if payload_type == "event_msg" {
        return object.get("payload").and_then(token_usage_from_payload);
    }
    if payload_type != "token_count" {
        return None;
    }
    let info = object.get("info")?.as_object()?;
    let usage_payload = info
        .get("total_token_usage")
        .or_else(|| info.get("last_token_usage"))?
        .as_object()?;
    token_usage_from_object(usage_payload)
}

fn token_usage_from_object(payload: &serde_json::Map<String, Value>) -> Option<TokenUsage> {
    let input_tokens = integer(payload, &["input_tokens"], None)?;
    let output_tokens = integer(payload, &["output_tokens"], None)?;
    let cached_input_tokens = integer(payload, &["cached_input_tokens"], Some(0)).unwrap_or(0);
    let thinking_tokens = integer(
        payload,
        &[
            "reasoning_output_tokens",
            "thinking_tokens",
            "reasoning_tokens",
        ],
        Some(0),
    )
    .unwrap_or(0);
    let total_tokens = integer(
        payload,
        &["total_tokens"],
        Some(input_tokens + output_tokens),
    )
    .unwrap_or(input_tokens + output_tokens);
    Some(TokenUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        thinking_tokens,
        total_tokens,
    })
}

fn integer(
    payload: &serde_json::Map<String, Value>,
    keys: &[&str],
    default: Option<u64>,
) -> Option<u64> {
    for key in keys {
        let Some(value) = payload.get(*key) else {
            continue;
        };
        if let Some(value) = value.as_u64() {
            return Some(value);
        }
        if let Some(value) = value.as_i64() {
            return u64::try_from(value).ok().or(default);
        }
        if let Some(value) = value.as_f64() {
            if value.fract() == 0.0 && value >= 0.0 {
                return Some(value as u64);
            }
        }
        return default;
    }
    default
}
