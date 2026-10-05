//! `cc-hooks herdr-session` (SessionStart): reports the session to the herdr pane it runs in,
//! exactly as the herdr-installed PowerShell integration did. Never prints anything.

use crate::hook_io::{Outcome, str_at};
use crate::proc::ProcessRunner;
use serde_json::Value;

/// The environment the herdr integration gates on.
#[derive(Debug, Default, Clone)]
pub struct HerdrEnv {
    /// `HERDR_ENV == "1"`.
    pub inside_herdr: bool,
    pub pane_id: Option<String>,
    /// `CURSOR_VERSION` is set (the integration stays out of Cursor).
    pub in_cursor: bool,
    pub bin_path: Option<String>,
    pub now_millis: u128,
}

impl HerdrEnv {
    /// Reads the gates from the process environment.
    ///
    /// Example: `HerdrEnv::from_process().inside_herdr`
    pub fn from_process() -> Self {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        HerdrEnv {
            inside_herdr: std::env::var("HERDR_ENV").is_ok_and(|v| v == "1"),
            pane_id: std::env::var("HERDR_PANE_ID").ok(),
            in_cursor: std::env::var_os("CURSOR_VERSION").is_some(),
            bin_path: std::env::var("HERDR_BIN_PATH").ok(),
            now_millis: now.map_or(0, |d| d.as_millis()),
        }
    }
}

/// Runs `herdr pane report-agent-session ...` when every gate passes.
///
/// Example: `run(&json!({"hook_event_name":"SessionStart","session_id":"s"}), &env, &SystemProcessRunner)`
pub fn run(payload: &Value, env: &HerdrEnv, runner: &dyn ProcessRunner) -> Outcome {
    if let Some(args) = report_args(payload, env) {
        let program = non_blank(env.bin_path.as_deref()).unwrap_or("herdr");
        runner.run_quiet(program, &args);
    }
    Outcome::Silent
}

/// The herdr arguments, or `None` when a gate says this session is not herdr's business.
///
/// Example: `report_args(&payload, &env)` -> `Some(["pane", "report-agent-session", "<pane>", ...])`
pub fn report_args(payload: &Value, env: &HerdrEnv) -> Option<Vec<String>> {
    let pane = non_blank(env.pane_id.as_deref()).filter(|_| env.inside_herdr)?;
    if env.in_cursor || payload.get("cursor_version").is_some() || !is_main_session_start(payload) {
        return None;
    }
    let session = session_id(payload)?;
    let mut args: Vec<String> = [
        "pane",
        "report-agent-session",
        pane,
        "--source",
        "herdr:claude",
    ]
    .into_iter()
    .chain(["--agent", "claude", "--seq"])
    .map(str::to_string)
    .collect();
    args.extend([
        env.now_millis.to_string(),
        "--agent-session-id".into(),
        session,
    ]);
    push_optional(
        &mut args,
        "--agent-session-path",
        payload,
        "transcript_path",
    );
    push_optional(&mut args, "--session-start-source", payload, "source");
    Some(args)
}

fn is_main_session_start(payload: &Value) -> bool {
    let is_start = str_at(payload, &["hook_event_name"]) == Some("SessionStart");
    let agent_id = payload.get("agent_id").map(scalar_text).unwrap_or_default();
    is_start && agent_id.trim().is_empty()
}

fn session_id(payload: &Value) -> Option<String> {
    let id = scalar_text(payload.get("session_id")?);
    (!id.trim().is_empty()).then_some(id)
}

fn push_optional(args: &mut Vec<String>, flag: &str, payload: &Value, key: &str) {
    if let Some(value) = non_blank(str_at(payload, &[key])) {
        args.extend([flag.to_string(), value.to_string()]);
    }
}

/// PowerShell's string view of a JSON scalar ("" for null, objects and arrays).
///
/// Example: `scalar_text(&json!(42))` -> `"42"`
pub fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        _ => String::new(),
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fakes::RecordingProcessRunner;
    use serde_json::json;

    fn herdr_env() -> HerdrEnv {
        HerdrEnv {
            inside_herdr: true,
            pane_id: Some("p1".into()),
            now_millis: 99,
            ..Default::default()
        }
    }

    fn start_payload() -> Value {
        json!({"hook_event_name": "SessionStart", "session_id": "s1", "transcript_path": "t.jsonl", "source": "startup"})
    }

    #[test]
    fn report_args_builds_full_invocation() {
        let args = report_args(&start_payload(), &herdr_env()).unwrap();
        let expected = "pane report-agent-session p1 --source herdr:claude --agent claude --seq 99 --agent-session-id s1 --agent-session-path t.jsonl --session-start-source startup";
        assert_eq!(args.join(" "), expected);
    }

    #[test]
    fn report_args_respects_gates() {
        let outside = HerdrEnv {
            inside_herdr: false,
            ..herdr_env()
        };
        assert_eq!(report_args(&start_payload(), &outside), None);
        let cursor = HerdrEnv {
            in_cursor: true,
            ..herdr_env()
        };
        assert_eq!(report_args(&start_payload(), &cursor), None);
        let subagent =
            json!({"hook_event_name": "SessionStart", "session_id": "s", "agent_id": "a"});
        assert_eq!(report_args(&subagent, &herdr_env()), None);
        assert_eq!(
            report_args(
                &json!({"hook_event_name": "sessionstart", "session_id": "s"}),
                &herdr_env()
            ),
            None
        );
    }

    #[test]
    fn run_spawns_custom_binary_and_stays_silent() {
        let runner = RecordingProcessRunner::default();
        let env = HerdrEnv {
            bin_path: Some("C:/h/herdr.exe".into()),
            ..herdr_env()
        };
        assert_eq!(run(&start_payload(), &env, &runner), Outcome::Silent);
        assert_eq!(runner.calls.borrow()[0].0, "C:/h/herdr.exe");
    }

    #[test]
    fn run_does_nothing_without_session_id() {
        let runner = RecordingProcessRunner::default();
        run(
            &json!({"hook_event_name": "SessionStart", "session_id": " "}),
            &herdr_env(),
            &runner,
        );
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn scalar_text_mirrors_powershell_strings() {
        assert_eq!(scalar_text(&json!(42)), "42");
        assert_eq!(scalar_text(&json!(true)), "True");
        assert_eq!(scalar_text(&json!(null)), "");
    }

    #[test]
    fn from_process_reads_without_panicking() {
        let _ = HerdrEnv::from_process();
    }
}
