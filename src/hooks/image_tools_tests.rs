use super::*;
use crate::fs_port::fakes::FakeFileSystem;
use serde_json::json;

const PASTED: &str = r#"{"type":"user","uuid":"u-7","message":{"content":[{"type":"text","text":"what is this"},{"type":"image","source":{}}]}}"#;

fn env_with(fs: &FakeFileSystem) -> ImageToolsEnv<'_> {
    ImageToolsEnv {
        fs,
        state_dir: PathBuf::from("state"),
        tool_path: "T/imgtool.py".into(),
    }
}

fn context_of(outcome: Outcome) -> String {
    let Outcome::Emit(value) = outcome else {
        panic!("expected output, got {outcome:?}")
    };
    value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn run_reminds_once_per_pasted_message() {
    let fs = FakeFileSystem::default().with_file("t.jsonl", PASTED.as_bytes());
    let env = env_with(&fs);
    let payload =
        json!({"hook_event_name": "PreToolUse", "transcript_path": "t.jsonl", "session_id": "s"});
    assert!(context_of(run(&payload, &env)).contains("Save the 1 pasted image(s)"));
    assert_eq!(
        fs.text_of("state\\reminded-s.txt")
            .or(fs.text_of("state/reminded-s.txt"))
            .as_deref(),
        Some("u-7")
    );
    assert_eq!(run(&payload, &env), Outcome::Silent);
}

#[test]
fn run_lists_prompt_paths_at_prompt_submit() {
    let fs = FakeFileSystem::default();
    let payload =
        json!({"hook_event_name": "UserPromptSubmit", "prompt": "compare C:/x/b.png and a.JPG"});
    let text = context_of(run(&payload, &env_with(&fs)));
    assert!(text.contains("1. Image path(s) in the prompt: C:/x/b.png, a.JPG"));
}

#[test]
fn run_honours_opt_out_and_no_image() {
    let fs = FakeFileSystem::default().with_file("t.jsonl", PASTED.as_bytes());
    let env = env_with(&fs);
    let opted = json!({"hook_event_name": "UserPromptSubmit", "prompt": "a.png, no zoom"});
    assert_eq!(run(&opted, &env), Outcome::Silent);
    assert_eq!(
        run(&json!({"hook_event_name": "PreToolUse"}), &env),
        Outcome::Silent
    );
}

#[test]
fn sight_only_counts_entry_matching_the_prompt() {
    let entry: Value = serde_json::from_str(PASTED).unwrap();
    let matched = sight("UserPromptSubmit", " what is this ", Some(&entry));
    assert_eq!((matched.pasted, matched.uid.as_deref()), (1, Some("u-7")));
    let other = sight("UserPromptSubmit", "different", Some(&entry));
    assert_eq!(other, Sighting::default());
    assert_eq!(sight("PreToolUse", "", Some(&entry)).pasted, 1);
}

#[test]
fn image_paths_sorted_and_unique() {
    assert_eq!(
        image_paths("b.png a.png b.png x.pngx"),
        vec!["a.png", "b.png"]
    );
}

#[test]
fn entry_text_handles_string_and_parts() {
    assert_eq!(entry_text(&json!({"message": {"content": "hi"}})), "hi");
    let parts = json!({"message": {"content": [{"type": "text", "text": "a"}, {"type": "text"}]}});
    assert_eq!(entry_text(&parts), "a\n");
    assert_eq!(entry_text(&json!({})), "");
}

#[test]
fn image_count_counts_image_parts() {
    let entry: Value = serde_json::from_str(PASTED).unwrap();
    assert_eq!(image_count(&entry), 1);
    assert_eq!(image_count(&json!({"message": {"content": "x"}})), 0);
}

#[test]
fn fallback_key_is_stable_only_for_empty_prompt() {
    assert_eq!(fallback_key(""), "prompt:0");
    assert!(fallback_key("p").starts_with("prompt:"));
}

#[test]
fn already_reminded_compares_trimmed_state() {
    let fs = FakeFileSystem::default();
    let env = env_with(&fs);
    assert!(!already_reminded(&env, "s", "k"));
    assert!(already_reminded(&env, "s", "k"));
    assert!(!already_reminded(&env, "s", "other"));
}

#[test]
fn reminder_text_includes_both_sources() {
    let text = reminder_text("T", "t.jsonl", 2, &["a.png".into()]);
    assert!(text.contains("   python \"T\" extract \"t.jsonl\""));
    assert!(text.contains("1. Image path(s) in the prompt: a.png"));
    assert!(text.ends_with("the message they just typed."));
}

#[test]
fn default_tool_path_uses_forward_slashes() {
    assert!(default_tool_path().ends_with("/.claude/tools/imgtool.py"));
    assert!(!default_tool_path().contains('\\'));
}
