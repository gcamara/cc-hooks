//! Text helpers shared by the prompt classifiers (grill, design-mockup).

use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

static COMBINING_MARK: LazyLock<regex::Regex> = LazyLock::new(|| fixed_regex(r"\p{Mn}"));

// mockup / mock-up / mock up / mocked up / mocking up, plus "mock it up", "mock something up",
// "mock the radar page up". "set up" after the object is excluded, hence the lookbehind and
// fancy-regex (the `regex` crate has no lookaround).
static MOCKUP: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fixed_fancy(
        r"(?i)\bmock(ed|ing)?(-|\s)?ups?\b|\bmock\s+(it|this|that|them|something|(the|a|an|one)(\s+\w+){1,3})(?<!\bset)\s+up\b",
    )
});

/// Strips accents (NFD, then drop combining marks) so ASCII patterns also match Portuguese.
///
/// Example: `fold_accents("integração")` -> `"integracao"`
pub fn fold_accents(text: &str) -> String {
    let decomposed: String = text.nfd().collect();
    COMBINING_MARK.replace_all(&decomposed, "").into_owned()
}

/// True when an accent-folded prompt asks for a mockup (shared by grill and design-mockup).
///
/// Example: `is_mockup_request("mock the radar page up")` -> `true`
pub fn is_mockup_request(folded: &str) -> bool {
    MOCKUP.is_match(folded).unwrap_or(false)
}

/// Compiles a pattern that is a compile-time constant; a typo is a programming error.
///
/// Example: `static R: LazyLock<Regex> = LazyLock::new(|| fixed_regex(r"\bfix\b"));`
pub fn fixed_regex(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).unwrap_or_else(|e| panic!("bad pattern {pattern:?}: {e}"))
}

/// Like `fixed_regex`, for the few patterns that need lookaround (copied verbatim from .NET).
///
/// Example: `fixed_fancy(r"(?<!\.)claude").is_match("claude")` -> `Ok(true)`
pub fn fixed_fancy(pattern: &str) -> fancy_regex::Regex {
    fancy_regex::Regex::new(pattern).unwrap_or_else(|e| panic!("bad pattern {pattern:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_accents_strips_portuguese_marks() {
        assert_eq!(fold_accents("integração página é"), "integracao pagina e");
        assert_eq!(fold_accents("plain"), "plain");
    }

    #[test]
    fn is_mockup_request_matches_the_mockup_forms() {
        for prompt in [
            "a mockup",
            "mock-up",
            "mocked up",
            "mock it up",
            "MOCK the radar page UP",
        ] {
            assert!(is_mockup_request(prompt), "{prompt}");
        }
    }

    #[test]
    fn is_mockup_request_rejects_set_up_and_test_doubles() {
        assert!(!is_mockup_request("mock the radar set up"));
        assert!(!is_mockup_request("mock the service"));
        assert!(is_mockup_request("mock the reset up"));
    }

    #[test]
    fn fixed_regex_compiles_and_panics_on_typos() {
        assert!(fixed_regex(r"\bfix\b").is_match("a fix"));
        assert!(std::panic::catch_unwind(|| fixed_regex("(")).is_err());
    }

    #[test]
    fn fixed_fancy_supports_lookbehind() {
        let pattern = fixed_fancy(r"(?<![\w.])claude");
        assert!(pattern.is_match("by claude").unwrap());
        assert!(!pattern.is_match(".claude").unwrap());
    }
}
