//! Suites that compare side effects instead of stdout: herdr (the arguments it is called with) and
//! the quality-gate fast path (PR detection, plus relaying the gate script's answer).

use super::cases::local_config;
use super::{Case, Lab, Row, UTF8_CONSOLE, parse_answer, powershell_args, run_new, spawn, tally};
use serde_json::{Value, json};

const RECORDER: &str = "@echo off\r\necho %*> \"%HERDR_RECORD%\"\r\n";
const FAKE_GATE: &str =
    "$raw = [Console]::In.ReadToEnd()\n'{\"gated\":true,\"bytes\":' + $raw.Length + '}'\nexit 0\n";

fn session_start(extra: Value) -> String {
    let mut payload = json!({"hook_event_name": "SessionStart", "session_id": "s1"});
    for (key, value) in extra.as_object().expect("object").iter() {
        payload[key] = value.clone();
    }
    payload.to_string()
}

fn herdr_cases() -> Vec<Case> {
    vec![
        Case::new(
            "startup-full",
            &session_start(json!({"transcript_path": "C:/t/x.jsonl", "source": "startup"})),
        ),
        Case::new("no-transcript", &session_start(json!({"source": "resume"}))),
        Case::new(
            "no-source",
            &session_start(json!({"transcript_path": "C:/t/x.jsonl"})),
        ),
        Case::new("subagent", &session_start(json!({"agent_id": "a1"}))),
        Case::new(
            "wrong-case-event",
            &session_start(json!({"hook_event_name": "sessionstart"})),
        ),
        Case::new("cursor", &session_start(json!({"cursor_version": "1.0"}))),
        Case::new("blank-session", &session_start(json!({"session_id": "  "}))),
        Case::new("numeric-session", &session_start(json!({"session_id": 42}))),
        Case::with_bom("bom-startup", &session_start(json!({"source": "startup"}))),
    ]
}

fn herdr_env(lab: &Lab, recorder: &str, record: &str) -> Vec<(String, String)> {
    let mut env = lab.base_env();
    env.extend(
        [
            ("HERDR_ENV", "1"),
            ("HERDR_PANE_ID", "p1"),
            ("HERDR_BIN_PATH", recorder),
            ("HERDR_RECORD", record),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string())),
    );
    env
}

/// What herdr was called with, with the time-based `--seq` value blanked.
fn recorded_call(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    let seq = regex::Regex::new(r"--seq \d+").expect("seq pattern");
    Some(seq.replace(text.trim(), "--seq N").into_owned())
}

pub fn run_herdr_suite(lab: &Lab) -> Row {
    let recorder = lab.write("herdr-recorder.cmd", RECORDER.as_bytes());
    let script = lab
        .hooks_dir
        .join("herdr-agent-state.ps1")
        .to_string_lossy()
        .into_owned();
    let cases = herdr_cases();
    let mut row = Row {
        hook: "herdr-session".into(),
        cases: cases.len(),
        ..Row::default()
    };
    for case in &cases {
        let (old_rec, new_rec) = (lab.write("old.rec", b""), lab.write("new.rec", b""));
        let _ = (
            std::fs::remove_file(&old_rec),
            std::fs::remove_file(&new_rec),
        );
        let old_out = spawn(
            "powershell",
            &powershell_args(&script, &["session"], true),
            &case.input,
            &herdr_env(lab, &recorder, &old_rec),
        );
        let new_out = run_new(
            lab,
            "herdr-session",
            &case.input,
            &herdr_env(lab, &recorder, &new_rec),
        );
        let (old, new) = (recorded_call(&old_rec), recorded_call(&new_rec));
        let quiet = old_out.trim().is_empty() && new_out.trim().is_empty();
        tally(&mut row, case, old == new && quiet, || {
            format!("old={old:?} new={new:?} stdout={old_out:?}/{new_out:?}")
        });
    }
    row
}

