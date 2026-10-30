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

/// Files held to the "every function cites a requirement" rule, written with
/// `/` separators.
const FUNCTION_RULE_FILES: &[&str] = &[];

/// Error fragments produced only by the per-function rules.
const FUNCTION_RULE_ERRORS: &[&str] = &[
    "has no requirement or example citation",
    "missing Rust function attestation file",
];

/// The Rust file a per-function error is about, with `/` separators, or
/// `None` for other errors.
///
/// Citation errors read `<file>:<line>: function ...`; attestation errors read
/// `evidence/rust/<file>.json: <file>:<line> ...`. Habeas prints paths with
/// the platform separator, so Windows labels use `\`.
fn function_rule_file(error: &str) -> Option<String> {
    if !FUNCTION_RULE_ERRORS.iter().any(|f| error.contains(f)) {
        return None;
    }
    let error = error.replace('\\', "/");
    let subject = match error.strip_prefix("evidence/rust/") {
        Some(rest) => rest.split_once(": ")?.1,
        None => &error,
    };
    subject.split_once(':').map(|(file, _)| file.to_owned())
}

#[test]
fn habeas() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let report = habeas::validate_repository(root).expect("cannot read requirement evidence");

    // Most of the repository is outside the allowlist, so Habeas always
    // reports per-function errors. If none are recognized, its wording has
    // changed and the filter below would let everything through.
    assert!(
        report
            .errors
            .iter()
            .any(|e| function_rule_file(e).is_some()),
        "no Habeas error matched the per-function filter; update FUNCTION_RULE_ERRORS"
    );

    let errors: Vec<&str> = report
        .errors
        .iter()
        .map(String::as_str)
        .filter(|e| function_rule_file(e).is_none_or(|f| FUNCTION_RULE_FILES.contains(&&*f)))
        .collect();
    assert!(
        errors.is_empty(),
        "Habeas validation failed ({} requirements, {} examples, {} errors):\n- {}",
        report.requirements,
        report.examples,
        errors.len(),
        errors.join("\n- ")
    );
}

/// The filter's view of each error shape Habeas 0.2.0 produces.
#[test]
fn function_rule_file_parses_habeas_errors() {
    let attestation_purpose =
        "review each function and confirm it cites every applicable requirement or named example";
    let cases = [
        (
            "src/output.rs:17: function normal has no requirement or example citation".to_owned(),
            Some("src/output.rs"),
        ),
        (
            format!(
                "evidence/rust/src/output.rs.json: src/output.rs:17 impl[Output]/fn[normal]: missing Rust function attestation file; {attestation_purpose}"
            ),
            Some("src/output.rs"),
        ),
        (
            r"src\output.rs:17: function normal has no requirement or example citation".to_owned(),
            Some("src/output.rs"),
        ),
        (
            format!(
                r"evidence\rust\src\output.rs.json: src\output.rs:17 impl[Output]/fn[normal]: missing Rust function attestation file; {attestation_purpose}"
            ),
            Some("src/output.rs"),
        ),
        (
            "src/output.rs:3: cannot parse Rust source: expected `;`".to_owned(),
            None,
        ),
        (
            "evidence/design.requirements.hooks.no_prompt.json: determination is false: stale"
                .to_owned(),
            None,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(function_rule_file(&error).as_deref(), expected, "{error}");
    }
}
