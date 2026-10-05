//! `cc-hooks attribution` (PreToolUse, Bash|PowerShell): hard-blocks AI-tool attribution from
//! reaching a commit, PR, issue, release or comment.
//!
//! Why a deny hook: the harness instructions tell the model to append a co-author trailer and a
//! "Generated with" footer; prose in a memory file loses to that prose, a deny does not.
//! Scans the command line AND every file passed as --body-file / -F / --file / --body / -m.

use crate::fs_port::FileSystem;
use crate::hook_io::{Outcome, deny_answer, str_at};
use crate::text::{fixed_fancy, fixed_regex};
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;

const RULE: &str = "BLOCKED by ~/.claude/CLAUDE.md (Regras de commit): ";
const ADVICE: &str = "Never add Co-Authored-By, a \"Generated with Claude Code\" footer, the robot emoji, or any Anthropic/Claude attribution to commits, PRs, descriptions or comments. Remove it and run the command again.";
const ROBOT: char = '\u{1F916}';

// PowerShell `-match` is case-insensitive, hence (?i); `[regex]::Matches` is not, hence none below.
static PUBLISHES: LazyLock<regex::Regex> = LazyLock::new(|| {
    fixed_regex(
        r"(?i)git\s+(-C\s+\S+\s+)?(commit|merge|tag|notes|revert)|gh\s+(pr|issue|release)\s|gh\s+api\s",
    )
});
static FILE_REFS: LazyLock<regex::Regex> =
    LazyLock::new(|| fixed_regex(r#"(?:--body-file|--file|--body|-F|-m)[=\s]+["']?([^"'\s]+)"#));
static SIGNATURES: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"Co-?Authored-?By", "a Co-Authored-By trailer"),
        (r"noreply@anthropic\.com", "the Anthropic noreply address"),
        (
            r"Generated with\s+\[?\s*Claude",
            "a \"Generated with Claude Code\" footer",
        ),
        (r"claude\.com/claude-code", "a Claude Code URL"),
        (r"claude\.ai/code", "a Claude Code URL"),
    ]
    .into_iter()
    .map(|(pattern, why)| (fixed_regex(&format!("(?i){pattern}")), why))
    .collect()
});
// Allows the word glued into a path or identifier (.claude/, claude-code-guide, claudeApi).
static BARE_WORD: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| fixed_fancy(r"(?i)(?<![\w\-/\\.])(claude|anthropic)(?![\w\-/\\.])"));

/// Denies a publishing command that carries attribution; everything else is silent.
///
/// Example: `run(&json!({"tool_input":{"command":"git commit -m 'by Claude'"}}), &DiskFileSystem)` -> deny
pub fn run(payload: &Value, fs: &dyn FileSystem) -> Outcome {
    let Some(command) = str_at(payload, &["tool_input", "command"]) else {
        return Outcome::Silent;
    };
    if command.is_empty() || !PUBLISHES.is_match(command) {
        return Outcome::Silent;
    }
    match find_violation(&scan_text(command, fs)) {
        Some(reason) => Outcome::Emit(deny_answer(&format!("{RULE}{reason}"))),
        None => Outcome::Silent,
    }
}

/// The command line followed by the body of every readable file it references.
///
/// Example: `scan_text("gh pr create --body-file b.md", &fs)` -> "gh pr create ...\r\n<b.md>\r\n"
pub fn scan_text(command: &str, fs: &dyn FileSystem) -> String {
    let mut text = format!("{command}\r\n");
    for reference in FILE_REFS.captures_iter(command) {
        if let Some(body) = fs.read_text(Path::new(&reference[1])) {
            text.push_str(&body);
            text.push_str("\r\n");
        }
    }
    text
}

/// The deny reason (without the rule prefix) for the first attribution found, in priority order.
///
/// Example: `find_violation("Co-Authored-By: x")` -> `Some("this command carries a Co-Authored-By trailer. ...")`
pub fn find_violation(text: &str) -> Option<String> {
    if text.contains(ROBOT) {
        return Some(format!(
            "this command carries the robot-emoji attribution footer. {ADVICE}"
        ));
    }
    if let Some((_, why)) = SIGNATURES
        .iter()
        .find(|(pattern, _)| pattern.is_match(text))
    {
        return Some(format!("this command carries {why}. {ADVICE}"));
    }
    bare_mention(text).map(|hit| {
        format!("this command mentions '{hit}' as a standalone word in a commit message or PR/issue body. No references to Claude or Anthropic in published text. (Paths and identifiers like .claude/, claude-code-guide or claudeApi are fine - a bare mention is not.) Reword it and run the command again.")
    })
}

