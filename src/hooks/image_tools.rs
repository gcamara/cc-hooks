//! `cc-hooks image-tools` (UserPromptSubmit + PreToolUse on every tool): when the user sends an
//! image, tell the model to inspect it with Pillow/OpenCV through ~/.claude/tools/imgtool.py.
//!
//! In the desktop app a pasted image never shows up in the hook's `prompt`; it only exists as base64
//! in the transcript. So the latest human entry is checked; if it is not written yet at
//! UserPromptSubmit, the PreToolUse pass catches it on the first tool call. One reminder per user
//! message, deduped by the entry's uuid in %TEMP%\claude-img\reminded-<session>.txt (same files as
//! the Python hook).

use super::transcript::{content_items, latest_human_entry};
use crate::fs_port::FileSystem;
use crate::hook_io::{Outcome, context_answer, str_at};
use crate::text::fixed_regex;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static IMAGE_PATH: LazyLock<Regex> =
    LazyLock::new(|| fixed_regex(r#"(?i)[^\s"'<>|]+\.(png|jpe?g|gif|bmp|webp|tiff?)\b"#));
static OPT_OUT: LazyLock<Regex> = LazyLock::new(|| {
    fixed_regex(r"(?i)\b(no|skip|sem|without)\s+(crop|zoom|imgtool|pil|opencv)\b")
});

/// Where things live; built from the real environment in `main`, from fakes in tests.
pub struct ImageToolsEnv<'a> {
    pub fs: &'a dyn FileSystem,
    pub state_dir: PathBuf,
    pub tool_path: String,
}

/// What the hook found worth reminding about.
#[derive(Debug, Default, PartialEq)]
pub struct Sighting {
    pub pasted: usize,
    pub paths: Vec<String>,
    pub uid: Option<String>,
}

/// Emits the inspect-with-imgtool reminder once per user message that carries an image.
///
/// Example: `run(&json!({"hook_event_name":"UserPromptSubmit","prompt":"see a.png"}), &env)` -> reminder
pub fn run(payload: &Value, env: &ImageToolsEnv) -> Outcome {
    let event = str_at(payload, &["hook_event_name"]).unwrap_or("");
    let transcript = str_at(payload, &["transcript_path"]).unwrap_or("");
    let prompt = str_at(payload, &["prompt"]).unwrap_or("");
    if OPT_OUT.is_match(prompt) {
        return Outcome::Silent;
    }
    let entry = (!transcript.is_empty()).then(|| latest_human_entry(env.fs, Path::new(transcript)));
    let entry = entry.flatten();
    if entry
        .as_ref()
        .is_some_and(|e| OPT_OUT.is_match(&entry_text(e)))
    {
        return Outcome::Silent;
    }
    let sighting = sight(event, prompt, entry.as_ref());
    if sighting.pasted == 0 && sighting.paths.is_empty() {
        return Outcome::Silent;
    }
    remind_once(payload, env, &sighting, prompt, event, transcript)
}

fn remind_once(
    payload: &Value,
    env: &ImageToolsEnv,
    s: &Sighting,
    prompt: &str,
    event: &str,
    transcript: &str,
) -> Outcome {
    let session = str_at(payload, &["session_id"])
        .filter(|s| !s.is_empty())
        .unwrap_or("x");
    let key = s.uid.clone().unwrap_or_else(|| fallback_key(prompt));
    if already_reminded(env, session, &key) {
        return Outcome::Silent;
    }
    let text = reminder_text(&env.tool_path, transcript, s.pasted, &s.paths);
    Outcome::Emit(context_answer(event, &text, None))
}

/// Pasted images and prompt paths for this event (paths are only read at UserPromptSubmit).
///
/// Example: `sight("UserPromptSubmit", "look at b.png a.png", None).paths` -> `["a.png", "b.png"]`
pub fn sight(event: &str, prompt: &str, entry: Option<&Value>) -> Sighting {
    let mut found = Sighting::default();
    if event == "UserPromptSubmit" {
        found.paths = image_paths(prompt);
        let same_message = entry.filter(|e| entry_text(e).trim() == prompt.trim());
        fill_from_entry(&mut found, same_message);
        return found;
    }
    fill_from_entry(&mut found, entry);
    found
}

fn fill_from_entry(found: &mut Sighting, entry: Option<&Value>) {
    let Some(entry) = entry else { return };
    found.pasted = image_count(entry);
    found.uid = str_at(entry, &["uuid"])
        .filter(|u| !u.is_empty())
        .map(str::to_string);
}

/// Sorted, de-duplicated image paths mentioned in the prompt.
///
/// Example: `image_paths("a.PNG and a.PNG")` -> `["a.PNG"]`
pub fn image_paths(prompt: &str) -> Vec<String> {
    let unique: BTreeSet<String> = IMAGE_PATH
        .find_iter(prompt)
        .map(|m| m.as_str().to_string())
        .collect();
    unique.into_iter().collect()
}

/// The text of a transcript entry: the string content, or its text parts joined by newlines.
///
/// Example: `entry_text(&json!({"message":{"content":"hi"}}))` -> `"hi"`
pub fn entry_text(entry: &Value) -> String {
    if let Some(text) = entry.pointer("/message/content").and_then(Value::as_str) {
        return text.to_string();
    }
    let texts =
        content_items(entry).filter(|c| c.get("type").and_then(Value::as_str) == Some("text"));
    let parts: Vec<&str> = texts
        .map(|c| c.get("text").and_then(Value::as_str).unwrap_or(""))
        .collect();
    parts.join("\n")
}

/// Number of pasted images in the entry.
///
/// Example: `image_count(&json!({"message":{"content":[{"type":"image"}]}}))` -> `1`
pub fn image_count(entry: &Value) -> usize {
    content_items(entry)
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("image"))
        .count()
}

