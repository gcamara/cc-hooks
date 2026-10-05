//! Shared hook plumbing: decode the stdin payload, build the hook answer JSON, write it out.
//!
//! Every subcommand goes through here so the "never block on a defect" contract lives in one place:
//! anything that cannot be decoded becomes `Outcome::Silent` (exit 0, no output).

use serde_json::{Map, Value, json};
use std::io::Write;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// What a hook decided. `Relay` passes a child script's answer through untouched.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Emit(Value),
    Relay { stdout: Vec<u8>, code: i32 },
}

/// Parses the hook payload, tolerating a leading UTF-8 BOM.
///
/// Example: `parse_payload(b"\xEF\xBB\xBF{\"prompt\":\"x\"}")` -> `Some(json!({"prompt":"x"}))`
pub fn parse_payload(raw: &[u8]) -> Option<Value> {
    let body = raw.strip_prefix(UTF8_BOM).unwrap_or(raw);
    let text = std::str::from_utf8(body).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// Reads a string at a nested key path, `None` when any step is missing or not a string.
///
/// Example: `str_at(&json!({"tool_input":{"command":"ls"}}), &["tool_input", "command"])` -> `Some("ls")`
pub fn str_at<'a>(payload: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut node = payload;
    for key in path {
        node = node.get(key)?;
    }
    node.as_str()
}

/// PreToolUse deny answer with the given reason.
///
/// Example: `deny_answer("no")["hookSpecificOutput"]["permissionDecision"] == "deny"`
pub fn deny_answer(reason: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
}

/// Answer that injects `context` for the given hook event, optionally with a user-visible message.
///
/// Example: `context_answer("SessionStart", "ctx", None)["hookSpecificOutput"]["additionalContext"] == "ctx"`
pub fn context_answer(event: &str, context: &str, system_message: Option<&str>) -> Value {
    let mut root = Map::new();
    if let Some(message) = system_message {
        root.insert("systemMessage".into(), Value::from(message));
    }
    let specific = json!({ "hookEventName": event, "additionalContext": context });
    root.insert("hookSpecificOutput".into(), specific);
    Value::Object(root)
}

/// Writes the outcome to `out` and returns the process exit code.
///
/// Example: `write_outcome(&Outcome::Silent, &mut Vec::new())` -> `0`
pub fn write_outcome(outcome: &Outcome, out: &mut dyn Write) -> i32 {
    match outcome {
        Outcome::Silent => 0,
        Outcome::Emit(value) => {
            let _ = writeln!(out, "{value}");
            0
        }
        Outcome::Relay { stdout, code } => {
            let _ = out.write_all(stdout);
            *code
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_payload_strips_bom_and_rejects_garbage() {
        assert_eq!(
            parse_payload(b"\xEF\xBB\xBF{\"a\":1}"),
            Some(json!({"a": 1}))
        );
        assert_eq!(parse_payload(b"  "), None);
        assert_eq!(parse_payload(b"{nope"), None);
        assert_eq!(parse_payload(b"\xFF\xFE"), None);
    }

    #[test]
    fn str_at_walks_nested_keys() {
        let payload = json!({"tool_input": {"command": "ls", "n": 3}});
        assert_eq!(str_at(&payload, &["tool_input", "command"]), Some("ls"));
        assert_eq!(str_at(&payload, &["tool_input", "n"]), None);
        assert_eq!(str_at(&payload, &["missing"]), None);
    }

    #[test]
    fn deny_answer_has_pretooluse_shape() {
        let answer = deny_answer("why");
        assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(
            answer["hookSpecificOutput"]["permissionDecisionReason"],
            "why"
        );
    }

    #[test]
    fn context_answer_adds_optional_system_message() {
        let plain = context_answer("SessionStart", "ctx", None);
        assert!(plain.get("systemMessage").is_none());
        let loud = context_answer("UserPromptSubmit", "ctx", Some("hi"));
        assert_eq!(loud["systemMessage"], "hi");
        assert_eq!(loud["hookSpecificOutput"]["additionalContext"], "ctx");
    }

    #[test]
    fn write_outcome_writes_one_line_or_relays() {
        let mut sink = Vec::new();
        assert_eq!(write_outcome(&Outcome::Emit(json!({"a": 1})), &mut sink), 0);
        assert_eq!(sink, b"{\"a\":1}\n");
        let mut relayed = Vec::new();
        let relay = Outcome::Relay {
            stdout: b"x".to_vec(),
            code: 3,
        };
        assert_eq!(write_outcome(&relay, &mut relayed), 3);
        assert_eq!(relayed, b"x");
    }
}
