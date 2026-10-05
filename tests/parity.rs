//! Parity harness: feeds the same hook payloads to the original scripts and to `cc-hooks`, and
//! compares the parsed JSON answers. Slow (one PowerShell / Python start per case) and tied to the
//! machine that still has the old scripts, so it only runs with `CC_HOOKS_PARITY=1`:
//!
//!     $env:CC_HOOKS_PARITY = '1'; cargo test --test parity -- --nocapture
//!
//! Old scripts are read from `%USERPROFILE%\.claude\hooks`; the quality-gate detector is found next
//! to the `pre_script` in the local cc-hooks.toml. Nothing machine-specific lives in this file.

mod parity_support;

use parity_support::*;

#[test]
fn old_scripts_and_binary_agree() {
    if std::env::var("CC_HOOKS_PARITY").as_deref() != Ok("1") {
        eprintln!("parity skipped: set CC_HOOKS_PARITY=1 to run it");
        return;
    }
    let lab = Lab::new();
    let rows = vec![
        run_suite(&lab, "attribution", attribution_cases(&lab)),
        run_suite(&lab, "grill", grill_cases()),
        run_suite(&lab, "design-mockup", design_cases()),
        run_suite(&lab, "image-tools", image_cases(&lab)),
        run_suite(&lab, "context-inject", context_cases()),
        run_herdr_suite(&lab),
        run_gate_suite(&lab),
    ];
    print_table(&rows);
    let mismatches: usize = rows.iter().map(|r| r.mismatch).sum();
    assert_eq!(mismatches, 0, "parity mismatches, see the table above");
}
