//! `cc-hooks`: one native binary answering Claude Code hooks in a few milliseconds, replacing
//! PowerShell / Python scripts that each paid ~200-500 ms of interpreter start per call.
//!
//! Contract for every subcommand: read the hook JSON from stdin, print at most one JSON line,
//! exit 0. A defect (bad input, panic) never blocks the user: it exits 0 with no output.

mod config;
mod dispatch;
mod fs_port;
mod hook_io;
mod hooks;
mod proc;
mod text;

use std::io::{Read, Write};

fn main() {
    // A panic must not print a backtrace into the hook channel; it just means "no answer".
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = std::panic::catch_unwind(|| answer_hook(&args)).unwrap_or(0);
    std::process::exit(code);
}

fn answer_hook(args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "--help" {
        eprintln!(
            "usage: cc-hooks <{}> [--post] < hook.json",
            dispatch::SUBCOMMANDS.join("|")
        );
        return 0;
    }
    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() {
        return 0;
    }
    let outcome = dispatch::dispatch(args, &raw);
    let mut stdout = std::io::stdout().lock();
    let code = hook_io::write_outcome(&outcome, &mut stdout);
    let _ = stdout.flush();
    code
}
