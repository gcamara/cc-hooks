//! The corpora: allow and deny/inject cases, accents, slash commands, opt-outs, BOM input.

use super::{Case, Lab};
use serde_json::{Value, json};

fn shell(cmd: &str) -> String {
    json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": cmd}})
        .to_string()
}

fn prompt(text: &str) -> String {
    json!({"hook_event_name": "UserPromptSubmit", "session_id": "@SESSION@", "prompt": text})
        .to_string()
}

pub fn attribution_cases(lab: &Lab) -> Vec<Case> {
    let robot_body = lab.write(
        "robot-body.md",
        "Summary\n\n\u{1F916} made by a tool\n".as_bytes(),
    );
    let footer_body = lab.write(
        "footer-body.md",
        b"Generated with [Claude Code](https://example)\n",
    );
    let clean_body = lab.write(
        "clean-body.md",
        b"Touches .claude/hooks and src/claude.js only.\n",
    );
    vec![
        Case::new(
            "clean-commit",
            &shell("git commit -m \"fix: parse config\""),
        ),
        Case::new(
            "co-author-trailer",
            &shell("git commit -m \"x\n\nCo-Authored-By: A <a@b.example>\""),
        ),
        Case::new(
            "robot-body-file",
            &shell(&format!("gh pr create -t x --body-file {robot_body}")),
        ),
        Case::new(
            "footer-body-file",
            &shell(&format!("gh pr create -t x -F \"{footer_body}\"")),
        ),
        Case::new(
            "clean-body-file",
            &shell(&format!("gh pr create -t x --body-file {clean_body}")),
        ),
        Case::new("bare-word", &shell("git commit -m \"written by Claude\"")),
        Case::new(
            "paths-allowed",
            &shell("git commit -m \"update .claude/ claude-code-guide claudeApi src/claude.js\""),
        ),
        Case::new("not-publishing", &shell("echo Claude Anthropic")),
        Case::new(
            "gh-api-url",
            &shell("gh api repos/o/r/issues -f body=see-claude.ai/code"),
        ),
        Case::new("dash-c-tag", &shell("git -C sub tag -m \"Anthropic\" v1")),
        Case::new(
            "accent-bare-word",
            &shell("git commit -m \"correção feita pelo Claude\""),
        )
        .accented(),
        Case::with_bom("bom-trailer", &shell("git commit -m \"Co-Authored-By: x\"")),
    ]
}

pub fn grill_cases() -> Vec<Case> {
    vec![
        Case::new("feature", &prompt("add a new settings page")),
        Case::new(
            "feature-no-agents",
            &prompt("implement the export endpoint, no agents"),
        ),
        Case::new("fix", &prompt("fix the crash on login")),
        Case::new("fix-no-agents", &prompt("fix the typo, single agent")),
        Case::new("question", &prompt("how does the report page work?")),
        Case::new("slash", &prompt("/review add a new page")),
        Case::new("no-grill", &prompt("build the dashboard, no grill")),
        Case::new("mockup", &prompt("mock up the new dashboard")),
        Case::new("trivial-new", &prompt("add a new test")),
        Case::new("pt-feature", &prompt("adiciona a página de relatório")).accented(),
        Case::new("pt-fix", &prompt("corrigir o erro na integração")).accented(),
        Case::new(
            "pt-novelty",
            &prompt("cria uma nova funcionalidade sozinho"),
        )
        .accented(),
        Case::with_bom("bom-feature", &prompt("add a new settings page")),
    ]
}

pub fn design_cases() -> Vec<Case> {
    vec![
        Case::new("mockup", &prompt("mock up the camera grid")),
        Case::new("mock-object-up", &prompt("mock the radar page up")),
        Case::new("set-up-excluded", &prompt("mock the radar set up")),
        Case::new("opt-out", &prompt("mockup of the page, no design")),
        Case::new("test-double", &prompt("mock up the jest module")),
        Case::new("slash", &prompt("/design mockup")),
        Case::new("not-a-mockup", &prompt("add a settings page")),
        Case::new("mocked-up", &prompt("I want it mocked-up first")),
        Case::new("pt-mockup", &prompt("faz um mockup da tela de câmeras")).accented(),
        Case::with_bom("bom-mockup", &prompt("mock up the camera grid")),
    ]
}

fn transcript_line(entry: Value) -> String {
    format!("{entry}\n")
}

