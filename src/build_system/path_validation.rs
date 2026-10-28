//! User-provided build path validation.
//!
//! WHAT: validates filesystem paths supplied to project/build commands.
//! WHY: this is build-system input handling, not compiler frontend semantics, so
//! file diagnostics should stay at the project orchestration boundary.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::path::{Path, PathBuf};

pub(crate) fn check_if_valid_path(path: &str) -> Result<PathBuf, InfrastructureFailure> {
    // If it contains Unix-style slashes, convert them on Windows before existence checks.
    let path = if cfg!(windows) && path.contains('/') {
        &path.replace('/', "\\")
    } else {
        path
    };

    let path = Path::new(path);

    if !path.exists() {
        return Err(InfrastructureFailure::filesystem(
            CompilerStage::ProjectBuildAssembly,
            path,
            "Path does not exist",
        ));
    }

    Ok(path.to_path_buf())
}
