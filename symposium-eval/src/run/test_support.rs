//! A scratch machine and the trials on it, shared by `run`'s tests.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::{ApiKey, Batch, Configuration, Machine, Trial};
use crate::evals::Evals;
use crate::task::{BatchName, Task, TaskName};

/// A canonical scratch directory holding `home/`, `checkout/evals/`, and a toolchain
/// under the home, as rustup installs one.
pub(crate) struct Host {
    _temp: TempDir,
    base: PathBuf,
}

impl Host {
    pub(crate) fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let host = Self { _temp: temp, base };
        host.add_toolchain("home/dev/.rustup/toolchains/stable");
        fs::create_dir_all(host.join("checkout/evals")).unwrap();
        host
    }

    /// `relative`, written with `/`, under the scratch directory.
    pub(crate) fn join(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.base.clone(), |path, part| path.join(part))
    }

    /// Lays out the parts of a sysroot that `Machine` checks for.
    pub(crate) fn add_toolchain(&self, relative: &str) -> PathBuf {
        let toolchain = self.join(relative);
        fs::create_dir_all(toolchain.join("bin")).unwrap();
        fs::create_dir_all(toolchain.join("lib").join("rustlib")).unwrap();
        let cargo = format!("cargo{}", std::env::consts::EXE_SUFFIX);
        fs::write(toolchain.join("bin").join(cargo), "").unwrap();
        toolchain
    }

    pub(crate) fn machine(&self) -> Machine {
        Machine::for_tests(
            vec![self.join("home")],
            &self.join("checkout"),
            &self.join("home/dev/.rustup/toolchains/stable"),
        )
    }

    pub(crate) fn batch(&self) -> Batch {
        Batch::create(&self.join("eval"), &BatchName::new("batch").unwrap()).unwrap()
    }
}

pub(crate) fn store_durability() -> (Evals, Task) {
    let evals = Evals::in_repository();
    let task = Task::load(&evals, &TaskName::new("store-durability").unwrap()).unwrap();
    (evals, task)
}

pub(crate) fn trial(batch: &Batch, number: usize, configuration: Configuration) -> Trial {
    let (evals, task) = store_durability();
    batch
        .create_trial(number, configuration, &evals, &task)
        .unwrap()
}

/// A key for tests that never leave the sandbox's proxy.
pub(crate) fn api_key() -> ApiKey {
    ApiKey::new("EXAMPLE_API_KEY", "sk-test".into())
}
