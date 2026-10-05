//! `cc-hooks context-inject` (SessionStart): injects a context file only for sessions whose cwd is
//! under a configured folder, so project-family instructions do not load on every project.

use crate::config::{ContextRule, expand_env};
use crate::fs_port::FileSystem;
use crate::hook_io::{Outcome, context_answer, str_at};
use serde_json::Value;
use std::path::Path;

/// Emits header + file body for every rule whose prefix contains the session cwd.
///
/// Example: `run(&json!({"cwd":"X:/work/repo"}), &rules, &fs, "C:/")` -> context for the X:/work rule
pub fn run(
    payload: &Value,
    rules: &[ContextRule],
    fs: &dyn FileSystem,
    process_cwd: &str,
) -> Outcome {
    let cwd = str_at(payload, &["cwd"])
        .filter(|c| !c.is_empty())
        .unwrap_or(process_cwd);
    let blocks: Vec<String> = rules
        .iter()
        .filter_map(|rule| render_rule(rule, cwd, fs))
        .collect();
    if blocks.is_empty() {
        return Outcome::Silent;
    }
    Outcome::Emit(context_answer("SessionStart", &blocks.concat(), None))
}

fn render_rule(rule: &ContextRule, cwd: &str, fs: &dyn FileSystem) -> Option<String> {
    if !is_under(cwd, &rule.cwd_prefix) {
        return None;
    }
    let body = fs.read_text(Path::new(&expand_env(&rule.file)))?;
    Some(format!("{}{body}", rule.header))
}

/// True when `path` is `prefix` or inside it: case-insensitive, either slash.
///
/// Example: `is_under("x:\\Work\\repo", "X:/work")` -> `true`; `is_under("X:/workshop", "X:/work")` -> `false`
pub fn is_under(path: &str, prefix: &str) -> bool {
    let path = normalize(path);
    let prefix = normalize(prefix);
    let prefix = prefix.trim_end_matches('/');
    let Some(rest) = path.strip_prefix(prefix) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('/')
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_port::fakes::FakeFileSystem;
    use serde_json::json;

    fn rule() -> ContextRule {
        ContextRule {
            cwd_prefix: "X:/work".into(),
            file: "ctx.md".into(),
            header: "H:\n\n".into(),
        }
    }

    #[test]
    fn run_injects_header_and_body_under_prefix() {
        let fs = FakeFileSystem::default().with_file("ctx.md", b"\xEF\xBB\xBFbody");
        let Outcome::Emit(answer) = run(&json!({"cwd": "x:\\WORK\\repo"}), &[rule()], &fs, "C:/")
        else {
            panic!()
        };
        assert_eq!(
            answer["hookSpecificOutput"]["additionalContext"],
            "H:\n\nbody"
        );
        assert_eq!(
            answer["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
    }

    #[test]
    fn run_falls_back_to_process_cwd_and_skips_missing_file() {
        let fs = FakeFileSystem::default();
        assert_eq!(run(&json!({}), &[rule()], &fs, "X:/work"), Outcome::Silent);
        let fs = fs.with_file("ctx.md", b"b");
        assert!(matches!(
            run(&json!({}), &[rule()], &fs, "X:/work"),
            Outcome::Emit(_)
        ));
    }

    #[test]
    fn render_rule_ignores_other_folders() {
        let fs = FakeFileSystem::default().with_file("ctx.md", b"b");
        assert_eq!(render_rule(&rule(), "C:/elsewhere", &fs), None);
    }

    #[test]
    fn is_under_needs_a_folder_boundary() {
        assert!(is_under("X:/work", "X:/work/"));
        assert!(is_under("x:\\work\\a", "X:/work"));
        assert!(!is_under("X:/workshop", "X:/work"));
    }
}
