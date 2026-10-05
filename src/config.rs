//! Machine-specific settings, read from `%USERPROFILE%\.claude\cc-hooks.toml`.
//!
//! Nothing personal lives in the source: roots, remotes, e-mail patterns, context files and script
//! paths all come from this file. A hook whose section is missing does nothing.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Env var that points at another config file (tests and the parity harness use it).
pub const CONFIG_ENV: &str = "CC_HOOKS_CONFIG";

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct Config {
    pub git_identity: Option<GitIdentityConfig>,
    pub quality_gate: Option<QualityGateConfig>,
    #[serde(default)]
    pub context_inject: Vec<ContextRule>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct GitIdentityConfig {
    #[serde(default)]
    pub work_roots: Vec<String>,
    #[serde(default)]
    pub personal_remote_patterns: Vec<String>,
    #[serde(default)]
    pub work_email_patterns: Vec<String>,
    /// Address suggested in the deny message, e.g. a GitHub noreply address.
    pub personal_email: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct QualityGateConfig {
    pub pre_script: Option<String>,
    pub post_script: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct ContextRule {
    pub cwd_prefix: String,
    pub file: String,
    pub header: String,
}

/// Parses config text; `None` (with no panic) when it is not valid TOML of the expected shape.
///
/// Example: `parse_config("[quality_gate]\npre_script = 'a.ps1'")` -> `Some(Config { .. })`
pub fn parse_config(text: &str) -> Option<Config> {
    toml::from_str(text).ok()
}

/// Loads the config from `CC_HOOKS_CONFIG` or the default path; `None` when absent or invalid.
///
/// Example: `load_config().and_then(|c| c.quality_gate)`
pub fn load_config() -> Option<Config> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    parse_config(text.trim_start_matches('\u{FEFF}'))
}

/// Where the config is read from.
///
/// Example: `config_path()` -> `Some("C:\\Users\\me\\.claude\\cc-hooks.toml")`
pub fn config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var(CONFIG_ENV) {
        return Some(PathBuf::from(explicit));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(Path::new(&home).join(".claude").join("cc-hooks.toml"))
}

/// Expands `%NAME%` references from the environment; unknown names are left as they are.
///
/// Example: `expand_env("%USERPROFILE%\\x")` -> `"C:\\Users\\me\\x"`
pub fn expand_env(raw: &str) -> String {
    expand_with(raw, |name| std::env::var(name).ok())
}

fn expand_with(raw: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(open) = rest.find('%') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('%') else { break };
        out.push_str(&rest[..open]);
        let name = &after[..close];
        match lookup(name) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[git_identity]
work_roots = ["X:/work"]
personal_remote_patterns = ["github.com/someone/"]
work_email_patterns = ["@work.example"]

[quality_gate]
pre_script = "C:/gate/pre.ps1"

[[context_inject]]
cwd_prefix = "X:/work"
file = "%HOME%/ctx.md"
header = "Header:\n\n"
"#;

    #[test]
    fn parse_config_reads_every_section() {
        let config = parse_config(SAMPLE).unwrap();
        let git = config.git_identity.unwrap();
        assert_eq!(git.work_email_patterns, vec!["@work.example"]);
        assert_eq!(config.quality_gate.unwrap().post_script, None);
        assert_eq!(config.context_inject[0].header, "Header:\n\n");
    }

    #[test]
    fn parse_config_rejects_wrong_shape() {
        assert_eq!(parse_config("context_inject = 3"), None);
        assert_eq!(parse_config(""), Some(Config::default()));
    }

    #[test]
    fn config_path_ends_in_claude_folder() {
        let path = config_path().unwrap();
        assert!(path.to_string_lossy().ends_with("cc-hooks.toml"));
    }

    #[test]
    fn load_config_never_panics() {
        let _ = load_config();
    }

    #[test]
    fn expand_with_replaces_known_names_only() {
        let lookup = |name: &str| (name == "A").then(|| "1".to_string());
        assert_eq!(expand_with("%A%/x/%B%/%A%", lookup), "1/x/%B%/1");
        assert_eq!(expand_with("50% off", lookup), "50% off");
    }

    #[test]
    fn expand_env_uses_process_environment() {
        let expected = std::env::var("PATH").unwrap();
        assert_eq!(expand_env("%PATH%"), expected);
    }
}
