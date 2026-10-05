//! Reads the latest human entry of a Claude Code transcript (JSONL) from its tail.
//!
//! The image-tools hook runs on EVERY tool call, so this must stay cheap: it reads a small window
//! from the end and only grows it (up to the same 8 MB cap the Python hook used) when the latest
//! human entry is further back.

use crate::fs_port::FileSystem;
use serde_json::Value;
use std::path::Path;

/// Only the last 8 MB of a transcript are considered, like the original hook.
pub const TAIL_LIMIT: u64 = 8 * 1024 * 1024;
const FIRST_WINDOW: u64 = 256 * 1024;

/// The newest user-typed entry: not a tool result, not a meta entry.
///
/// Example: `latest_human_entry(&DiskFileSystem, Path::new("t.jsonl"))` -> `Some(json!({"type":"user",..}))`
pub fn latest_human_entry(fs: &dyn FileSystem, path: &Path) -> Option<Value> {
    let size = fs.file_size(path)?;
    let floor = size.saturating_sub(TAIL_LIMIT);
    let mut window = FIRST_WINDOW;
    loop {
        let start = size.saturating_sub(window).max(floor);
        let bytes = fs.read_range(path, start, (size - start) as usize)?;
        // Below the floor the first line is cut mid-way; the next, larger window reads it whole.
        let skip_first = usize::from(start > floor);
        if let Some(entry) = newest_human_in(&decode_ignoring_errors(&bytes), skip_first) {
            return Some(entry);
        }
        if start == floor {
            return None;
        }
        window *= 4;
    }
}

fn newest_human_in(text: &str, skip_first: usize) -> Option<Value> {
    let lines: Vec<&str> = split_lines(text).collect();
    let complete = lines.get(skip_first..).unwrap_or(&[]);
    complete.iter().rev().find_map(|line| human_entry(line))
}

/// Parses one JSONL line when it is a human entry.
///
/// Example: `human_entry(r#"{"type":"user","message":{"content":"hi"}}"#)` -> `Some(..)`
pub fn human_entry(line: &str) -> Option<Value> {
    if !line.contains("\"type\":\"user\"") {
        return None;
    }
    // Cheap skip before parsing: tool results can be megabytes, and an unescaped
    // "type":"tool_result" can only be JSON structure, never text inside a string.
    if line.contains("\"type\":\"tool_result\"") {
        return None;
    }
    let entry: Value = serde_json::from_str(line).ok()?;
    if !entry.is_object() || has_tool_result(&entry) || is_truthy(entry.get("isMeta")) {
        return None;
    }
    Some(entry)
}

fn has_tool_result(entry: &Value) -> bool {
    content_items(entry).any(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
}

/// The dict items of `message.content` when it is a list.
///
/// Example: `content_items(&json!({"message":{"content":[{"type":"text"}]}})).count()` -> `1`
pub fn content_items(entry: &Value) -> impl Iterator<Item = &Value> {
    let items = entry.pointer("/message/content").and_then(Value::as_array);
    items.into_iter().flatten().filter(|item| item.is_object())
}

/// Python truthiness for a JSON value (`None`, false, 0, "", [] and {} are false).
///
/// Example: `is_truthy(Some(&json!("x")))` -> `true`
pub fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// UTF-8 decode that drops invalid bytes, like Python's `errors="ignore"`.
///
/// Example: `decode_ignoring_errors(b"a\xFFb")` -> `"ab"`
pub fn decode_ignoring_errors(mut bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    loop {
        match std::str::from_utf8(bytes) {
            Ok(valid) => return out + valid,
            Err(error) => {
                let (good, rest) = bytes.split_at(error.valid_up_to());
                out.push_str(std::str::from_utf8(good).unwrap_or_default());
                bytes = &rest[error.error_len().unwrap_or(rest.len()).min(rest.len())..];
            }
        }
    }
}

/// Splits on every boundary Python's `str.splitlines()` knows.
///
/// Example: `split_lines("a\nb\u{2028}c").count()` -> `3`
pub fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    const BREAKS: [char; 10] = [
        '\n', '\r', '\u{0B}', '\u{0C}', '\u{1C}', '\u{1D}', '\u{1E}', '\u{85}', '\u{2028}',
        '\u{2029}',
    ];
    text.split(BREAKS).filter(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_port::fakes::FakeFileSystem;
    use serde_json::json;

    const HUMAN: &str = r#"{"type":"user","uuid":"u1","message":{"content":"hello"}}"#;
    const RESULT: &str =
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#;
    const META: &str = r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#;

    #[test]
    fn latest_human_entry_skips_results_and_meta() {
        let body = format!("{HUMAN}\n{RESULT}\n{META}\n{{\"type\":\"assistant\"}}\n");
        let fs = FakeFileSystem::default().with_file("t.jsonl", body.as_bytes());
        let entry = latest_human_entry(&fs, Path::new("t.jsonl")).unwrap();
        assert_eq!(entry["uuid"], "u1");
        assert_eq!(latest_human_entry(&fs, Path::new("missing.jsonl")), None);
    }

    #[test]
    fn latest_human_entry_grows_window_past_big_tail() {
        let filler = format!(
            "{{\"type\":\"assistant\",\"pad\":\"{}\"}}\n",
            "x".repeat(300 * 1024)
        );
        let body = format!("{HUMAN}\n{filler}{filler}");
        let fs = FakeFileSystem::default().with_file("t.jsonl", body.as_bytes());
        assert_eq!(
            latest_human_entry(&fs, Path::new("t.jsonl")).unwrap()["uuid"],
            "u1"
        );
    }

    #[test]
    fn latest_human_entry_ignores_entries_beyond_tail_limit() {
        let filler = format!(
            "{{\"type\":\"assistant\",\"pad\":\"{}\"}}\n",
            "x".repeat(TAIL_LIMIT as usize)
        );
        let body = format!("{HUMAN}\n{filler}");
        let fs = FakeFileSystem::default().with_file("t.jsonl", body.as_bytes());
        assert_eq!(latest_human_entry(&fs, Path::new("t.jsonl")), None);
    }

    #[test]
    fn human_entry_filters_by_shape() {
        assert!(human_entry(HUMAN).is_some());
        assert!(human_entry(RESULT).is_none());
        assert!(human_entry(META).is_none());
        assert!(human_entry(r#"{"type":"user" broken"#).is_none());
    }

    #[test]
    fn content_items_yields_only_objects() {
        let entry = json!({"message": {"content": [{"type": "text"}, "raw", 3]}});
        assert_eq!(content_items(&entry).count(), 1);
        assert_eq!(
            content_items(&json!({"message": {"content": "s"}})).count(),
            0
        );
    }

    #[test]
    fn is_truthy_matches_python() {
        assert!(!is_truthy(None));
        assert!(!is_truthy(Some(&json!(0))));
        assert!(!is_truthy(Some(&json!(""))));
        assert!(is_truthy(Some(&json!([1]))));
        assert!(is_truthy(Some(&json!(true))));
    }

    #[test]
    fn decode_ignoring_errors_drops_invalid_bytes() {
        assert_eq!(decode_ignoring_errors(b"a\xFFb\xE2\x82"), "ab");
        assert_eq!(decode_ignoring_errors("é".as_bytes()), "é");
    }

    #[test]
    fn split_lines_handles_python_breaks() {
        let lines: Vec<&str> = split_lines("a\r\nb\u{2028}c\n").collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }
}
