//! Maps a subcommand to its hook and wires the real dependencies (disk, processes, config, env).

use crate::config::{Config, load_config};
use crate::fs_port::DiskFileSystem;
use crate::hook_io::{Outcome, parse_payload};
use crate::hooks::quality_gate::{GateDeps, GateStage};
use crate::hooks::{
    attribution, context_inject, design_mockup, git_identity, grill, herdr_session,
};
use crate::hooks::{image_tools, quality_gate};
use crate::proc::SystemProcessRunner;
use serde_json::Value;

/// Every subcommand name, for usage text and tests.
pub const SUBCOMMANDS: [&str; 8] = [
    "attribution",
    "grill",
    "design-mockup",
    "image-tools",
    "herdr-session",
    "context-inject",
    "quality-gate",
    "git-identity",
];

/// Runs the hook named by `args[0]` on the raw stdin bytes. Unknown names and bad input are silent.
///
/// Example: `dispatch(&["grill".into()], br#"{"prompt":"add a page"}"#)` -> `Outcome::Emit(..)`
pub fn dispatch(args: &[String], raw: &[u8]) -> Outcome {
    let Some(name) = args.first() else {
        return Outcome::Silent;
    };
    let Some(payload) = parse_payload(raw) else {
        return Outcome::Silent;
    };
    let post = args.iter().any(|a| a == "--post");
    match name.as_str() {
        "attribution" => attribution::run(&payload, &DiskFileSystem),
        "grill" => grill::run(&payload),
        "design-mockup" => design_mockup::run(&payload),
        "image-tools" => run_image_tools(&payload),
        "herdr-session" => herdr_session::run(
            &payload,
            &herdr_session::HerdrEnv::from_process(),
            &SystemProcessRunner,
        ),
        "context-inject" => run_context_inject(&payload),
        "quality-gate" => run_quality_gate(&payload, raw, post),
        "git-identity" => run_git_identity(&payload),
        _ => Outcome::Silent,
    }
}

fn run_image_tools(payload: &Value) -> Outcome {
    let env = image_tools::ImageToolsEnv {
        fs: &DiskFileSystem,
        state_dir: std::env::temp_dir().join("claude-img"),
        tool_path: image_tools::default_tool_path(),
    };
    image_tools::run(payload, &env)
}

fn run_context_inject(payload: &Value) -> Outcome {
    let rules = load_config().map(|c| c.context_inject).unwrap_or_default();
    context_inject::run(payload, &rules, &DiskFileSystem, &process_cwd())
}

fn run_quality_gate(payload: &Value, raw: &[u8], post: bool) -> Outcome {
    let config: Option<Config> = load_config();
    let deps = GateDeps {
        config: config.as_ref().and_then(|c| c.quality_gate.as_ref()),
        fs: &DiskFileSystem,
        runner: &SystemProcessRunner,
        disabled: std::env::var("QUALITY_GATE").is_ok_and(|v| v.eq_ignore_ascii_case("off")),
        process_cwd: process_cwd(),
    };
    let stage = if post {
        GateStage::Post
    } else {
        GateStage::Pre
    };
    quality_gate::run(payload, raw, stage, &deps)
}

fn run_git_identity(payload: &Value) -> Outcome {
    let config = load_config();
    let probe = git_identity::CliGitProbe {
        runner: &SystemProcessRunner,
    };
    let section = config.as_ref().and_then(|c| c.git_identity.as_ref());
    git_identity::run(payload, section, &probe, &process_cwd())
}

fn process_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_is_silent_for_unknown_or_bad_input() {
        assert_eq!(dispatch(&[], b"{}"), Outcome::Silent);
        assert_eq!(dispatch(&["nope".into()], b"{}"), Outcome::Silent);
        assert_eq!(dispatch(&["grill".into()], b"not json"), Outcome::Silent);
    }

    #[test]
    fn dispatch_routes_prompt_hooks() {
        let raw = br#"{"prompt":"add a new settings page"}"#;
        assert!(matches!(dispatch(&["grill".into()], raw), Outcome::Emit(_)));
        assert_eq!(dispatch(&["design-mockup".into()], raw), Outcome::Silent);
    }

    #[test]
    fn subcommands_all_dispatch_without_panicking() {
        for name in SUBCOMMANDS {
            let _ = dispatch(&[name.to_string()], br#"{"tool_input":{"command":"ls"}}"#);
        }
    }

    #[test]
    fn process_cwd_is_not_empty() {
        assert!(!process_cwd().is_empty());
    }
}