/// The first standalone "claude"/"anthropic", as written.
///
/// Example: `bare_mention("see src/claude.js, by Claude")` -> `Some("Claude")`
pub fn bare_mention(text: &str) -> Option<String> {
    let captures = BARE_WORD.captures(text).ok()??;
    Some(captures.get(1)?.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_port::fakes::FakeFileSystem;
    use serde_json::json;

    fn command(cmd: &str) -> Value {
        json!({"tool_input": {"command": cmd}})
    }

    fn reason(outcome: Outcome) -> String {
        let Outcome::Emit(value) = outcome else {
            panic!("expected deny, got {outcome:?}")
        };
        value["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn run_ignores_non_publishing_commands() {
        let fs = FakeFileSystem::default();
        assert_eq!(run(&command("echo Claude"), &fs), Outcome::Silent);
        assert_eq!(run(&json!({"tool_input": {}}), &fs), Outcome::Silent);
    }

    #[test]
    fn run_denies_trailer_in_commit_message() {
        let fs = FakeFileSystem::default();
        let text = reason(run(
            &command("git commit -m \"x\n\nCo-authored-by: A <a@b>\""),
            &fs,
        ));
        assert!(text.starts_with(RULE));
        assert!(text.contains("a Co-Authored-By trailer"));
    }

    #[test]
    fn run_allows_clean_commit_and_uppercase_git() {
        let fs = FakeFileSystem::default();
        assert_eq!(
            run(&command("git commit -m 'fix .claude/ paths'"), &fs),
            Outcome::Silent
        );
        assert!(matches!(
            run(&command("GIT COMMIT -m 'by claude'"), &fs),
            Outcome::Emit(_)
        ));
    }

    #[test]
    fn scan_text_reads_referenced_body_files() {
        let fs = FakeFileSystem::default().with_file("b.md", "made with \u{1F916}".as_bytes());
        let text = scan_text("gh pr create --body-file b.md -m missing.md", &fs);
        assert_eq!(
            text,
            "gh pr create --body-file b.md -m missing.md\r\nmade with \u{1F916}\r\n"
        );
        assert!(
            reason(run(&command("gh pr create --body-file \"b.md\""), &fs)).contains("robot-emoji")
        );
    }

    #[test]
    fn scan_text_file_flags_are_case_sensitive() {
        let fs = FakeFileSystem::default().with_file("b.md", b"Claude");
        assert_eq!(scan_text("gh api -f b.md", &fs), "gh api -f b.md\r\n");
    }

    #[test]
    fn find_violation_follows_priority_order() {
        assert!(
            find_violation("Co-Authored-By \u{1F916}")
                .unwrap()
                .contains("robot-emoji")
        );
        assert!(
            find_violation("noreply@anthropic.com")
                .unwrap()
                .contains("noreply address")
        );
        assert!(
            find_violation("Generated with [Claude Code]")
                .unwrap()
                .contains("footer")
        );
        assert!(
            find_violation("see claude.ai/code")
                .unwrap()
                .contains("Claude Code URL")
        );
        assert_eq!(find_violation("plain message"), None);
    }

    #[test]
    fn bare_mention_allows_paths_and_identifiers() {
        for allowed in [
            ".claude/",
            "claude-code-guide",
            "claudeApi",
            "src/claude.js",
            "x\\claude",
        ] {
            assert_eq!(bare_mention(allowed), None, "{allowed}");
        }
    }

    #[test]
    fn bare_mention_denies_standalone_words() {
        assert_eq!(bare_mention("written by Claude").as_deref(), Some("Claude"));
        assert_eq!(
            bare_mention("ANTHROPIC rocks").as_deref(),
            Some("ANTHROPIC")
        );
        assert_eq!(bare_mention("'claude'").as_deref(), Some("claude"));
    }
}
