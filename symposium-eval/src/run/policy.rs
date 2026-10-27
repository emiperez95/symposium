//! The `srt` policy for one trial (RFD reference, "Enforced: `srt`").
//!
//! `srt` reads are deny-then-allow: everything is readable unless a `denyRead` region
//! covers it, and `allowRead` reopens paths inside one, unless a more specific `denyRead`
//! closes them again. Writes and network are denied unless allowed. [`Policy::can_read`]
//! and [`Policy::can_write`] model those rules, so the isolation invariants are tested
//! as rules rather than as JSON (after Codex's `WritableRoot::is_path_writable`).
//!
//! Every entry is written as `{ "path": ..., "literal": true }`, so `srt` never reads
//! `*`, `?`, or `[` in a path as a pattern. On Linux `srt` matches a read rule against the
//! name a path is reached by, not its target, so each denied root is listed both as given
//! and as resolved.
//!
//! FIXME: trials are not isolated from each other's temporary files. `srt` lets every
//! sandbox write `/tmp/claude`, and a `denyRead` of it does not hide what is already
//! there (sandbox-runtime issues 654 and 611). The launcher has to give each trial its
//! own `/tmp/claude`, for example by binding the trial's `tmp/` over it in a mount
//! namespace, and the `srt` smoke test has to show `$TMPDIR` and cross-trial reads.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{Configuration, RunError, Trial, absolute};

