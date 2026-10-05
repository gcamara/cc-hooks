//! `cc-hooks grill` (UserPromptSubmit). Two independent interceptors:
//!   1. NEW FEATURE -> route the turn through the grilling skill before any code, then offer to
//!      fan the batches out across multiple agents.
//!   2. BUG / FIX   -> no grilling (that is the diagnosing skill's job), but still offer the agents.

use super::grill_text::*;
use crate::hook_io::{Outcome, context_answer, str_at};
use crate::text::{fixed_regex, fold_accents, is_mockup_request};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

/// The patterns, compiled once, in the order the classifier needs them.
struct Patterns {
    skip_agents: Regex,
    skip_grill: Regex,
    question: Regex,
    fixish: Regex,
    verb: Regex,
    noun: Regex,
    novelty: Regex,
    trivial: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    skip_agents: fixed_regex(
        r"(?i)(\b(no|skip|sem|without|dont|do not)\s+(agents?|subagents?|agentes?|parallel|paralelo)\b|\b(single|one)\s+agent\b|\bsozinho\b)",
    ),
    skip_grill: fixed_regex(r"(?i)\b(no|skip|sem|without|dont|do not)\s+grill"),
    question: fixed_regex(
        r"(?i)^\s*(what|why|how|where|when|which|who|does|do|is|are|can|could|should|would|explain|show|list|find|check|read|review|look|tell|o que|por que|porque|como|onde|quando|qual|quem)\b",
    ),
    fixish: fixed_regex(
        r"(?i)\b(bug|broken|failing|fails|crash|regression|stacktrace|stack trace|typo|revert|rollback|hotfix|debug|diagnose|fix|fixes|fixing|repair|patch|erro|quebrado|falhando|corrige|corrigir|conserta|consertar|arruma|arrumar)\b",
    ),
    verb: fixed_regex(
        r"(?i)\b(add|build|create|implement|develop|ship|introduce|scaffold|design|wire up|set up|implementa|implementar|cria|criar|adiciona|adicionar|constroi|construir|desenvolve|desenvolver)\b",
    ),
    noun: fixed_regex(
        r"(?i)\b(feature|functionality|capability|endpoint|api|page|screen|view|component|module|flow|workflow|integration|dashboard|form|report|command|service|system|wizard|step|section|panel|widget|tab|modal|chart|table|column|field|filter|toggle|notification|job|worker|migration|setting|funcionalidade|tela|pagina|componente|modulo|fluxo|integracao|relatorio|formulario|servico|coluna|campo|etapa|secao)s?\b",
    ),
    novelty: fixed_regex(r"(?i)\b(new|another|nova|novo)\b"),
    trivial: fixed_regex(
        r"(?i)\b(log|logging|comment|test|tests|branch|commit|import|variable|constant|type|interface|todo|alias|env var)s?\b",
    ),
});

/// How a prompt was classified.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PromptKind {
    Feature { skip_agents: bool },
    Fix,
}

/// Injects the grilling / agents instructions for feature and fix prompts.
///
/// Example: `run(&json!({"prompt":"add a new settings page"}))` -> context with the grill block
pub fn run(payload: &Value) -> Outcome {
    let Some(prompt) = str_at(payload, &["prompt"]) else {
        return Outcome::Silent;
    };
    match classify(prompt) {
        Some(kind) => Outcome::Emit(answer_for(kind)),
        None => Outcome::Silent,
    }
}

/// Feature, fix, or `None` (pass through untouched).
///
/// Example: `classify("fix the login crash")` -> `Some(PromptKind::Fix)`
pub fn classify(prompt: &str) -> Option<PromptKind> {
    let trimmed = prompt.trim();
    // An explicit slash command already says what the user wants. Never hijack it.
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    let norm = fold_accents(trimmed);
    let p = &*PATTERNS;
    let skip_agents = p.skip_agents.is_match(&norm);
    // Mockups belong to design-mockup; questions are neither feature nor fix work.
    if p.skip_grill.is_match(&norm) || is_mockup_request(&norm) || p.question.is_match(&norm) {
        return None;
    }
    if p.fixish.is_match(&norm) {
        // A fix prompt with no agents question has nothing left to inject.
        return (!skip_agents).then_some(PromptKind::Fix);
    }
    is_feature(&norm).then_some(PromptKind::Feature { skip_agents })
}

/// A build verb plus a feature noun, or a build verb plus "new X" where X is not a trivial edit.
///
/// Example: `is_feature("add a new endpoint")` -> `true`
pub fn is_feature(norm: &str) -> bool {
    let p = &*PATTERNS;
    if !p.verb.is_match(norm) {
        return false;
    }
    p.noun.is_match(norm) || (p.novelty.is_match(norm) && !p.trivial.is_match(norm))
}

/// The hook answer for a classified prompt.
///
/// Example: `answer_for(PromptKind::Fix)["systemMessage"]` -> the fix message
pub fn answer_for(kind: PromptKind) -> Value {
    let (parts, message): (Vec<&str>, &str) = match kind {
        PromptKind::Feature { skip_agents: true } => (vec![GRILL_BLOCK], MSG_FEATURE_NO_AGENTS),
        PromptKind::Feature { skip_agents: false } => (vec![GRILL_BLOCK, AGENT_BLOCK], MSG_FEATURE),
        PromptKind::Fix => (vec![FIX_BLOCK, AGENT_BLOCK], MSG_FIX),
    };
    let mut blocks = parts;
    blocks.push(CLOSING);
    context_answer("UserPromptSubmit", &blocks.join("\n\n"), Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_feature_with_and_without_agents() {
        let with_agents = PromptKind::Feature { skip_agents: false };
        assert_eq!(classify("add a new settings page"), Some(with_agents));
        let solo = PromptKind::Feature { skip_agents: true };
        assert_eq!(classify("build the export endpoint, no agents"), Some(solo));
    }

    #[test]
    fn classify_fix_wins_over_feature() {
        assert_eq!(
            classify("add a fix for the broken page"),
            Some(PromptKind::Fix)
        );
        assert_eq!(classify("fix the crash sem agentes"), None);
    }

    #[test]
    fn classify_passes_through_slash_questions_mockups_and_opt_out() {
        for prompt in [
            "/review add page",
            "what does the api page do",
            "mock it up",
            "add page no grill",
            "  ",
        ] {
            assert_eq!(classify(prompt), None, "{prompt}");
        }
    }

    #[test]
    fn classify_folds_portuguese_accents() {
        let expected = Some(PromptKind::Feature { skip_agents: false });
        assert_eq!(classify("cria uma integração"), expected);
        assert_eq!(classify("corrigir a página"), Some(PromptKind::Fix));
    }

    #[test]
    fn is_feature_needs_verb_and_noun_or_nontrivial_novelty() {
        assert!(is_feature("add a new endpoint"));
        assert!(is_feature("create another thing"));
        assert!(!is_feature("add a new test"));
        assert!(!is_feature("the new dashboard"));
    }

    #[test]
    fn answer_for_joins_blocks_with_blank_lines() {
        let answer = answer_for(PromptKind::Fix);
        assert_eq!(answer["systemMessage"], MSG_FIX);
        let context = answer["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert_eq!(
            context,
            format!("{FIX_BLOCK}\n\n{AGENT_BLOCK}\n\n{CLOSING}")
        );
    }

    #[test]
    fn run_reads_prompt_field() {
        assert_eq!(run(&json!({"prompt": 3})), Outcome::Silent);
        assert!(matches!(
            run(&json!({"prompt": "implement the report"})),
            Outcome::Emit(_)
        ));
    }
}
