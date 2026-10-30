//! Scenario files: `xtask/sandbox/scenarios/<name>.toml`.

use anyhow::{Context, bail};
use regex::Regex;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default)]
    pub description: String,
    /// `tests/fixtures/<name>` folders overlaid into the project, in order.
    #[serde(default)]
    pub fixtures: Vec<String>,
    #[serde(default)]
    pub symposium: SymposiumSetup,
    /// Extra domains the sandbox may reach (`api.anthropic.com` is implied).
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    /// Inline project files, written after the fixtures.
    #[serde(default)]
    pub files: Vec<InlineFile>,
    #[serde(default)]
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymposiumSetup {
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default = "default_hook_scope")]
    pub hook_scope: String,
    /// Text written to `$HOME/.symposium/config.toml` before `init`.
    #[serde(default)]
    pub config: Option<String>,
}

impl Default for SymposiumSetup {
    fn default() -> Self {
        Self {
            agent: default_agent(),
            hook_scope: default_hook_scope(),
            config: None,
        }
    }
}

fn default_agent() -> String {
    "claude".into()
}
fn default_hook_scope() -> String {
    "global".into()
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Timeouts {
    /// Seconds to wait for the agent's input box after launch.
    #[serde(default = "default_startup")]
    pub startup: u64,
    /// Seconds to wait for an answer to finish after a prompt.
    #[serde(default = "default_answer")]
    pub answer: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            startup: default_startup(),
            answer: default_answer(),
        }
    }
}

fn default_startup() -> u64 {
    60
}
fn default_answer() -> u64 {
    90
}

/// A file with literal content. `path` is relative to the project, or to
/// `$HOME` with a `~/` prefix.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct InlineFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub name: String,
    /// Directories to create (same path rules as [`InlineFile::path`]).
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub files: Vec<InlineFile>,
}

/// One entry of `[[steps]]`; exactly one action key is set.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawStep {
    wait: Option<u64>,
    prompt: Option<String>,
    expect_answer: Option<String>,
    expect_log: Option<String>,
    reject_log: Option<String>,
    expect_file: Option<String>,
    /// For `expect_log` / `reject_log`: the exact number of matches.
    count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Wait(u64),
    Prompt(String),
    ExpectAnswer(String),
    ExpectLog {
        pattern: String,
        count: Option<usize>,
    },
    RejectLog {
        pattern: String,
        count: Option<usize>,
    },
    ExpectFile(String),
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = RawStep::deserialize(d)?;
        let set = [
            raw.wait.is_some(),
            raw.prompt.is_some(),
            raw.expect_answer.is_some(),
            raw.expect_log.is_some(),
            raw.reject_log.is_some(),
            raw.expect_file.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if set != 1 {
            return Err(D::Error::custom("a step must set exactly one action key"));
        }
        if raw.count.is_some() && raw.expect_log.is_none() && raw.reject_log.is_none() {
            return Err(D::Error::custom(
                "`count` only applies to expect_log / reject_log",
            ));
        }
        Ok(if let Some(s) = raw.wait {
            Step::Wait(s)
        } else if let Some(p) = raw.prompt {
            Step::Prompt(p)
        } else if let Some(p) = raw.expect_answer {
            Step::ExpectAnswer(p)
        } else if let Some(pattern) = raw.expect_log {
            Step::ExpectLog {
                pattern,
                count: raw.count,
            }
        } else if let Some(pattern) = raw.reject_log {
            Step::RejectLog {
                pattern,
                count: raw.count,
            }
        } else {
            Step::ExpectFile(raw.expect_file.unwrap())
        })
    }
}

impl Scenario {
    pub fn parse(text: &str) -> anyhow::Result<Scenario> {
        let s: Scenario = toml::from_str(text)?;
        s.validate()?;
        Ok(s)
    }

    pub fn load(dir: &Path, name: &str) -> anyhow::Result<Scenario> {
        let path = dir.join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("no scenario `{name}` ({})", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.variants.is_empty() {
            bail!("a scenario needs at least one [[variants]] entry");
        }
        let mut seen = std::collections::BTreeSet::new();
        for v in &self.variants {
            if !seen.insert(&v.name) {
                bail!("duplicate variant `{}`", v.name);
            }
            if v.name.is_empty()
                || !v
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                bail!("variant name `{}` must be [A-Za-z0-9_-]+", v.name);
            }
        }
        for step in &self.steps {
            let pattern = match step {
                Step::ExpectAnswer(p)
                | Step::ExpectLog { pattern: p, .. }
                | Step::RejectLog { pattern: p, .. } => Some(p),
                _ => None,
            };
            if let Some(p) = pattern {
                Regex::new(p).with_context(|| format!("invalid regex `{p}`"))?;
            }
        }
        Ok(())
    }
}

/// Names of the scenarios in `dir`, sorted.
pub fn list(dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        [[variants]]
        name = "a"
        [[steps]]
        wait = 3
        [[steps]]
        expect_log = "x"
        count = 2
    "#;

    #[test]
    fn parses_steps_and_defaults() {
        let s = Scenario::parse(MINIMAL).unwrap();
        assert_eq!(s.symposium.agent, "claude");
        assert_eq!(s.timeouts.answer, 90);
        assert_eq!(
            s.steps,
            vec![
                Step::Wait(3),
                Step::ExpectLog {
                    pattern: "x".into(),
                    count: Some(2)
                }
            ]
        );
    }

    #[test]
    fn rejects_step_with_two_actions() {
        let t = "[[variants]]\nname='a'\n[[steps]]\nwait=1\nprompt='hi'\n";
        assert!(Scenario::parse(t).is_err());
    }

    #[test]
    fn rejects_count_on_other_steps() {
        let t = "[[variants]]\nname='a'\n[[steps]]\nprompt='hi'\ncount=1\n";
        assert!(Scenario::parse(t).is_err());
    }

    #[test]
    fn rejects_bad_regex_and_bad_variants() {
        assert!(Scenario::parse("[[variants]]\nname='a'\n[[steps]]\nexpect_answer='('\n").is_err());
        assert!(Scenario::parse("[[variants]]\nname='a/b'\n").is_err());
        assert!(Scenario::parse("[[variants]]\nname='a'\n[[variants]]\nname='a'\n").is_err());
        assert!(Scenario::parse("").is_err());
    }

    #[test]
    fn shipped_scenarios_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("sandbox/scenarios");
        for name in list(&dir).unwrap() {
            Scenario::load(&dir, &name).unwrap();
        }
    }
}
