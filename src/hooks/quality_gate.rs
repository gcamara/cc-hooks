//! `cc-hooks quality-gate [--post]` (PreToolUse / PostToolUse, Bash|PowerShell): the FAST PATH in
//! front of the quality-gate PowerShell scripts.
//!
//! Why: the gate scripts cost ~500 ms of interpreter start on every shell command, yet only PR
//! creations matter. Anything else exits here in a few ms; a PR creation execs the configured
//! script with the original stdin and relays its stdout and exit code unchanged.

use super::pr_detect::is_pr_create;
use crate::config::QualityGateConfig;
use crate::fs_port::FileSystem;
use crate::hook_io::{Outcome, str_at};
use crate::proc::ProcessRunner;
use serde_json::Value;

/// Which gate script to hand a PR creation to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateStage {
    Pre,
    Post,
}

/// Everything the fast path needs besides the payload.
pub struct GateDeps<'a> {
    pub config: Option<&'a QualityGateConfig>,
    pub fs: &'a dyn FileSystem,
    pub runner: &'a dyn ProcessRunner,
    /// `$env:QUALITY_GATE == 'off'`.
    pub disabled: bool,
    pub process_cwd: String,
}

/// Silent for anything that is not a PR creation; otherwise relays the gate script's answer.
///
/// Example: `run(&payload, raw_stdin, GateStage::Pre, &deps)` -> `Outcome::Relay { .. }` for `gh pr create`
pub fn run(payload: &Value, raw_stdin: &[u8], stage: GateStage, deps: &GateDeps) -> Outcome {
    if deps.disabled {
        return Outcome::Silent;
    }
    let Some(script) = script_for(deps.config, stage) else {
        return Outcome::Silent;
    };
    let command = str_at(payload, &["tool_input", "command"]).unwrap_or("");
    let cwd = str_at(payload, &["cwd"])
        .filter(|c| !c.is_empty())
        .unwrap_or(&deps.process_cwd);
    if !is_pr_create(command, cwd, deps.fs) {
        return Outcome::Silent;
    }
    match deps
        .runner
        .run_piped("powershell", &script_args(script), raw_stdin)
    {
        Some(answer) => Outcome::Relay {
            stdout: answer.stdout,
            code: answer.code,
        },
        None => Outcome::Silent,
    }
}

/// The configured script for the stage, ignoring blank entries.
///
/// Example: `script_for(Some(&cfg), GateStage::Post)` -> `Some("C:/gate/post.ps1")`
pub fn script_for(config: Option<&QualityGateConfig>, stage: GateStage) -> Option<&str> {
    let config = config?;
    let script = match stage {
        GateStage::Pre => config.pre_script.as_deref(),
        GateStage::Post => config.post_script.as_deref(),
    };
    script.filter(|s| !s.trim().is_empty())
}

/// The same PowerShell invocation settings.json used to make.
///
/// Example: `script_args("g.ps1")` -> `["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "g.ps1"]`
pub fn script_args(script: &str) -> Vec<String> {
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script]
        .map(str::to_string)
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_port::fakes::FakeFileSystem;
    use crate::proc::ChildAnswer;
    use crate::proc::fakes::RecordingProcessRunner;
    use serde_json::json;

    fn config() -> QualityGateConfig {
        QualityGateConfig {
            pre_script: Some("pre.ps1".into()),
            post_script: Some(" ".into()),
        }
    }

    fn deps<'a>(
        cfg: &'a QualityGateConfig,
        fs: &'a FakeFileSystem,
        runner: &'a RecordingProcessRunner,
    ) -> GateDeps<'a> {
        GateDeps {
            config: Some(cfg),
            fs,
            runner,
            disabled: false,
            process_cwd: "C:/".into(),
        }
    }

    fn runner_answering() -> RecordingProcessRunner {
        let answer = ChildAnswer {
            stdout: b"{\"x\":1}\n".to_vec(),
            code: 0,
        };
        RecordingProcessRunner {
            piped_answer: Some(answer),
            ..Default::default()
        }
    }

    #[test]
    fn run_relays_gate_for_pr_create() {
        let (cfg, fs, runner) = (config(), FakeFileSystem::default(), runner_answering());
        let payload = json!({"tool_input": {"command": "gh pr create -t x"}});
        let outcome = run(&payload, b"raw", GateStage::Pre, &deps(&cfg, &fs, &runner));
        assert_eq!(
            outcome,
            Outcome::Relay {
                stdout: b"{\"x\":1}\n".to_vec(),
                code: 0
            }
        );
        assert_eq!(runner.calls.borrow()[0].2, b"raw");
    }

    #[test]
    fn run_is_silent_for_other_commands_and_when_off() {
        let (cfg, fs, runner) = (config(), FakeFileSystem::default(), runner_answering());
        let other = json!({"tool_input": {"command": "git status"}});
        assert_eq!(
            run(&other, b"", GateStage::Pre, &deps(&cfg, &fs, &runner)),
            Outcome::Silent
        );
        let pr = json!({"tool_input": {"command": "gh pr create"}});
        let off = GateDeps {
            disabled: true,
            ..deps(&cfg, &fs, &runner)
        };
        assert_eq!(run(&pr, b"", GateStage::Pre, &off), Outcome::Silent);
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn script_for_needs_config_and_non_blank_path() {
        assert_eq!(script_for(Some(&config()), GateStage::Pre), Some("pre.ps1"));
        assert_eq!(script_for(Some(&config()), GateStage::Post), None);
        assert_eq!(script_for(None, GateStage::Pre), None);
    }

    #[test]
    fn script_args_matches_old_invocation() {
        assert_eq!(
            script_args("g.ps1").join(" "),
            "-NoProfile -ExecutionPolicy Bypass -File g.ps1"
        );
    }
}
