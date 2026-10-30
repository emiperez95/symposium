//! Requirement evidence check (Habeas Codicem).
//!
//! `habeas::validate()` requires every Rust function in the repository to cite
//! a requirement. Symposium adopts requirements incrementally, so this test
//! runs the full validation but enforces the per-function rules only for the
//! files listed in `FUNCTION_RULE_FILES`. Every other check (evidence present
//! and current, references resolving, examples tested) applies everywhere.
//!
//! The filter matches Habeas's error text, which is why `habeas` is pinned to
//! an exact version in `Cargo.toml`. See `md/design/requirements-workflow.md`.

use std::path::Path;

/// Files held to the "every function cites a requirement" rule.
const FUNCTION_RULE_FILES: &[&str] = &[];

/// Error fragments produced only by the per-function rules.
const FUNCTION_RULE_ERRORS: &[&str] = &[
    "has no requirement or example citation",
    "missing Rust function attestation file",
];

/// The Rust file a per-function error is about, or `None` for other errors.
///
/// Citation errors read `<file>:<line>: function ...`; attestation errors read
/// `evidence/rust/<file>.json: <file>:<line> ...`.
fn function_rule_file(error: &str) -> Option<&str> {
    if !FUNCTION_RULE_ERRORS.iter().any(|f| error.contains(f)) {
        return None;
    }
    let subject = match error.strip_prefix("evidence/rust/") {
        Some(rest) => rest.split_once(": ")?.1,
        None => error,
    };
    subject.split_once(':').map(|(file, _)| file)
}

#[test]
fn habeas() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let report = habeas::validate_repository(root).expect("cannot read requirement evidence");
    let errors: Vec<&String> = report
        .errors
        .iter()
        .filter(|error| match function_rule_file(error) {
            Some(file) => FUNCTION_RULE_FILES.contains(&file),
            None => true,
        })
        .collect();
    assert!(
        errors.is_empty(),
        "Habeas validation failed ({} requirements, {} examples, {} errors):\n- {}",
        report.requirements,
        report.examples,
        errors.len(),
        errors
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join("\n- ")
    );
}
