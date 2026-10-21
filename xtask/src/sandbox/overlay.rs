//! Building the project and fake HOME from a scenario.
//!
//! Mirrors `symposium_testlib`'s fixture overlay (which is too heavy to link
//! into xtask): fixtures copy in order, text files (`toml md json txt ts js sh`)
//! get `$TEST_DIR` / `$BINARY` expanded, and a `dot-symposium/` directory is
//! the Symposium config dir. Here that means `$HOME/.symposium/`, not a folder
//! inside the project. `$HOME` is also expanded.

use anyhow::{Context, bail};
use std::fs;
use std::path::{Path, PathBuf};

use super::scenario::InlineFile;

pub struct Vars<'a> {
    pub test_dir: &'a Path,
    pub binary: &'a Path,
    pub home: &'a Path,
}

fn is_text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("toml" | "md" | "json" | "txt" | "ts" | "js" | "sh")
    )
}

pub fn expand(content: &str, vars: &Vars) -> String {
    content
        .replace("$TEST_DIR", &vars.test_dir.display().to_string())
        .replace("$BINARY", &vars.binary.display().to_string())
        .replace("$HOME", &vars.home.display().to_string())
}

/// Resolve a scenario path: relative to the project, or `~/` for HOME.
/// Rejects anything that could leave either root.
pub fn resolve(rel: &str, project: &Path, home: &Path) -> anyhow::Result<PathBuf> {
    let (root, tail) = match rel.strip_prefix("~/") {
        Some(t) => (home, t),
        None => (project, rel),
    };
    let tail = Path::new(tail);
    if tail.is_absolute()
        || tail
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("path `{rel}` must stay inside the project or HOME");
    }
    Ok(root.join(tail))
}

pub fn overlay_fixtures(
    fixtures_base: &Path,
    names: &[String],
    project: &Path,
    home: &Path,
    vars: &Vars,
) -> anyhow::Result<()> {
    for name in names {
        let dir = fixtures_base.join(name);
        if !dir.is_dir() {
            bail!("fixture not found: {}", dir.display());
        }
        copy_tree(&dir, project, home, vars, true)?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path, home: &Path, vars: &Vars, top: bool) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let name = entry.file_name();
        if from.is_dir() {
            let to = if top && name == "dot-symposium" {
                home.join(".symposium")
            } else {
                dst.join(&name)
            };
            copy_tree(&from, &to, home, vars, false)?;
        } else {
            let to = dst.join(&name);
            if is_text_file(&from) {
                let text = fs::read_to_string(&from)
                    .with_context(|| format!("reading {}", from.display()))?;
                fs::write(&to, expand(&text, vars))?;
            } else {
                fs::copy(&from, &to)?;
            }
        }
    }
    Ok(())
}

pub fn write_inline(
    files: &[InlineFile],
    project: &Path,
    home: &Path,
    vars: &Vars,
) -> anyhow::Result<()> {
    for f in files {
        let path = resolve(&f.path, project, home)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, expand(&f.content, vars))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rules() {
        let p = Path::new("/p");
        let h = Path::new("/h");
        assert_eq!(resolve("a/b", p, h).unwrap(), Path::new("/p/a/b"));
        assert_eq!(resolve("~/.x/y", p, h).unwrap(), Path::new("/h/.x/y"));
        assert!(resolve("../x", p, h).is_err());
        assert!(resolve("~/../x", p, h).is_err());
        assert!(resolve("/etc/passwd", p, h).is_err());
    }

    #[test]
    fn expands_variables() {
        let vars = Vars {
            test_dir: Path::new("/t"),
            binary: Path::new("/b/cargo-agents"),
            home: Path::new("/h"),
        };
        assert_eq!(
            expand("$TEST_DIR $BINARY $HOME", &vars),
            "/t /b/cargo-agents /h"
        );
    }

    #[test]
    fn dot_symposium_maps_to_home_and_text_expands() {
        let tmp = std::env::temp_dir().join(format!("xtask-overlay-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let fx = tmp.join("fx/one");
        fs::create_dir_all(fx.join("dot-symposium")).unwrap();
        fs::write(fx.join("dot-symposium/config.toml"), "d = \"$TEST_DIR\"").unwrap();
        fs::write(fx.join("Cargo.toml"), "x").unwrap();
        let (project, home) = (tmp.join("project"), tmp.join("home"));
        fs::create_dir_all(&home).unwrap();
        let vars = Vars {
            test_dir: &project,
            binary: Path::new("/b"),
            home: &home,
        };
        overlay_fixtures(&tmp.join("fx"), &["one".into()], &project, &home, &vars).unwrap();
        let cfg = fs::read_to_string(home.join(".symposium/config.toml")).unwrap();
        assert!(cfg.contains(&project.display().to_string()));
        assert!(project.join("Cargo.toml").exists());
        assert!(!project.join("dot-symposium").exists());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
