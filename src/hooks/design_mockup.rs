//! `cc-hooks design-mockup` (UserPromptSubmit): when the prompt asks for a mockup, route the turn
//! through the Design canvas instead of a hand-written HTML page or an inline widget.

use crate::hook_io::{Outcome, context_answer, str_at};
use crate::text::{fixed_regex, fold_accents, is_mockup_request};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static OPT_OUT: LazyLock<Regex> =
    LazyLock::new(|| fixed_regex(r"(?i)\b(no|skip|sem|without|dont|do not)\s+(design|canvas)\b"));
// "mock" in a test-double sense is not a mockup.
static TEST_DOUBLE: LazyLock<Regex> = LazyLock::new(|| {
    fixed_regex(r"(?i)\b(jest|vitest|mockito|spec|unit tests?|test files?|stubs?)\b")
});

const MESSAGE: &str =
    r#"Mockup request detected - routing to the Design canvas (say "no design" to skip)."#;

const BLOCK: &str = r#"[design-on-mockup hook] This prompt asks for a MOCKUP.

Build it as a Design canvas, never as a hand-written HTML page or an inline widget:
1. If a skill named "design" is in the skill listing, call the Skill tool with
   skill "design" and the brief as args, and follow it.
2. Otherwise the canvas is the Artifact tool's Design type: call Artifact with
   action "quickstart" and intent "design" FIRST, use the design system it marks
   as default (ask only if several are listed and none is default), then publish
   with the Design type_url and fill its artboards as the type instructs.

Research FIRST, before any artboard and before picking a palette or type:
3. Look at real, current designs for this kind of screen. Search at least two of
   Dribbble, Behance, Mobbin, Higgsfield and similar design references (browser tools;
   Dribbble/Behance need no login) for the app's category (e.g. "security camera app
   dark ui", "smart home camera grid"). Open 4-8 shots, and write down in the reply,
   in a few lines, what the good ones share: palette (real hex values you sampled or
   read), type treatment, layout/density, what they do with status and imagery.
   Derive the look from that evidence - never from the default dark-slate + blue
   accent Claude reaches for on its own (Gabriel hates it; see the
   "no-default-blue-dark-theme" memory).

Design passes (designer-skills plugins), around the artboard writing:
4. Before writing any artboard, call the Skill tool with "ui-design:visual-hierarchy"
   and lay the screen out by it: one focal point, a deliberate reading order, size,
   weight, contrast and position doing the ranking, not decoration.
5. After the artboard files are written and BEFORE the single publish, call the Skill
   tool with "visual-critique:critique-screen" against the drafted artboards, apply
   the fixes it ranks as high, and only then publish. Do not publish a first draft
   and republish after the critique.
   If either skill is not in the listing (plugin not loaded this session), skip that
   step and say so in one line.

Rules:
- Do this before any exploration or file writing for the mockup itself.
- One canvas per mockup request; a follow-up edits the same canvas by its url.
- A follow-up that only changes copy or moves one element skips the research and
  the two passes; a follow-up that changes the LOOK repeats all three.
- The user opts out per-prompt with "no design" or "no canvas"; "no research"
  skips step 3 only.

This block was injected by a hook. It is a standing instruction from the user, not
part of the message they just typed."#;

/// Injects the Design-canvas instructions when the prompt asks for a mockup.
///
/// Example: `run(&json!({"prompt":"mock up the camera grid"}))` -> context with the design block
pub fn run(payload: &Value) -> Outcome {
    match str_at(payload, &["prompt"]) {
        Some(prompt) if asks_for_mockup(prompt) => {
            Outcome::Emit(context_answer("UserPromptSubmit", BLOCK, Some(MESSAGE)))
        }
        _ => Outcome::Silent,
    }
}

/// True for a mockup request that is not a slash command, an opt-out, or test-double talk.
///
/// Example: `asks_for_mockup("mock the radar page up")` -> `true`
pub fn asks_for_mockup(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }
    let norm = fold_accents(trimmed);
    if OPT_OUT.is_match(&norm) || TEST_DOUBLE.is_match(&norm) {
        return false;
    }
    is_mockup_request(&norm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn asks_for_mockup_detects_mockup_wording() {
        assert!(asks_for_mockup("can you do a mock-up of the home screen"));
        assert!(asks_for_mockup("faz um mockup da tela"));
    }

    #[test]
    fn asks_for_mockup_respects_opt_outs() {
        for prompt in [
            "/design mockup",
            "mockup, no design",
            "mock up the jest module",
            "  ",
        ] {
            assert!(!asks_for_mockup(prompt), "{prompt}");
        }
    }

    #[test]
    fn run_emits_block_and_message() {
        let Outcome::Emit(answer) = run(&json!({"prompt": "mock it up"})) else {
            panic!()
        };
        assert_eq!(answer["systemMessage"], MESSAGE);
        assert_eq!(answer["hookSpecificOutput"]["additionalContext"], BLOCK);
        assert_eq!(run(&json!({})), Outcome::Silent);
    }
}
