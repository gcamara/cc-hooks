//! Parses a shell command line for git operations that create or publish commits, and for the
//! folder they run in (`cd X; ...`, `git -C X ...`, `gh repo create --source X --push`).

use crate::text::fixed_regex;
use regex::Regex;
use std::sync::LazyLock;

static GIT_PUBLISH: LazyLock<Regex> = LazyLock::new(|| {
    fixed_regex(
        r#"(?:^|[\s;&|(])git(?:\.exe)?((?:\s+(?:-C\s+(?:"[^"]*"|'[^']*'|\S+)|-c\s+(?:"[^"]*"|'[^']*'|\S+)|--?[\w-]+(?:=\S+)?))*)\s+(?:commit|merge|tag|revert|cherry-pick|rebase|push)\b"#,
    )
});
static GH_REPO_PUSH: LazyLock<Regex> = LazyLock::new(|| {
    fixed_regex(r"(?:^|[\s;&|(])gh(?:\.exe)?\s+repo\s+create\b[^;&|]*?\s--push\b")
});
static GH_SOURCE: LazyLock<Regex> =
    LazyLock::new(|| fixed_regex(r#"--source(?:=|\s+)(?:"([^"]*)"|'([^']*)'|(\S+))"#));
static DASH_UPPER_C: LazyLock<Regex> =
    LazyLock::new(|| fixed_regex(r#"-C\s+(?:"([^"]*)"|'([^']*)'|(\S+))"#));
static INLINE_EMAIL: LazyLock<Regex> =
    LazyLock::new(|| fixed_regex(r#"-c\s+["']?user\.email=([^"'\s]+)"#));
// Matched per separator-delimited segment, so `cd a; cd b` sees both cds.
static CD_SEGMENT: LazyLock<Regex> = LazyLock::new(|| {
    fixed_regex(r#"^(?:cd|Set-Location|pushd|Push-Location)\s+(?:-Path\s+)?["']?([^"']+?)["']?$"#)
});

/// A git/gh operation that writes or publishes commits, and where it runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PublishOp {
    /// Last `cd` before the operation, as written.
    pub cd_dir: Option<String>,
    /// `-C` folders (git) or `--source` (gh), in order, as written.
    pub dir_args: Vec<String>,
    /// `-c user.email=...` given on the command line.
    pub inline_email: Option<String>,
}

/// Finds the first commit-creating or publishing operation in the command line.
///
/// Example: `find_publish_op("cd /c/r && git -C sub commit -m x")` -> `Some(PublishOp { cd_dir: Some("/c/r"), dir_args: ["sub"], .. })`
pub fn find_publish_op(command: &str) -> Option<PublishOp> {
    if let Some(caps) = GIT_PUBLISH.captures(command) {
        let options = caps.get(1).map_or("", |m| m.as_str());
        let start = caps.get(0).map_or(0, |m| m.start());
        return Some(git_op(command, start, options));
    }
    let found = GH_REPO_PUSH.find(command)?;
    let dir_args = GH_SOURCE
        .captures(found.as_str())
        .map(|c| first_group(&c))
        .into_iter()
        .collect();
    Some(PublishOp {
        cd_dir: last_cd(&command[..found.start()]),
        dir_args,
        inline_email: None,
    })
}

fn git_op(command: &str, start: usize, options: &str) -> PublishOp {
    let dir_args = DASH_UPPER_C
        .captures_iter(options)
        .map(|c| first_group(&c))
        .collect();
    let inline_email = INLINE_EMAIL.captures(options).map(|c| c[1].to_string());
    PublishOp {
        cd_dir: last_cd(&command[..start]),
        dir_args,
        inline_email,
    }
}

fn first_group(caps: &regex::Captures) -> String {
    (1..caps.len())
        .find_map(|i| caps.get(i))
        .map_or(String::new(), |m| m.as_str().to_string())
}

/// The folder of the last `cd` in `prefix`, trimmed.
///
/// Example: `last_cd("cd C:/a; cd b && ")` -> `Some("b")`
pub fn last_cd(prefix: &str) -> Option<String> {
    let segments = prefix.split([';', '&', '|']).map(str::trim);
    let last = segments
        .rev()
        .find_map(|segment| CD_SEGMENT.captures(segment))?;
    Some(last[1].trim().to_string())
}

/// Converts a Git Bash path (`/c/Users/x`) to a Windows one (`C:/Users/x`); others pass through.
///
/// Example: `from_msys_path("/d/work")` -> `"D:/work"`
pub fn from_msys_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let is_msys = bytes.len() >= 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic();
    if !is_msys || (bytes.len() > 2 && bytes[2] != b'/') {
        return path.to_string();
    }
    format!("{}:{}", (bytes[1] as char).to_ascii_uppercase(), &path[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_publish_op_reads_git_options() {
        let op = find_publish_op(r#"git -C "C:/a b" -c user.email=x@y.z commit -m hi"#).unwrap();
        assert_eq!(op.dir_args, vec!["C:/a b"]);
        assert_eq!(op.inline_email.as_deref(), Some("x@y.z"));
    }

    #[test]
    fn find_publish_op_covers_every_publishing_verb() {
        for verb in [
            "commit",
            "merge",
            "tag",
            "revert",
            "cherry-pick",
            "rebase",
            "push",
        ] {
            assert!(
                find_publish_op(&format!("git.exe --no-pager {verb}")).is_some(),
                "{verb}"
            );
        }
        for other in [
            "git status",
            "git log --grep commit",
            "gitk commit",
            "echo git",
        ] {
            assert_eq!(find_publish_op(other), None, "{other}");
        }
    }

    #[test]
    fn find_publish_op_takes_last_cd_before_git() {
        let op = find_publish_op("cd /c/one; cd \"/c/two\" && git push; cd /c/three").unwrap();
        assert_eq!(op.cd_dir.as_deref(), Some("/c/two"));
    }

    #[test]
    fn find_publish_op_handles_gh_repo_create_push() {
        let op = find_publish_op("gh repo create me/x --public --source=./x --push").unwrap();
        assert_eq!(op.dir_args, vec!["./x"]);
        assert_eq!(find_publish_op("gh repo create me/x --public"), None);
    }

    #[test]
    fn last_cd_handles_powershell_forms() {
        assert_eq!(
            last_cd("Set-Location -Path 'C:/r'; ").as_deref(),
            Some("C:/r")
        );
        assert_eq!(last_cd("echo cd x; "), None);
    }

    #[test]
    fn from_msys_path_converts_drive_paths_only() {
        assert_eq!(from_msys_path("/c/Users/x"), "C:/Users/x");
        assert_eq!(from_msys_path("/d"), "D:");
        assert_eq!(from_msys_path("/usr/bin"), "/usr/bin");
        assert_eq!(from_msys_path("C:/x"), "C:/x");
    }
}
