//! Everything preparing or launching a trial can report.

use std::env::JoinPathsError;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

use crate::grade::{ImportError, ProcessError};

#[derive(Debug, Error)]
pub(crate) enum RunError {
    #[error("`{}` is not an absolute path", .0.display())]
    NotAbsolute(PathBuf),
    #[error("cannot resolve `{}`", .path.display())]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("`{}` is not a Rust sysroot: it has no `bin/cargo` and `lib/rustlib`", .0.display())]
    NotToolchain(PathBuf),
    #[error("`{}` lies inside `{}`", .inner.display(), .outer.display())]
    Nested { inner: PathBuf, outer: PathBuf },
    #[error("cannot create `{}`", .path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot copy the task into the trial")]
    Copy(#[source] ImportError),
    #[error("cannot write the `srt` policy")]
    Policy(#[source] serde_json::Error),
    #[error("`{}` is a baseline trial, which has no Symposium to set up", .0.display())]
    NotSymposium(PathBuf),
    #[error("cannot write the Symposium config")]
    SymposiumConfig(#[source] toml::ser::Error),
    #[error("a tool named `{0}` is already staged")]
    StagedTwice(String),
    #[error("`{}` is not a file inside its package", .0.display())]
    ToolEntry(PathBuf),
    #[error("cannot stage a tool")]
    Stage(#[source] ImportError),
    #[error("cannot link `{}` to a staged tool", .path.display())]
    Link {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot build `PATH` for the trial")]
    PathList(#[source] JoinPathsError),
    #[error("`{}` has not been written, so the trial cannot run", .0.display())]
    NoPolicy(PathBuf),
    #[error(
        "`srt` reports version `{found}`, but the runner is pinned to {}",
        super::SRT_VERSION
    )]
    SrtVersion { found: String },
    #[error("`srt` needs Node 22.12 or later, but `node --version` reports `{found}`")]
    NodeTooOld { found: String },
    #[error("`{tool}` exited with {status}: {stderr}")]
    ToolFailed {
        tool: &'static str,
        status: ExitStatus,
        stderr: String,
    },
    #[error("`{tool}` did not finish in time")]
    ToolTimeout { tool: &'static str },
    #[error("cannot run `{tool}`")]
    Process {
        tool: &'static str,
        #[source]
        source: ProcessError,
    },
}
