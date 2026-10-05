//! Does a shell command create a pull request? Port of the quality gate's pr-create-detect.ps1:
//! `gh pr create`, a REST POST to repos/o/r/pulls (gh api, curl, Invoke-RestMethod, fetch), or a
//! local script (up to 1 MB) that does one.
//!
//! PowerShell `-match` is case-insensitive, so those patterns carry (?i); the script-path pattern
//! went through `[regex]::Matches`, which is case-sensitive, so it does not.

use crate::fs_port::FileSystem;
use crate::text::fixed_fancy;
use fancy_regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

static GH_PR_CREATE: LazyLock<Regex> =
    LazyLock::new(|| fixed_fancy(r#"(?i)gh(\.exe)?["']?\s+pr\s+create"#));
static GH_API: LazyLock<Regex> = LazyLock::new(|| fixed_fancy(r#"(?i)gh(\.exe)?["']?\s+api\b"#));
static HTTP_CLIENT: LazyLock<Regex> = LazyLock::new(|| {
    fixed_fancy(r"(?i)\bcurl(\.exe)?\b|Invoke-RestMethod|Invoke-WebRequest|\bfetch\s*\(")
});
// The create endpoint and nothing after it: /pulls/12 (an update or a comment) does not count.
static PULLS_ENDPOINT: LazyLock<Regex> =
    LazyLock::new(|| fixed_fancy(r#"(?i)repos/[^/\s"']+/[^/\s"']+/pulls(?![/\w])"#));
// An explicit POST, or a flag that makes gh api / curl send a body (gh api then defaults to POST).
static POST_HINT: LazyLock<Regex> = LazyLock::new(|| {
    fixed_fancy(
        r#"(?i)(-X|--method|--request|-Method)[\s=:]+["']?POST\b|\s(-f|-F|--field|--raw-field|--input|-d|--data[\w-]*)\s"#,
    )
});
static SCRIPT_PATH: LazyLock<Regex> = LazyLock::new(|| {
    fixed_fancy(r#"(?:[A-Za-z]:)?[^\s"'<>|;&]*\.(ps1|sh|bash|js|mjs|cjs|py)(?=["']?(\s|$|;|&|\|))"#)
});

fn found(pattern: &Regex, text: &str) -> bool {
    pattern.is_match(text).unwrap_or(false)
}

/// True when the command opens a PR.
///
/// Example: `is_pr_create("gh api -X POST repos/o/r/pulls --input pr.json", "C:/repo", &fs)` -> `true`
pub fn is_pr_create(command: &str, cwd: &str, fs: &dyn FileSystem) -> bool {
    if command.is_empty() {
        return false;
    }
    if found(&GH_PR_CREATE, command) || is_api_pr_create(command) {
        return true;
    }
    script_paths(command, cwd, fs)
        .iter()
        .any(|path| script_creates_pr(path, fs))
}

/// A REST create call: the pulls endpoint, a client that can POST, and a POST.
///
/// Example: `is_api_pr_create("curl -X POST https://api.github.com/repos/o/r/pulls")` -> `true`
pub fn is_api_pr_create(text: &str) -> bool {
    if !found(&PULLS_ENDPOINT, text) {
        return false;
    }
    if !found(&GH_API, text) && !found(&HTTP_CLIENT, text) {
        return false;
    }
    found(&POST_HINT, text)
}

/// Local script files the command runs, resolved against `cwd`; missing or huge files are skipped.
///
/// Example: `script_paths("pwsh ./pr.ps1", "C:/repo", &fs)` -> `["C:/repo/./pr.ps1"]` when it exists
pub fn script_paths(command: &str, cwd: &str, fs: &dyn FileSystem) -> Vec<PathBuf> {
    let candidates = SCRIPT_PATH.find_iter(command).filter_map(Result::ok);
    let resolved = candidates.map(|m| resolve(m.as_str(), cwd));
    resolved
        .filter(|path| {
            fs.file_size(path)
                .is_some_and(|size| size <= MAX_SCRIPT_BYTES)
        })
        .collect()
}

fn resolve(raw: &str, cwd: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.has_root() || cwd.is_empty() {
        return path.to_path_buf();
    }
    Path::new(cwd).join(path)
}

fn script_creates_pr(path: &Path, fs: &dyn FileSystem) -> bool {
    fs.read_text(path)
        .is_some_and(|content| is_api_pr_create(&content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_port::fakes::FakeFileSystem;

    #[test]
    fn is_pr_create_detects_gh_pr_create() {
        let fs = FakeFileSystem::default();
        assert!(is_pr_create("cd x; gh pr create -t a", "", &fs));
        assert!(is_pr_create("\"gh.exe\" PR  Create", "", &fs));
        assert!(!is_pr_create("gh pr view 3", "", &fs));
        assert!(!is_pr_create("", "", &fs));
    }

    #[test]
    fn is_api_pr_create_needs_endpoint_client_and_post() {
        assert!(is_api_pr_create("gh api repos/o/r/pulls -f title=x "));
        assert!(is_api_pr_create(
            "Invoke-RestMethod -Method POST https://api.github.com/repos/o/r/pulls"
        ));
        assert!(!is_api_pr_create("gh api repos/o/r/pulls/12 -X POST"));
        assert!(!is_api_pr_create("gh api repos/o/r/pulls"));
        assert!(!is_api_pr_create("echo -X POST repos/o/r/pulls"));
    }

    #[test]
    fn script_paths_resolves_relative_and_skips_big_or_missing() {
        let big = vec![b'x'; (MAX_SCRIPT_BYTES + 1) as usize];
        let fs = FakeFileSystem::default()
            .with_file("C:/repo\\pr.ps1", b"x")
            .with_file("C:/repo\\big.py", &big);
        let paths = script_paths("pwsh pr.ps1; python big.py; node gone.js", "C:/repo", &fs);
        assert_eq!(paths, vec![PathBuf::from("C:/repo\\pr.ps1")]);
    }

    #[test]
    fn script_paths_is_case_sensitive_on_extension() {
        let fs = FakeFileSystem::default().with_file("C:/s/PR.PS1", b"x");
        assert!(script_paths("pwsh C:/s/PR.PS1", "", &fs).is_empty());
    }

    #[test]
    fn is_pr_create_reads_script_bodies() {
        let body = b"gh api -X POST repos/o/r/pulls --input b.json";
        let fs = FakeFileSystem::default().with_file("C:/s/open-pr.ps1", body);
        assert!(is_pr_create(
            "powershell -File C:/s/open-pr.ps1",
            "D:/x",
            &fs
        ));
    }

    #[test]
    fn resolve_keeps_rooted_paths() {
        assert_eq!(resolve("C:/a.ps1", "D:/x"), PathBuf::from("C:/a.ps1"));
        assert_eq!(resolve("a.ps1", ""), PathBuf::from("a.ps1"));
    }
}
