//! Entry-root and homepage path policy for HTML projects.
//!
//! WHAT: resolves the configured entry root and derives homepage expectations for directory builds.
//! WHY: the HTML project builder should focus on artifact orchestration, not filesystem policy.

use crate::build_system::create_project_modules::resolve_project_entry_root;
use crate::build_system::utils::file_error_messages;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::diagnostics::missing_homepage_messages;
use crate::projects::settings::Config;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct HtmlEntryPathPlan {
    pub(crate) resolved_entry_root: Option<PathBuf>,
    is_directory_build: bool,
}

impl HtmlEntryPathPlan {
    /// Build the entry-path plan for one HTML project build.
    ///
    /// WHY: canonical entry-root resolution needs to happen once so page output logic and
    /// homepage validation operate on the same normalized paths.
    pub(crate) fn from_config(
        config: &Config,
        string_table: &StringTable,
    ) -> Result<Self, CompilerMessages> {
        let is_directory_build = config.entry_dir.is_dir();
        let resolved_entry_root =
            resolve_canonical_entry_root(config, is_directory_build, string_table)?;

        Ok(Self {
            resolved_entry_root,
            is_directory_build,
        })
    }

    /// Return whether an artifact-producing module is the directory-build homepage.
    ///
    /// WHAT: identifies the active module whose root directory is exactly `entry_root`.
    /// WHY: normal-root filenames are cosmetic and API-only roots must not claim the homepage.
    pub(crate) fn is_homepage_entry(&self, entry_point: &Path) -> bool {
        self.is_directory_build && entry_point.parent() == self.resolved_entry_root.as_deref()
    }

    pub(crate) fn is_directory_build(&self) -> bool {
        self.is_directory_build
    }

    /// Enforce the HTML homepage requirement for directory builds.
    ///
    /// WHY: directory routing depends on an active artifact-producing module at `entry_root`,
    /// while single-file builds do not have that contract.
    pub(crate) fn require_homepage_if_directory_build(
        &self,
        config: &Config,
        has_directory_homepage: bool,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        if self.is_directory_build && !has_directory_homepage {
            return Err(missing_homepage_error(
                config,
                self.resolved_entry_root.as_deref(),
                string_table,
            ));
        }

        Ok(())
    }
}

/// Resolve and canonicalize the entry root for directory builds.
///
/// Returns `None` for single-file builds and `Some(canonical_path)` for directory builds.
fn resolve_canonical_entry_root(
    config: &Config,
    is_directory_build: bool,
    string_table: &StringTable,
) -> Result<Option<PathBuf>, CompilerMessages> {
    if !is_directory_build {
        return Ok(None);
    }

    let entry_root_path = resolve_project_entry_root(config);
    let canonical = fs::canonicalize(&entry_root_path).map_err(|error| {
        file_error_messages(
            &config.entry_dir,
            format!(
                "Failed to resolve configured HTML entry root '{}': {error}",
                entry_root_path.display()
            ),
            string_table,
        )
    })?;

    Ok(Some(canonical))
}

fn missing_homepage_error(
    config: &Config,
    resolved_entry_root: Option<&Path>,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let entry_root = resolved_entry_root.unwrap_or_else(|| Path::new("."));

    missing_homepage_messages(&config.entry_dir, entry_root, string_table)
}
