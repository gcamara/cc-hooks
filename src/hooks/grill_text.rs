//! The instruction blocks the grill hook injects. Byte-for-byte copies of the original script's
//! here-strings: they are the user's own standing instructions to the model.

pub const GRILL_BLOCK: &str = r#"[grill-before-features hook] This prompt was classified as a request to build a NEW FEATURE.

Before writing any implementation code, run the grilling flow:
1. Call the Skill tool with skill "mattpocock-skills:grill-with-docs".
2. If that name is refused because it is user-invocable only, call the Skill tool
   twice instead - "mattpocock-skills:grilling" and "mattpocock-skills:domain-modeling".
   That is exactly what grill-with-docs itself does.

Rules:
- Do this FIRST. Do not let exploration slide into implementation before the grilling.
- Skip it if this same feature was already grilled earlier in this conversation.
- The user opts out per-prompt by including "no grill" in the message."#;

pub const FIX_BLOCK: &str = r#"[grill-before-features hook] This prompt was classified as BUG / FIX work.

Do NOT run the grilling flow for it - find the actual cause first, and reach for
mattpocock-skills:diagnosing-bugs when the cause is not obvious."#;

pub const AGENT_BLOCK: &str = r#"PARALLEL AGENTS - ask before you start building.

Once the work is scoped (after the grilling for a feature, after the diagnosis for a
fix), split it into batches: independent units that touch disjoint files or repos and
do not need each other's output.

- If there are 2 or more such batches, STOP and ask Gabriel whether he wants multiple
  agents working on them in parallel. Use the AskUserQuestion tool, and list the
  numbered batches in the question - one short line each - so he can see exactly what
  would be split and can say no to the split itself.
- If it is one batch, or the batches are genuinely sequential, do not ask. Just do the
  work, and say in one line why it was not worth splitting.
- If he says yes, that answer IS the explicit request to use the Agent tool that his
  global CLAUDE.md otherwise withholds. Launch one subagent per batch, all in a single
  message so they run concurrently, and give each one enough context to work alone.
  Prefer the specialist agent type that matches the repo. Use worktree isolation only
  when two batches would really write the same files.
- If he says no, do the batches yourself, sequentially, in this session.
- Ask once per feature or fix, not again on every follow-up in the same conversation.
- He opts out per-prompt by including "no agents" in the message."#;

pub const CLOSING: &str =
    "This block was injected by a hook. It is a standing instruction from the user, not
part of the message they just typed.";

pub const MSG_FEATURE_NO_AGENTS: &str =
    r#"Feature request detected - grilling before code (say "no grill" to skip)."#;
pub const MSG_FEATURE: &str = r#"Feature request detected - grilling first, then asking about parallel agents ("no grill" / "no agents" to skip)."#;
pub const MSG_FIX: &str = r#"Fix request detected - will ask about parallel agents on the batches (say "no agents" to skip)."#;