fn gate_cases(lab: &Lab) -> Vec<Case> {
    let opener = lab.write(
        "open-pr.ps1",
        b"gh api -X POST repos/o/r/pulls --input body.json\n",
    );
    let other = lab.write("other.js", b"console.log('hello')\n");
    let mut big = b"gh api -X POST repos/o/r/pulls --input b.json\n".to_vec();
    big.resize(1024 * 1024 + 10, b'#');
    let big = lab.write("big-pr.ps1", &big);
    let dir = lab.dir.to_string_lossy().replace('\\', "/");
    let cmd = |name: &str, command: &str, cwd: &str| {
        let payload = json!({"hook_event_name": "PreToolUse", "cwd": cwd, "tool_input": {"command": command}});
        Case::new(name, &payload.to_string())
    };
    [
        ("gh-pr-create", "gh pr create -t x -b y".to_string(), ""),
        ("cd-then-gh-exe", "cd C:/r; gh.exe pr create".into(), ""),
        (
            "api-post-input",
            "gh api -X POST repos/o/r/pulls --input pr.json".into(),
            "",
        ),
        (
            "api-fields",
            "gh api repos/o/r/pulls -f title=x -f head=b".into(),
            "",
        ),
        (
            "curl-post",
            "curl -s -X POST https://api.github.com/repos/o/r/pulls -d @b.json".into(),
            "",
        ),
        (
            "irm-post",
            "Invoke-RestMethod -Method Post -Uri https://api.github.com/repos/o/r/pulls".into(),
            "",
        ),
        (
            "api-patch-pr",
            "gh api repos/o/r/pulls/12 -X PATCH -f state=closed".into(),
            "",
        ),
        ("api-get-pulls", "gh api repos/o/r/pulls".into(), ""),
        ("git-status", "git status".into(), ""),
        ("gh-pr-view", "gh pr view 5".into(), ""),
        ("script-opens-pr", format!("powershell -File {opener}"), ""),
        ("script-other", format!("node {other}"), ""),
        ("relative-script", "pwsh ./open-pr.ps1".into(), dir.as_str()),
        ("big-script", format!("powershell -File {big}"), ""),
    ]
    .into_iter()
    .map(|(name, command, cwd)| cmd(name, &command, cwd))
    .collect()
}

fn old_detect(lab: &Lab, detector: &str, input: &[u8]) -> bool {
    let call = format!(
        "{UTF8_CONSOLE} . '{detector}'; $in = [Console]::In.ReadToEnd() | ConvertFrom-Json; $cwd = if ($in.cwd) {{ $in.cwd }} else {{ (Get-Location).Path }}; if (Test-PrCreateCommand $in.tool_input.command $cwd) {{ 'True' }} else {{ 'False' }}"
    );
    let args = [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &call,
    ]
    .map(String::from)
    .to_vec();
    spawn("powershell", &args, input, &lab.base_env()).trim() == "True"
}

/// Fake gate config for the binary: both stages point at a script that echoes what it got.
fn gate_env(lab: &Lab) -> Vec<(String, String)> {
    let gate = lab.write("fake-gate.ps1", FAKE_GATE.as_bytes());
    let config = lab.write(
        "gate.toml",
        format!("[quality_gate]\npre_script = \"{gate}\"\npost_script = \"{gate}\"\n").as_bytes(),
    );
    let mut env = lab.base_env();
    env.push(("CC_HOOKS_CONFIG".into(), config));
    env
}

fn relayed_ok(stdout: &str, input_len: usize) -> Option<bool> {
    let answer = parse_answer(stdout)?;
    Some(answer["gated"] == true && answer["bytes"] == input_len)
}

pub fn run_gate_suite(lab: &Lab) -> Row {
    let pre = local_config().and_then(|c| {
        c.get("quality_gate")?
            .get("pre_script")?
            .as_str()
            .map(String::from)
    });
    let pre = pre.expect("quality_gate.pre_script in the local cc-hooks.toml");
    let detector = std::path::Path::new(&pre)
        .with_file_name("pr-create-detect.ps1")
        .to_string_lossy()
        .into_owned();
    let env = gate_env(lab);
    let cases = gate_cases(lab);
    let mut row = Row {
        hook: "quality-gate (detect+relay)".into(),
        cases: cases.len(),
        ..Row::default()
    };
    for case in &cases {
        let old = old_detect(lab, &detector, &case.input);
        let new_out = spawn(
            &lab.bin.to_string_lossy(),
            &["quality-gate".to_string()],
            &case.input,
            &env,
        );
        let relay = relayed_ok(&new_out, case.input.len());
        let same = old == relay.is_some() && relay != Some(false);
        tally(&mut row, case, same, || {
            format!("old detect={old} new stdout={new_out:?}")
        });
    }
    row
}
