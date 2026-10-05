use super::*;
use crate::proc::fakes::RecordingProcessRunner;
use serde_json::json;
use std::collections::HashMap;

/// A repo layout keyed by folder: (toplevel, email, origin).
#[derive(Default)]
struct FakeGitProbe {
    repos: HashMap<String, (String, Option<String>, Option<String>)>,
}

impl FakeGitProbe {
    fn with_repo(mut self, dir: &str, email: Option<&str>, origin: Option<&str>) -> Self {
        let entry = (
            dir.to_string(),
            email.map(String::from),
            origin.map(String::from),
        );
        self.repos.insert(dir.to_string(), entry);
        self
    }

    fn repo(&self, dir: &str) -> Option<&(String, Option<String>, Option<String>)> {
        self.repos.get(&dir.replace('\\', "/"))
    }
}

impl GitProbe for FakeGitProbe {
    fn toplevel(&self, dir: &str) -> Option<String> {
        self.repo(dir).map(|r| r.0.clone())
    }

    fn user_email(&self, repo: &str) -> Option<String> {
        self.repo(repo).and_then(|r| r.1.clone())
    }

    fn origin_url(&self, repo: &str) -> Option<String> {
        self.repo(repo).and_then(|r| r.2.clone())
    }
}

fn config() -> GitIdentityConfig {
    GitIdentityConfig {
        work_roots: vec!["X:/work".into()],
        personal_remote_patterns: vec!["github.com/someone/".into(), "github.com:someone/".into()],
        work_email_patterns: vec!["@work.example".into()],
        personal_email: Some("someone@users.noreply.example".into()),
    }
}

fn command_in(cmd: &str, cwd: &str) -> Value {
    json!({"tool_input": {"command": cmd}, "cwd": cwd})
}

fn is_deny(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Emit(v) if v["hookSpecificOutput"]["permissionDecision"] == "deny")
}

#[test]
fn run_denies_work_email_in_personal_origin_repo() {
    let origin = Some("git@github.com:someone/app.git");
    let probe = FakeGitProbe::default().with_repo("C:/me/app", Some("me@work.example"), origin);
    let outcome = run(
        &command_in("git push", "C:/me/app"),
        Some(&config()),
        &probe,
        "C:/",
    );
    assert!(is_deny(&outcome));
}

#[test]
fn run_denies_no_origin_repo_outside_work_roots_via_cd() {
    let probe = FakeGitProbe::default().with_repo("C:/me/new", Some("Me@Work.Example"), None);
    let cmd = "cd /c/me/new && git commit -m init";
    assert!(is_deny(&run(
        &command_in(cmd, "X:/work"),
        Some(&config()),
        &probe,
        "C:/"
    )));
}

#[test]
fn run_allows_work_repos_other_remotes_and_personal_email() {
    let probe = FakeGitProbe::default()
        .with_repo("X:/work/svc", Some("me@work.example"), None)
        .with_repo(
            "C:/me/fork",
            Some("me@work.example"),
            Some("https://github.com/other/x"),
        )
        .with_repo(
            "C:/me/app",
            Some("someone@users.noreply.example"),
            Some("https://github.com/someone/app"),
        );
    for (cmd, cwd) in [
        ("git commit", "X:/work/svc"),
        ("git push", "C:/me/fork"),
        ("git push", "C:/me/app"),
    ] {
        assert_eq!(
            run(&command_in(cmd, cwd), Some(&config()), &probe, "C:/"),
            Outcome::Silent,
            "{cwd}"
        );
    }
}

#[test]
fn run_uses_inline_email_and_dash_c_dir() {
    let probe =
        FakeGitProbe::default().with_repo("C:/me/app", Some("ok@users.noreply.example"), None);
    let cmd = "git -C app -c user.email=me@work.example commit -m x";
    assert!(is_deny(&run(
        &command_in(cmd, "C:/me"),
        Some(&config()),
        &probe,
        "C:/"
    )));
}

#[test]
fn run_is_silent_without_config_match_or_repo() {
    let probe = FakeGitProbe::default();
    assert_eq!(
        run(&command_in("git push", "C:/x"), None, &probe, "C:/"),
        Outcome::Silent
    );
    assert_eq!(
        run(
            &command_in("git status", "C:/x"),
            Some(&config()),
            &probe,
            "C:/"
        ),
        Outcome::Silent
    );
    assert_eq!(
        run(
            &command_in("git push", "C:/not-a-repo"),
            Some(&config()),
            &probe,
            "C:/"
        ),
        Outcome::Silent
    );
}

#[test]
fn target_dir_applies_cd_then_dash_c() {
    let op = PublishOp {
        cd_dir: Some("/c/me".into()),
        dir_args: vec!["app".into()],
        inline_email: None,
    };
    assert_eq!(target_dir(&op, "D:/x").replace('\\', "/"), "C:/me/app");
    assert_eq!(target_dir(&PublishOp::default(), "/d/r"), "D:/r");
}

#[test]
fn is_personal_prefers_origin_over_path() {
    assert!(is_personal(
        "X:/work/a",
        Some("https://github.com/someone/a"),
        &config()
    ));
    assert!(!is_personal(
        "C:/me/a",
        Some("https://github.com/other/a"),
        &config()
    ));
    assert!(is_personal("C:/me/a", None, &config()));
    assert!(!is_personal("x:\\WORK\\a", None, &config()));
}

#[test]
fn is_work_email_is_case_insensitive() {
    assert!(is_work_email("ME@WORK.EXAMPLE", &config()));
    assert!(!is_work_email("me@home.example", &config()));
}

#[test]
fn deny_reason_names_email_repo_and_fix() {
    let reason = deny_reason("C:/me/app", None, "me@work.example", &config());
    assert!(reason.contains("me@work.example") && reason.contains("origin: none"));
    assert!(
        reason.contains("git -C \"C:/me/app\" config user.email someone@users.noreply.example")
    );
    let bare = GitIdentityConfig {
        personal_email: None,
        ..config()
    };
    assert!(deny_reason("r", Some("u"), "e", &bare).contains(FALLBACK_EMAIL_HINT));
}

#[test]
fn cli_git_probe_builds_dash_c_invocations() {
    let mut runner = RecordingProcessRunner::default();
    runner.captures.insert(
        "git -C C:/r remote get-url origin".into(),
        "https://x/y".into(),
    );
    let probe = CliGitProbe { runner: &runner };
    assert_eq!(probe.origin_url("C:/r").as_deref(), Some("https://x/y"));
    assert_eq!(probe.user_email("C:/r"), None);
    assert_eq!(probe.toplevel("C:/r"), None);
    assert_eq!(runner.calls.borrow().len(), 3);
}
