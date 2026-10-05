//! `cc-hooks git-identity` (PreToolUse, Bash|PowerShell): denies creating or publishing commits in
//! a PERSONAL repo while the effective git e-mail is a WORK address.
//!
//! Why: the global user.email is the work one, and commits carrying it were pushed to a public
//! personal repo. Git is only spawned once a command matches, so every other command stays cheap.

use super::context_inject::is_under;
use super::git_command::{PublishOp, find_publish_op, from_msys_path};
use crate::config::GitIdentityConfig;
use crate::hook_io::{Outcome, deny_answer, str_at};
use crate::proc::ProcessRunner;
use serde_json::Value;
use std::path::Path;

const FALLBACK_EMAIL_HINT: &str = "<your-noreply-address>";

/// What the hook needs to know about a repository.
pub trait GitProbe {
    /// Top folder of the work tree containing `dir`, `None` when it is not a repo.
    fn toplevel(&self, dir: &str) -> Option<String>;
    /// Effective `user.email` (repo-local, else global).
    fn user_email(&self, repo: &str) -> Option<String>;
    /// `remote.origin.url`, `None` when there is no origin.
    fn origin_url(&self, repo: &str) -> Option<String>;
}

/// Asks the real git CLI.
pub struct CliGitProbe<'a> {
    pub runner: &'a dyn ProcessRunner,
}

impl CliGitProbe<'_> {
    fn git(&self, dir: &str, args: &[&str]) -> Option<String> {
        let mut full = vec!["-C".to_string(), dir.to_string()];
        full.extend(args.iter().map(|a| a.to_string()));
        self.runner
            .capture("git", &full)
            .filter(|out| !out.is_empty())
    }
}

impl GitProbe for CliGitProbe<'_> {
    fn toplevel(&self, dir: &str) -> Option<String> {
        self.git(dir, &["rev-parse", "--show-toplevel"])
    }

    fn user_email(&self, repo: &str) -> Option<String> {
        self.git(repo, &["config", "user.email"])
    }

    fn origin_url(&self, repo: &str) -> Option<String> {
        self.git(repo, &["remote", "get-url", "origin"])
    }
}

/// Denies the command when it publishes commits from a personal repo with a work e-mail.
///
/// Example: `run(&json!({"tool_input":{"command":"git push"},"cwd":"C:/me/app"}), Some(&cfg), &probe, "C:/")`
pub fn run(
    payload: &Value,
    config: Option<&GitIdentityConfig>,
    probe: &dyn GitProbe,
    process_cwd: &str,
) -> Outcome {
    let Some(config) = config else {
        return Outcome::Silent;
    };
    let command = str_at(payload, &["tool_input", "command"]).unwrap_or("");
    let Some(op) = find_publish_op(command) else {
        return Outcome::Silent;
    };
    let cwd = str_at(payload, &["cwd"])
        .filter(|c| !c.is_empty())
        .unwrap_or(process_cwd);
    let Some(repo) = probe.toplevel(&target_dir(&op, cwd)) else {
        return Outcome::Silent;
    };
    let origin = probe.origin_url(&repo);
    let email = op.inline_email.clone().or_else(|| probe.user_email(&repo));
    match email
        .filter(|e| is_personal(&repo, origin.as_deref(), config) && is_work_email(e, config))
    {
        Some(email) => Outcome::Emit(deny_answer(&deny_reason(
            &repo,
            origin.as_deref(),
            &email,
            config,
        ))),
        None => Outcome::Silent,
    }
}

/// The folder the operation runs in: cwd, then the last `cd`, then each `-C` / `--source`.
///
/// Example: `target_dir(&op_with_cd("/c/r"), "D:/x")` -> `"C:/r"`
pub fn target_dir(op: &PublishOp, cwd: &str) -> String {
    let steps = op.cd_dir.iter().chain(op.dir_args.iter());
    steps.fold(from_msys_path(cwd), |base, step| {
        join_dir(&base, &from_msys_path(step))
    })
}

fn join_dir(base: &str, step: &str) -> String {
    Path::new(base).join(step).to_string_lossy().into_owned()
}

/// Personal when origin matches a personal pattern, or there is no origin and the repo is not
/// under a work root.
///
/// Example: `is_personal("C:/me/app", None, &cfg)` -> `true` when no work root contains it
pub fn is_personal(repo: &str, origin: Option<&str>, config: &GitIdentityConfig) -> bool {
    match origin {
        Some(url) => contains_any(url, &config.personal_remote_patterns),
        None => !config.work_roots.iter().any(|root| is_under(repo, root)),
    }
}

/// True when the e-mail matches a configured work pattern (case-insensitive substring).
///
/// Example: `is_work_email("me@WORK.example", &cfg)` -> `true` for pattern "@work.example"
pub fn is_work_email(email: &str, config: &GitIdentityConfig) -> bool {
    contains_any(email, &config.work_email_patterns)
}

fn contains_any(text: &str, patterns: &[String]) -> bool {
    let text = text.to_lowercase();
    patterns
        .iter()
        .any(|p| !p.is_empty() && text.contains(&p.to_lowercase()))
}

/// The deny message: the e-mail found, the repo, and the one-line fix.
///
/// Example: `deny_reason("C:/me/app", None, "me@work.example", &cfg)` -> "BLOCKED by cc-hooks git-identity: ..."
pub fn deny_reason(
    repo: &str,
    origin: Option<&str>,
    email: &str,
    config: &GitIdentityConfig,
) -> String {
    let hint = config
        .personal_email
        .as_deref()
        .unwrap_or(FALLBACK_EMAIL_HINT);
    let origin = origin.unwrap_or("none");
    format!(
        "BLOCKED by cc-hooks git-identity: {repo} is a personal repo (origin: {origin}) but the commit e-mail is {email}, which matches a work e-mail pattern. Fix it in that repo first: git -C \"{repo}\" config user.email {hint} - then run the command again."
    )
}

#[cfg(test)]
#[path = "git_identity_tests.rs"]
mod tests;
