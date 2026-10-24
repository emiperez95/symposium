//! The `task.toml` schema, field for field.
//!
//! `Option` keeps "absent" distinct from "empty" where the format gives them different
//! meanings. Unknown keys are errors, so a misspelled expectation cannot silently make
//! no claim.

use serde::Deserialize;

use super::{EditablePattern, FixtureName, HookName, PluginName, SkillName, TaskKind};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawTask {
    pub kind: TaskKind,
    pub prompt: String,
    pub expected_output: Option<String>,
    #[serde(default)]
    pub plugins: Vec<PluginName>,
    #[serde(default)]
    pub fixtures: Vec<FixtureName>,
    pub editable: Option<Vec<EditablePattern>>,
    #[serde(default)]
    pub fmt_check: bool,
    #[serde(default)]
    pub expected: RawExpected,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawExpected {
    #[serde(default)]
    pub hooks: Vec<HookName>,
    #[serde(default)]
    pub skills: RawSkills,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawSkills {
    #[serde(default)]
    pub installed: Vec<SkillName>,
    #[serde(default)]
    pub not_installed: Vec<SkillName>,
    /// Absent means `installed` minus `not_used`; `[]` makes no claim.
    pub used: Option<Vec<SkillName>>,
    #[serde(default)]
    pub not_used: Vec<SkillName>,
}
