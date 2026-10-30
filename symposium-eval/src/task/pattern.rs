use std::fmt;

use globset::{Glob, GlobBuilder};
use serde::Deserialize;
use thiserror::Error;

/// A relative, `/`-separated glob such as `src/**`, compiled when the task loads so an
/// unusable pattern is an authoring error rather than a grading failure. `*` never
/// crosses a `/`, on every platform.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct EditablePattern(Glob);

#[derive(Debug, Error)]
pub(crate) enum PatternError {
    #[error("editable pattern `{0}` must be a relative path using `/`, without `..`")]
    Escapes(String),
    #[error("editable pattern `{pattern}` is not a valid glob")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
}

impl EditablePattern {
    pub(crate) fn new(pattern: &str) -> Result<Self, PatternError> {
        let escapes = pattern.split('/').any(|segment| segment == "..");
        if pattern.is_empty()
            || pattern.starts_with('/')
            || pattern.contains(['\\', ':'])
            || escapes
        {
            return Err(PatternError::Escapes(pattern.to_owned()));
        }
        GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map(Self)
            .map_err(|source| PatternError::Glob {
                pattern: pattern.to_owned(),
                source,
            })
    }

    #[expect(
        dead_code,
        reason = "grading compiles these into a matcher in a later step"
    )]
    pub(crate) fn glob(&self) -> &Glob {
        &self.0
    }
}

impl TryFrom<String> for EditablePattern {
    type Error = PatternError;

    fn try_from(pattern: String) -> Result<Self, Self::Error> {
        Self::new(&pattern)
    }
}

impl AsRef<str> for EditablePattern {
    fn as_ref(&self) -> &str {
        self.0.glob()
    }
}

impl fmt::Display for EditablePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.glob())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_globs() {
        for pattern in ["src/**", "src/lib.rs", "tests/*.rs", "Cargo.toml"] {
            assert!(EditablePattern::new(pattern).is_ok(), "{pattern}");
        }
    }

    #[test]
    fn rejects_patterns_that_could_leave_the_project() {
        for pattern in ["", "/src/**", "../src", "src/../..", r"src\**", "C:/src"] {
            assert!(
                matches!(EditablePattern::new(pattern), Err(PatternError::Escapes(_))),
                "{pattern:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_globs() {
        assert!(matches!(
            EditablePattern::new("src/[abc"),
            Err(PatternError::Glob { .. })
        ));
    }

    #[test]
    fn star_does_not_cross_directories() {
        let matcher = EditablePattern::new("src/*.rs")
            .unwrap()
            .0
            .compile_matcher();

        assert!(matcher.is_match("src/lib.rs"));
        assert!(!matcher.is_match("src/nested/lib.rs"));
    }
}