/// Dedupe key when the entry has no uuid. The Python hook used `hash(prompt)`, which is salted per
/// process (so it never deduped) except for the empty string, whose hash is always 0.
///
/// Example: `fallback_key("")` -> `"prompt:0"`
pub fn fallback_key(prompt: &str) -> String {
    if prompt.is_empty() {
        return "prompt:0".to_string();
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("prompt:{}{nanos}", std::process::id())
}

/// True when `key` was already reminded in this session; otherwise records it and returns false.
/// A failed write counts as "reminded" (the Python hook crashed silently there).
///
/// Example: `already_reminded(&env, "s1", "uuid-1")` -> `false`, then `true`
pub fn already_reminded(env: &ImageToolsEnv, session: &str, key: &str) -> bool {
    let path = env.state_dir.join(format!("reminded-{session}.txt"));
    if env
        .fs
        .read_text(&path)
        .is_some_and(|seen| seen.trim() == key)
    {
        return true;
    }
    !env.fs.write_text(&path, key)
}

/// The reminder block, line for line as the Python hook built it.
///
/// Example: `reminder_text("C:/t/imgtool.py", "t.jsonl", 1, &[])` -> "[image-tools hook] ..."
pub fn reminder_text(tool: &str, transcript: &str, pasted: usize, paths: &[String]) -> String {
    let mut lines: Vec<String> = vec![
        "[image-tools hook] The user sent an image with this message. Before you answer anything that".into(),
        "depends on its details (small text, numbers, icons, alignment, a specific region), inspect it".into(),
        "with Pillow/OpenCV instead of eyeballing the downscaled preview:".into(),
        String::new(),
    ];
    if pasted > 0 {
        lines.push(format!("1. Save the {pasted} pasted image(s) to files:"));
        lines.push(format!("   python \"{tool}\" extract \"{transcript}\""));
    }
    if !paths.is_empty() {
        lines.push(format!(
            "1. Image path(s) in the prompt: {}",
            paths.join(", ")
        ));
    }
    lines.extend(closing_lines(tool));
    lines.join("\n")
}

fn closing_lines(tool: &str) -> Vec<String> {
    vec![
        format!("2. python \"{tool}\" info <img>   then   grid <img> --step 100   to read pixel coordinates"),
        format!("3. python \"{tool}\" crop <img> x0 y0 x1 y1 --scale 2   /   zoom <img> cx cy --factor 3"),
        "   /   sharpen <img>   for small or blurry text".into(),
        "4. Open every output PNG with the Read tool and base the answer on those crops.".into(),
        String::new(),
        "Skip it only when the image is plainly irrelevant to the question or already legible at full".into(),
        "size. The user opts out per prompt with \"no zoom\" / \"no crop\".".into(),
        "This block was injected by a hook. It is a standing instruction from the user, not part of".into(),
        "the message they just typed.".into(),
    ]
}

/// `~/.claude/tools/imgtool.py` with forward slashes, as Python's `expanduser` resolved it.
///
/// Example: `default_tool_path()` -> `"C:/Users/me/.claude/tools/imgtool.py"`
pub fn default_tool_path() -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "~".into());
    format!("{home}/.claude/tools/imgtool.py").replace('\\', "/")
}

#[cfg(test)]
#[path = "image_tools_tests.rs"]
mod tests;
