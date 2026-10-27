//! Validated names from `task.toml` and the `evals/` tree.
//!
//! Task, plugin, fixture, skill, check, and batch names become directory or file names, so
//! [`DirName`] accepts only names that are one portable path component on every
//! platform. A marker type per kind keeps a plugin name from being passed where a skill
//! name is expected. Hook names never touch the filesystem, so [`HookName`] only
//! requires text.

use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use thiserror::Error;

/// Marker types for [`DirName`]. They have no values.
pub(crate) mod marker {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) enum Task {}
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) enum Plugin {}
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) enum Fixture {}
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) enum Skill {}
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) enum Check {}
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) enum Batch {}
}

pub(crate) type TaskName = DirName<marker::Task>;
pub(crate) type PluginName = DirName<marker::Plugin>;
pub(crate) type FixtureName = DirName<marker::Fixture>;
pub(crate) type SkillName = DirName<marker::Skill>;
pub(crate) type CheckName = DirName<marker::Check>;
/// A batch directory under the evaluation root, named by the runner rather than `evals/`.
pub(crate) type BatchName = DirName<marker::Batch>;

/// A name that is safe to join onto any directory, on any platform.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String", bound = "")]
pub(crate) struct DirName<K> {
    value: String,
    kind: PhantomData<K>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("`{name}` is not a portable directory name: {reason}")]
pub(crate) struct NameError {
    name: String,
    reason: &'static str,
}

/// A Symposium hook name, as written in a plugin's `[[hooks]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct HookName(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("a hook name cannot be empty")]
pub(crate) struct EmptyHookName;

impl<K> DirName<K> {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, NameError> {
        let value = value.into();
        match portability_problem(&value) {
            None => Ok(Self {
                value,
                kind: PhantomData,
            }),
            Some(reason) => Err(NameError {
                name: value,
                reason,
            }),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.value
    }
}

impl<K> TryFrom<String> for DirName<K> {
    type Error = NameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<K> AsRef<str> for DirName<K> {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl<K> fmt::Display for DirName<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl HookName {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, EmptyHookName> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(EmptyHookName)
        } else {
            Ok(Self(value))
        }
    }
}

impl TryFrom<String> for HookName {
    type Error = EmptyHookName;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for HookName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HookName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Device names Windows reserves in every directory, with or without an extension.
/// Cargo rejects the same list for package names.
const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn portability_problem(name: &str) -> Option<&'static str> {
    let stem = name.split('.').next().unwrap_or_default();
    if name.is_empty() {
        Some("it is empty")
    } else if name == "." || name == ".." {
        Some("it names a directory relative to itself")
    } else if name.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*']) {
        Some("it contains a path separator or a character Windows forbids")
    } else if name.chars().any(char::is_control) {
        Some("it contains a control character")
    } else if name.ends_with(['.', ' ']) {
        Some("Windows drops a trailing dot or space")
    } else if WINDOWS_RESERVED
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        Some("Windows reserves it as a device name")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_names() {
        for name in [
            "formatting",
            "fixture-store",
            "rust_formatting",
            "v2",
            "a.b",
            "con-tools",
        ] {
            assert!(SkillName::new(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_names_that_are_not_one_portable_component() {
        const SEPARATOR: &str = "it contains a path separator or a character Windows forbids";
        let cases = [
            ("", "it is empty"),
            (".", "it names a directory relative to itself"),
            ("..", "it names a directory relative to itself"),
            ("a/b", SEPARATOR),
            (r"a\b", SEPARATOR),
            ("c:", SEPARATOR),
            ("what?", SEPARATOR),
            ("a\u{7}b", "it contains a control character"),
            ("task.", "Windows drops a trailing dot or space"),
            ("task ", "Windows drops a trailing dot or space"),
            ("CON", "Windows reserves it as a device name"),
            ("nul.txt", "Windows reserves it as a device name"),
            ("Com1", "Windows reserves it as a device name"),
        ];
        for (name, reason) in cases {
            let expected = NameError {
                name: name.to_owned(),
                reason,
            };
            assert_eq!(SkillName::new(name), Err(expected), "{name:?}");
        }
    }

    #[test]
    fn hook_names_allow_anything_but_blank() {
        assert!(HookName::new("plugin:format/on-stop").is_ok());
        assert_eq!(HookName::new("  "), Err(EmptyHookName));
    }
}