/// The settings file `srt --settings` reads.
#[derive(Debug, Serialize)]
pub(crate) struct Policy {
    filesystem: Filesystem,
    network: Network,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Filesystem {
    deny_read: Vec<Literal>,
    allow_read: Vec<Literal>,
    allow_write: Vec<Literal>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Network {
    allowed_domains: Vec<String>,
}

/// One path `srt` takes as a name, never a pattern.
#[derive(Debug, Serialize)]
struct Literal {
    path: PathBuf,
    literal: bool,
}

/// What the policy needs to know about the machine running the batch.
#[derive(Debug, Clone)]
pub(crate) struct Machine {
    /// Every user's home and the usual mount points, each as given and, where it
    /// resolves elsewhere, as resolved.
    user_data_roots: Vec<PathBuf>,
    /// The Symposium checkout, which holds `evals/`: hidden checks, solutions, and other
    /// tasks' plugins. Canonical.
    checkout: PathBuf,
    /// The checkout as given, when that is another name for it.
    checkout_as_given: PathBuf,
    /// The active Rust toolchain's sysroot, which usually sits under a denied home.
    /// Canonical.
    toolchain: PathBuf,
}

impl Machine {
    /// Denies the platform's user data roots and the current user's home, plus
    /// `extra_user_data_roots`, which can add to those but never replace them.
    /// `toolchain` is a sysroot, as `rustc --print sysroot` reports it.
    ///
    /// # Errors
    ///
    /// When a path is relative, the checkout or toolchain does not exist, the toolchain
    /// is not a sysroot, or it lies inside the checkout.
    pub(crate) fn new(
        extra_user_data_roots: Vec<PathBuf>,
        checkout: &Path,
        toolchain: &Path,
    ) -> Result<Self, RunError> {
        let roots = user_data_roots(
            platform_user_data_roots(),
            std::env::home_dir(),
            extra_user_data_roots,
        );
        Self::with_exact_roots(roots, checkout, toolchain)
    }

    /// A machine whose only user data roots are `user_data_roots`, so tests can predict
    /// the policy exactly.
    fn with_exact_roots(
        user_data_roots: Vec<PathBuf>,
        checkout: &Path,
        toolchain: &Path,
    ) -> Result<Self, RunError> {
        let checkout_as_given = absolute(checkout)?.to_owned();
        let checkout = canonical(checkout)?;
        let toolchain = canonical(toolchain)?;
        if !is_sysroot(&toolchain) {
            return Err(RunError::NotToolchain(toolchain));
        }
        reject_nested(&toolchain, &checkout)?;

        let mut names = Vec::with_capacity(user_data_roots.len());
        for root in user_data_roots {
            absolute(&root)?;
            // A root that cannot be resolved is still denied by the name it was given.
            let resolved = fs::canonicalize(&root).ok();
            for name in [Some(root), resolved].into_iter().flatten() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Ok(Self {
            user_data_roots: names,
            checkout,
            checkout_as_given,
            toolchain,
        })
    }
}

/// The platform's roots, then the current user's home, then the caller's. The home is
/// listed even when a platform root covers it: it can sit anywhere, such as
/// `/var/lib/<service>`.
fn user_data_roots(
    platform: Vec<PathBuf>,
    home: Option<PathBuf>,
    extra: Vec<PathBuf>,
) -> Vec<PathBuf> {
    platform.into_iter().chain(home).chain(extra).collect()
}

/// Every user's home and the usual mount points. The runner uses `srt` only on macOS and
/// Linux, so elsewhere only the current home is denied.
fn platform_user_data_roots() -> Vec<PathBuf> {
    let roots: &[&str] = if cfg!(target_os = "macos") {
        &["/Users", "/Volumes", "/var/root"]
    } else if cfg!(unix) {
        &["/home", "/root", "/mnt", "/media", "/run/media"]
    } else {
        &[]
    };
    roots.iter().map(PathBuf::from).collect()
}

impl Policy {
    /// Confines `trial` to its own directories, the tools its configuration may run, and
    /// the toolchain, with network access to `api_host` only.
    ///
    /// # Errors
    ///
    /// When the evaluation root lies inside the checkout, or the toolchain inside the
    /// evaluation root: either would let one rule silently undo another.
    pub(crate) fn for_trial(
        trial: &Trial,
        machine: &Machine,
        api_host: &str,
    ) -> Result<Self, RunError> {
        let batch = trial.batch();
        reject_nested(batch.eval_root(), &machine.checkout)?;
        reject_nested(&machine.toolchain, batch.eval_root())?;

        let mut deny_read = machine.user_data_roots.clone();
        for name in batch
            .eval_root_names()
            .into_iter()
            .chain([machine.checkout_as_given.clone(), machine.checkout.clone()])
        {
            if !deny_read.contains(&name) {
                deny_read.push(name);
            }
        }

        let mut allow_read = vec![
            trial.root().to_owned(),
            batch.common_tools(),
            machine.toolchain.clone(),
        ];
        // The one difference between the configurations: a baseline cannot read Symposium.
        if trial.configuration() == Configuration::Symposium {
            allow_read.push(batch.symposium_tools());
        }

        let allow_write = vec![
            trial.project(),
            trial.home(),
            trial.cargo_home(),
            trial.tmp(),
            trial.evidence(),
        ];

        Ok(Self {
            filesystem: Filesystem {
                deny_read: literals(deny_read),
                allow_read: literals(allow_read),
                allow_write: literals(allow_write),
            },
            network: Network {
                allowed_domains: vec![api_host.to_owned()],
            },
        })
    }

    /// Whether `srt` lets the trial read `path`: no `denyRead` covers it, or the most
    /// specific `allowRead` covering it is at least as specific as every such deny.
    pub(crate) fn can_read(&self, path: &Path) -> bool {
        let filesystem = &self.filesystem;
        match most_specific(&filesystem.deny_read, path) {
            None => true,
            Some(deny) => {
                most_specific(&filesystem.allow_read, path).is_some_and(|allow| allow >= deny)
            }
        }
    }

    /// Whether `srt` lets the trial write `path`. `srt`'s own rules are not modelled: it
    /// always allows writing its shared temporary directory and never `.git/hooks` or
    /// shell startup files.
    pub(crate) fn can_write(&self, path: &Path) -> bool {
        most_specific(&self.filesystem.allow_write, path).is_some()
    }
}

fn literals(paths: Vec<PathBuf>) -> Vec<Literal> {
    paths
        .into_iter()
        .map(|path| Literal {
            path,
            literal: true,
        })
        .collect()
}

/// The depth of the deepest entry that is `path` or one of its ancestors.
fn most_specific(entries: &[Literal], path: &Path) -> Option<usize> {
    entries
        .iter()
        .filter(|entry| path.starts_with(&entry.path))
        .map(|entry| entry.path.components().count())
        .max()
}

/// `path`, absolute and with links resolved. It must exist: on Linux, `srt` ignores a
/// `denyRead` for a path that does not.
fn canonical(path: &Path) -> Result<PathBuf, RunError> {
    fs::canonicalize(absolute(path)?).map_err(|source| RunError::Canonicalize {
        path: path.to_owned(),
        source,
    })
}

/// Whether `path` holds a toolchain, rather than, say, the `~/.cargo` above one, which
/// would reopen the credentials and binaries beside it.
fn is_sysroot(path: &Path) -> bool {
    let cargo = format!("cargo{}", std::env::consts::EXE_SUFFIX);
    path.join("bin").join(cargo).is_file() && path.join("lib").join("rustlib").is_dir()
}

fn reject_nested(inner: &Path, outer: &Path) -> Result<(), RunError> {
    if inner.starts_with(outer) {
        Err(RunError::Nested {
            inner: inner.to_owned(),
            outer: outer.to_owned(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;
    use crate::evals::Evals;
    use crate::run::Batch;
    use crate::task::{BatchName, Task, TaskName};

    /// A canonical scratch directory holding `home/`, `checkout/evals/`, and a toolchain
    /// under the home, as rustup installs one.
    struct Host {
        _temp: TempDir,
        base: PathBuf,
    }

    impl Host {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let base = fs::canonicalize(temp.path()).unwrap();
            let toolchain = base.join("home/dev/.rustup/toolchains/stable");
            fs::create_dir_all(toolchain.join("bin")).unwrap();
            fs::create_dir_all(toolchain.join("lib/rustlib")).unwrap();
            let cargo = format!("cargo{}", std::env::consts::EXE_SUFFIX);
            fs::write(toolchain.join("bin").join(cargo), "").unwrap();
            fs::create_dir_all(base.join("checkout/evals")).unwrap();
            Self { _temp: temp, base }
        }

        fn join(&self, relative: &str) -> PathBuf {
            relative
                .split('/')
                .fold(self.base.clone(), |path, part| path.join(part))
        }

        fn machine(&self) -> Machine {
            Machine::with_exact_roots(
                vec![self.join("home")],
                &self.join("checkout"),
                &self.join("home/dev/.rustup/toolchains/stable"),
            )
            .unwrap()
        }

        fn batch(&self) -> Batch {
            Batch::create(&self.join("eval"), &BatchName::new("batch").unwrap()).unwrap()
        }
    }

    fn store_durability() -> (Evals, Task) {
        let evals = Evals::in_repository();
        let task = Task::load(&evals, &TaskName::new("store-durability").unwrap()).unwrap();
        (evals, task)
    }

    fn trial(batch: &Batch, number: usize, configuration: Configuration) -> Trial {
        let (evals, task) = store_durability();
        batch
            .create_trial(number, configuration, &evals, &task)
            .unwrap()
    }

    fn policy(trial: &Trial, machine: &Machine) -> Policy {
        Policy::for_trial(trial, machine, "api.anthropic.com").unwrap()
    }

    #[test]
    fn a_trial_cannot_read_another_trial() {
        let host = Host::new();
        let batch = host.batch();
        let first = trial(&batch, 1, Configuration::Symposium);
        let second = trial(&batch, 2, Configuration::Baseline);

        let policy = policy(&first, &host.machine());

        assert!(policy.can_read(&first.project().join("src/lib.rs")));
        assert!(!policy.can_read(&second.project().join("src/lib.rs")));
        assert!(!policy.can_read(&second.policy_file()));
    }

    #[test]
    fn only_a_symposium_trial_can_read_the_staged_cargo_agents() {
        let host = Host::new();
        let batch = host.batch();
        let staged = batch.symposium_tools().join("cargo-agents");
        let symposium = trial(&batch, 1, Configuration::Symposium);
        let baseline = trial(&batch, 2, Configuration::Baseline);

        assert!(policy(&symposium, &host.machine()).can_read(&staged));
        assert!(!policy(&baseline, &host.machine()).can_read(&staged));
    }

    #[test]
    fn a_trial_cannot_read_the_checkout_or_a_home() {
        let host = Host::new();
        let batch = host.batch();
        let trial = trial(&batch, 1, Configuration::Symposium);

        let policy = policy(&trial, &host.machine());

        for path in [
            host.join("checkout/evals/tasks/formatting/checks/double.rs"),
            host.join("home/dev/.cargo/credentials.toml"),
            host.join("home/dev/.cargo/bin/cargo-agents"),
        ] {
            assert!(!policy.can_read(&path), "{}", path.display());
        }
        for path in [
            host.join("home/dev/.rustup/toolchains/stable/bin/cargo"),
            batch.common_tools().join("claude"),
        ] {
            assert!(policy.can_read(&path), "{}", path.display());
        }
    }

    #[test]
    fn a_trial_writes_only_its_working_directories() {
        let host = Host::new();
        let batch = host.batch();
        let trial = trial(&batch, 1, Configuration::Symposium);

        let policy = policy(&trial, &host.machine());

        for dir in [
            trial.project(),
            trial.home(),
            trial.cargo_home(),
            trial.tmp(),
            trial.evidence(),
        ] {
            assert!(policy.can_write(&dir.join("file")), "{}", dir.display());
        }
        for path in [
            trial.fixtures().join("fixture-store/src/lib.rs"),
            trial.registry().join("fixture-store/Symposium.toml"),
            trial.bin().join("cargo"),
            trial.policy_file(),
            batch.symposium_tools().join("cargo-agents"),
        ] {
            assert!(!policy.can_write(&path), "{}", path.display());
        }
    }

    /// The file `srt` reads, kept as a record of its format.
    #[test]
    fn the_policy_is_written_as_literal_paths() {
        let host = Host::new();
        let trial = trial(&host.batch(), 1, Configuration::Symposium);

        let policy = serde_json::to_value(policy(&trial, &host.machine())).unwrap();

        let literal = |path: PathBuf| json!({ "path": path, "literal": true });
        let at = |relative: &str| literal(host.join(relative));
        let trial_dir = |name: &str| at(&format!("eval/batch/trial-1-with-symposium/{name}"));
        let expected: Value = json!({
            "filesystem": {
                "denyRead": [
                    at("home"),
                    at("eval"),
                    at("checkout"),
                ],
                "allowRead": [
                    at("eval/batch/trial-1-with-symposium"),
                    at("eval/batch/tools/common"),
                    at("home/dev/.rustup/toolchains/stable"),
                    at("eval/batch/tools/symposium"),
                ],
                "allowWrite": [
                    trial_dir("project"),
                    trial_dir("home"),
                    trial_dir("cargo-home"),
                    trial_dir("tmp"),
                    trial_dir("evidence"),
                ],
            },
            "network": { "allowedDomains": ["api.anthropic.com"] },
        });
        assert_eq!(policy, expected);
    }

    #[test]
    fn machine_paths_must_be_absolute() {
        let host = Host::new();
        let checkout = host.join("checkout");
        let toolchain = host.join("home/dev/.rustup/toolchains/stable");
        let relative = Path::new("relative");

        for (roots, checkout, toolchain) in [
            (
                vec![relative.to_owned()],
                checkout.as_path(),
                toolchain.as_path(),
            ),
            (Vec::new(), relative, toolchain.as_path()),
            (Vec::new(), checkout.as_path(), relative),
        ] {
            let result = Machine::with_exact_roots(roots, checkout, toolchain);
            assert!(
                matches!(result, Err(RunError::NotAbsolute(_))),
                "{result:?}"
            );
        }
    }

    #[test]
    fn a_missing_checkout_is_an_error() {
        let host = Host::new();

        let result = Machine::with_exact_roots(
            Vec::new(),
            &host.join("misspelled"),
            &host.join("home/dev/.rustup/toolchains/stable"),
        );

        assert!(
            matches!(result, Err(RunError::Canonicalize { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn the_toolchain_must_be_a_sysroot() {
        let host = Host::new();

        for not_a_sysroot in ["home", "home/dev/.rustup"] {
            let result = Machine::with_exact_roots(
                Vec::new(),
                &host.join("checkout"),
                &host.join(not_a_sysroot),
            );
            assert!(
                matches!(result, Err(RunError::NotToolchain(_))),
                "{not_a_sysroot}: {result:?}"
            );
        }
    }

    #[test]
    fn overlapping_layouts_are_rejected() {
        let host = Host::new();
        let machine = host.machine();

        let inside_checkout =
            Batch::create(&host.join("checkout/eval"), &BatchName::new("b").unwrap()).unwrap();
        let trial = trial(&inside_checkout, 1, Configuration::Baseline);
        let result = Policy::for_trial(&trial, &machine, "api.anthropic.com");
        assert!(matches!(result, Err(RunError::Nested { .. })), "{result:?}");

        let toolchain = host.join("checkout/toolchain");
        fs::create_dir_all(toolchain.join("lib/rustlib")).unwrap();
        fs::create_dir_all(toolchain.join("bin")).unwrap();
        let cargo = format!("cargo{}", std::env::consts::EXE_SUFFIX);
        fs::write(toolchain.join("bin").join(cargo), "").unwrap();
        let result = Machine::with_exact_roots(Vec::new(), &host.join("checkout"), &toolchain);
        assert!(matches!(result, Err(RunError::Nested { .. })), "{result:?}");
    }

    #[test]
    fn extra_roots_add_to_the_platform_roots_and_the_home() {
        let [platform, home, extra] = ["/home", "/var/lib/service", "/srv"].map(PathBuf::from);

        let roots = user_data_roots(
            vec![platform.clone()],
            Some(home.clone()),
            vec![extra.clone()],
        );

        assert_eq!(roots, [platform, home, extra]);
    }

    #[test]
    fn a_root_named_twice_is_denied_once() {
        let host = Host::new();
        let home = host.join("home");

        let machine = Machine::with_exact_roots(
            vec![home.clone(), home.clone()],
            &host.join("checkout"),
            &host.join("home/dev/.rustup/toolchains/stable"),
        )
        .unwrap();

        assert_eq!(machine.user_data_roots, [home]);
    }

    /// `srt` matches read rules by the name a path is reached through, so a root reached
    /// through a link is denied by both names.
    #[cfg(unix)]
    #[test]
    fn a_root_reached_through_a_link_is_denied_by_both_names() {
        use std::os::unix::fs::symlink;

        let host = Host::new();
        for (link, target) in [("home-link", "home"), ("checkout-link", "checkout")] {
            symlink(host.join(target), host.join(link)).unwrap();
        }
        fs::create_dir(host.join("eval")).unwrap();
        symlink(host.join("eval"), host.join("eval-link")).unwrap();
        let machine = Machine::with_exact_roots(
            vec![host.join("home-link")],
            &host.join("checkout-link"),
            &host.join("home/dev/.rustup/toolchains/stable"),
        )
        .unwrap();
        let batch =
            Batch::create(&host.join("eval-link"), &BatchName::new("batch").unwrap()).unwrap();
        let trial = trial(&batch, 1, Configuration::Baseline);

        let policy = policy(&trial, &machine);

        for name in [
            "home-link",
            "home",
            "eval-link",
            "eval",
            "checkout-link",
            "checkout",
        ] {
            let path = host.join(name).join("file");
            assert!(!policy.can_read(&path), "{}", path.display());
        }
    }
}
