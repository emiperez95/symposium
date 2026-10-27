//! Everything preparing a trial can report.

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
    #[error("cannot create `{}`", .path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot copy the task into the trial")]
    Copy(#[source] ImportError),
}
