//! Everything preparing a trial can report.

use std::env::JoinPathsError;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::grade::ImportError;

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
    #[error("cannot build `PATH` for the trial")]
    PathList(#[source] JoinPathsError),
}