fn pasted_transcript(lab: &Lab) -> String {
    let human = json!({"type": "user", "uuid": "u-1", "message": {"content": [
        {"type": "text", "text": "what is in this screenshot"}, {"type": "image", "source": {"data": "AAAA"}}]}});
    let result =
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "ok"}]}});
    let meta = json!({"type": "user", "isMeta": true, "message": {"content": "caveat"}});
    let body = [human, json!({"type": "assistant"}), result, meta]
        .map(transcript_line)
        .concat();
    lab.write("pasted.jsonl", body.as_bytes())
}

fn plain_transcript(lab: &Lab, text: &str) -> String {
    let human = json!({"type": "user", "uuid": "u-2", "message": {"content": text}});
    lab.write(
        &format!("plain-{}.jsonl", text.len()),
        transcript_line(human).as_bytes(),
    )
}

fn image_event(event: &str, transcript: &str, text: &str) -> String {
    let mut payload =
        json!({"hook_event_name": event, "session_id": "@SESSION@", "transcript_path": transcript});
    if event == "UserPromptSubmit" {
        payload["prompt"] = Value::from(text);
    }
    payload.to_string()
}

pub fn image_cases(lab: &Lab) -> Vec<Case> {
    let pasted = pasted_transcript(lab);
    let plain = plain_transcript(lab, "just text");
    let opted = plain_transcript(lab, "look at it, sem crop");
    let ups = "UserPromptSubmit";
    vec![
        Case::new(
            "ups-pasted",
            &image_event(ups, &pasted, "what is in this screenshot"),
        ),
        Case::new(
            "ups-paths",
            &prompt("compare C:/shots/b.png and a.JPG and a.JPG"),
        ),
        Case::new("pretool-pasted", &image_event("PreToolUse", &pasted, "")),
        Case::new("pretool-dedupe", &image_event("PreToolUse", &pasted, "")).twice(),
        Case::new("pretool-plain", &image_event("PreToolUse", &plain, "")),
        Case::new("opt-out-prompt", &prompt("see a.png, no zoom")),
        Case::new("opt-out-entry", &image_event("PreToolUse", &opted, "")),
        Case::new(
            "ups-other-message",
            &image_event(ups, &pasted, "a different prompt"),
        ),
        Case::new("no-transcript", &image_event("PreToolUse", "", "")),
        Case::new("pt-path", &prompt("veja a imagem página.png")).accented(),
        // Python strips the BOM itself once stdin is decoded as UTF-8, so this one must match.
        Case::new("bom-paths", &prompt("bom input with shot.webp"))
            .bom_prefixed()
            .accented(),
    ]
}

pub fn context_cases() -> Vec<Case> {
    let prefix = context_prefix().expect("a [[context_inject]] rule in the local cc-hooks.toml");
    let cwd = |dir: &str| json!({"hook_event_name": "SessionStart", "cwd": dir}).to_string();
    vec![
        // The context file may hold non-ASCII text, which 5.1 garbles on a raw OEM-code-page stdout.
        Case::new("prefix", &cwd(&prefix)).accented(),
        Case::new("below", &cwd(&format!("{prefix}/repo/src"))),
        Case::new(
            "backslash-lower",
            &cwd(&format!(
                "{}\\repo",
                prefix.replace('/', "\\").to_lowercase()
            )),
        ),
        Case::new("upper", &cwd(&prefix.to_uppercase())),
        Case::new("no-boundary", &cwd(&format!("{prefix}x"))),
        Case::new("elsewhere", &cwd("C:/elsewhere/repo")),
        Case::new(
            "no-cwd",
            &json!({"hook_event_name": "SessionStart"}).to_string(),
        ),
        Case::new("empty-cwd", &cwd("")),
        Case::with_bom("bom-prefix", &cwd(&prefix)),
    ]
}

/// The first context rule's prefix from the local config (the old script hardcodes the same).
pub fn context_prefix() -> Option<String> {
    let config = local_config()?;
    let rules = config.get("context_inject")?.as_array()?;
    Some(rules.first()?.get("cwd_prefix")?.as_str()?.to_string())
}

/// The local cc-hooks.toml as a TOML tree.
pub fn local_config() -> Option<toml::Value> {
    let home = std::env::var("USERPROFILE").ok()?;
    let text = std::fs::read_to_string(format!("{home}/.claude/cc-hooks.toml")).ok()?;
    toml::from_str(&text).ok()
}
