//! Process plumbing and reporting for the parity harness.

mod cases;
mod special;

pub use cases::*;
pub use special::*;

use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Scratch folder, script locations and the binary under test.
pub struct Lab {
    pub dir: PathBuf,
    pub hooks_dir: PathBuf,
    pub bin: PathBuf,
}

impl Lab {
    pub fn new() -> Lab {
        let dir = std::env::temp_dir().join(format!("cc-hooks-parity-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("tmp")).expect("scratch dir");
        let home = std::env::var("USERPROFILE").expect("USERPROFILE");
        let hooks_dir = PathBuf::from(home).join(".claude").join("hooks");
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_cc-hooks"));
        Lab {
            dir,
            hooks_dir,
            bin,
        }
    }

    /// Writes a scratch file and returns its path with forward slashes.
    pub fn write(&self, name: &str, body: &[u8]) -> String {
        let path = self.dir.join(name);
        std::fs::write(&path, body).expect("write scratch file");
        path.to_string_lossy().replace('\\', "/")
    }

    /// Env every child gets: a private TEMP so dedupe state never touches the real one.
    pub fn base_env(&self) -> Vec<(String, String)> {
        let tmp = self.dir.join("tmp").to_string_lossy().into_owned();
        vec![("TMP".into(), tmp.clone()), ("TEMP".into(), tmp)]
    }
}

/// One corpus entry. `@SESSION@` in the input is replaced per side so dedupe files never collide.
pub struct Case {
    pub name: String,
    pub input: Vec<u8>,
    pub known_diff: Option<&'static str>,
    pub accent: bool,
    pub repeat: bool,
}

impl Case {
    pub fn new(name: &str, input: &str) -> Case {
        Case {
            name: name.into(),
            input: input.as_bytes().to_vec(),
            known_diff: None,
            accent: false,
            repeat: false,
        }
    }

    pub fn known(mut self, why: &'static str) -> Case {
        self.known_diff = Some(why);
        self
    }

    pub fn accented(mut self) -> Case {
        self.accent = true;
        self
    }

    pub fn twice(mut self) -> Case {
        self.repeat = true;
        self
    }

    pub fn bom_prefixed(mut self) -> Case {
        self.input.splice(0..0, [0xEF, 0xBB, 0xBF]);
        self
    }

    pub fn with_bom(name: &str, input: &str) -> Case {
        Case::new(name, input)
            .bom_prefixed()
            .known("old script cannot parse a BOM; cc-hooks strips it")
    }

    fn input_for(&self, side: &str) -> Vec<u8> {
        let text = String::from_utf8_lossy(&self.input)
            .replace("@SESSION@", &format!("{side}-{}", self.name));
        text.into_bytes()
    }
}

/// Per-hook summary line.
#[derive(Default)]
pub struct Row {
    pub hook: String,
    pub cases: usize,
    pub matched: usize,
    pub known: usize,
    pub mismatch: usize,
    pub raw_mode_diffs: usize,
}

/// Runs a program with stdin and env, returning its stdout.
pub fn spawn(program: &str, args: &[String], input: &[u8], env: &[(String, String)]) -> String {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.envs(env.iter().map(|(k, v)| (k, v)));
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start {program}: {e}"));
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Parses an answer; empty stdout is `None`, anything that is not JSON becomes a marker value.
pub fn parse_answer(stdout: &str) -> Option<Value> {
    let text = stdout.trim();
    if text.is_empty() {
        return None;
    }
    // Never echo the text itself: context files can hold private notes.
    let fallback = || serde_json::json!({"unparseable_bytes": text.len()});
    Some(serde_json::from_str(text).unwrap_or_else(|_| fallback()))
}

/// Makes Windows PowerShell 5.1 read and write the console pipes as UTF-8 instead of the OEM code page.
pub const UTF8_CONSOLE: &str = "$u = New-Object System.Text.UTF8Encoding($false); [Console]::InputEncoding = $u; [Console]::OutputEncoding = $u;";

/// PowerShell invocation of an old script. `utf8` forces UTF-8 on both pipes (see
/// INSTALL.md: Windows PowerShell 5.1 otherwise uses the OEM code page for piped stdin/stdout).
pub fn powershell_args(script: &str, script_args: &[&str], utf8: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-NoProfile", "-ExecutionPolicy", "Bypass"]
        .map(String::from)
        .to_vec();
    if !utf8 {
        args.extend(["-File".to_string(), script.to_string()]);
        args.extend(script_args.iter().map(|a| a.to_string()));
        return args;
    }
    let call = format!("{UTF8_CONSOLE} & '{script}' {}", script_args.join(" "));
    args.extend(["-Command".to_string(), call]);
    args
}

/// Runs the old implementation of `hook` (PowerShell or Python) on `input`.
pub fn run_old(
    lab: &Lab,
    hook: &str,
    input: &[u8],
    env: &[(String, String)],
    utf8: bool,
) -> String {
    let script = |name: &str| lab.hooks_dir.join(name).to_string_lossy().into_owned();
    let mut env = env.to_vec();
    match hook {
        "image-tools" => {
            if utf8 {
                env.push(("PYTHONIOENCODING".into(), "utf-8".into()));
            }
            spawn("python", &[script("image-tools.py")], input, &env)
        }
        _ => spawn(
            "powershell",
            &powershell_args(&script(&old_script(lab, hook)), &[], utf8),
            input,
            &env,
        ),
    }
}

fn old_script(lab: &Lab, hook: &str) -> String {
    let name = match hook {
        "attribution" => "no-claude-attribution.ps1",
        "grill" => "grill-before-features.ps1",
        "design-mockup" => "design-on-mockup.ps1",
        "context-inject" => return context_script(lab),
        other => panic!("no old script for {other:?}; expected a ported hook name"),
    };
    name.to_string()
}

/// The old SessionStart context script is the one `*-context.ps1` in the hooks folder.
fn context_script(lab: &Lab) -> String {
    let entries = std::fs::read_dir(&lab.hooks_dir).expect("hooks folder");
    let names = entries.filter_map(|e| e.ok()?.file_name().into_string().ok());
    let mut found: Vec<String> = names.filter(|n| n.ends_with("-context.ps1")).collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one *-context.ps1 in {:?}, got {found:?}",
        lab.hooks_dir
    );
    found.remove(0)
}

/// Runs the binary under test.
pub fn run_new(lab: &Lab, hook: &str, input: &[u8], env: &[(String, String)]) -> String {
    let bin = lab.bin.to_string_lossy().into_owned();
    spawn(&bin, &[hook.to_string()], input, env)
}

/// Feeds every case to both sides and tallies the result.
pub fn run_suite(lab: &Lab, hook: &str, cases: Vec<Case>) -> Row {
    let mut row = Row {
        hook: hook.into(),
        cases: cases.len(),
        ..Row::default()
    };
    let env = lab.base_env();
    for case in &cases {
        let runs = if case.repeat { 2 } else { 1 };
        let old: Vec<_> = (0..runs)
            .map(|_| parse_answer(&run_old(lab, hook, &case.input_for("old"), &env, true)))
            .collect();
        let new: Vec<_> = (0..runs)
            .map(|_| parse_answer(&run_new(lab, hook, &case.input_for("new"), &env)))
            .collect();
        tally(&mut row, case, old == new, || {
            format!("old={old:?}\nnew={new:?}")
        });
        if case.accent {
            let raw = parse_answer(&run_old(lab, hook, &case.input_for("raw"), &env, false));
            row.raw_mode_diffs += usize::from(raw != new[0]);
        }
    }
    row
}

/// Counts one comparison into the row, printing the details of anything unexpected.
pub fn tally(row: &mut Row, case: &Case, same: bool, details: impl Fn() -> String) {
    match (same, case.known_diff) {
        (true, _) => row.matched += 1,
        (false, Some(why)) => {
            row.known += 1;
            eprintln!("[{}] {}: known difference - {why}", row.hook, case.name);
        }
        (false, None) => {
            row.mismatch += 1;
            eprintln!("[{}] {}: MISMATCH\n{}", row.hook, case.name, details());
        }
    }
}

/// Prints the summary table.
pub fn print_table(rows: &[Row]) {
    println!("\n| hook | cases | match | known diff | mismatch | raw-mode accent diffs |");
    println!("|---|---|---|---|---|---|");
    for r in rows {
        let line = format!(
            "| {} | {} | {} | {} | {} | {} |",
            r.hook, r.cases, r.matched, r.known, r.mismatch, r.raw_mode_diffs
        );
        println!("{line}");
    }
}
