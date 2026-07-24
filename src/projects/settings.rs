//! Global compiler and project constants.
//!
//! WHAT: defines file extensions, reserved names, heuristic capacity constants, and project
//!       configuration structures shared across the compiler and build system.
//! WHY: keeping these values in one module prevents magic literals from spreading through the
//!      codebase and makes capacity tuning explicit.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidConfigReason};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::HashMap;
use std::path::PathBuf;

/// The canonical language source file extension (without dot).
pub const LANGUAGE_SOURCE_EXTENSION: &str = "moth";

/// Dotted language source extension.
pub const LANGUAGE_SOURCE_SUFFIX: &str = ".moth";

/// The canonical content/template file extension (without dot).
pub const CONTENT_EXTENSION: &str = "mtf";

/// Dotted content extension.
pub const CONTENT_SUFFIX: &str = ".mtf";

/// The canonical Markdown file extension (without dot).
pub const MARKDOWN_EXTENSION: &str = "md";

/// Dotted Markdown extension.
pub const MARKDOWN_SUFFIX: &str = ".md";

pub const COMP_PAGE_KEYWORD: &str = "#page";
pub const GLOBAL_PAGE_KEYWORD: &str = "#global";
pub const INDEX_PAGE_NAME: &str = "index.html";
pub const CONFIG_FILE_NAME: &str = "config.moth";

/// Special reserved names for functions and variables created by the compiler
pub const TOP_LEVEL_TEMPLATE_NAME: &str = "#template";
pub const TOP_LEVEL_CONST_TEMPLATE_NAME: &str = "#const_template";
pub const IMPLICIT_START_FUNC_NAME: &str = "start";
pub const TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY: &str = "template_const_loop_iteration_limit";
pub const DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 10_000;
pub const MAX_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 1_000_000;

// This is a guess about how much should be initially allocated for vecs in the compiler.
// This should be a rough guess to help avoid too many allocations
// and is just a heuristic based on tests with rudimentary small snippets of code.
// Should be recalculated at a later point.
pub const MINIMUM_STRING_TABLE_CAPACITY: usize = 32;
pub const SRC_TO_TOKEN_RATIO: usize = 5; // (Maybe) About 1/6 source code to tokens observed
pub const IMPORTS_CAPACITY: usize = 6; // (No Idea atm)
pub const EXPORTS_CAPACITY: usize = 6; // (No Idea atm)
pub const TOKEN_TO_HEADER_RATIO: usize = 35; // (Maybe) About 1/35 tokens to AstNode ratio
pub const TOKEN_TO_DECLARATION_RATIO: usize = 20; // (Maybe) About 1/20 tokens for each new declaration symbol
pub const TOKEN_TO_NODE_RATIO: usize = 10; // (Maybe) About 1/10 tokens to AstNode ratio
pub const MINIMUM_LIKELY_DECLARATIONS: usize = 10; // (Maybe) How many symbols the smallest common Ast blocks will likely have

/// WHAT: project configuration loaded from config.moth that controls build behavior.
/// WHY: config is the control plane for the build system; it must be validated early
///      and provide precise error locations for all settings.
///
/// Design Principles:
/// - Config is loaded in Stage 0 before any compilation work begins
/// - All config keys are validated early so backends can reject invalid settings
/// - Source locations are tracked for precise error reporting
/// - Multi-error collection helps developers fix all issues in one iteration
///
/// Standard Config Keys:
/// - `entry_root`: The root directory for source files (default: "")
/// - `dev_folder`: Output directory for development builds (default: "dev")
/// - `output_folder`: Output directory for release builds (default: "release")
/// - `package_folders`: Top-level folders scanned for project-local source-backed packages (default: ["lib"])
/// - `project_name` or `name`: The project name
/// - `version`: The project version (default: "0.1.0")
/// - `author`: The project author
/// - `license`: The project license (default: "MIT")
///
/// Custom Keys:
/// - Backend-specific keys are stored in the `settings` HashMap
/// - Backends validate their own keys through `BackendBuilder::validate_project_config`
#[derive(Clone)]
pub struct Config {
    pub project_name: String,
    pub entry_dir: PathBuf,
    pub entry_root: PathBuf,
    pub dev_folder: PathBuf,
    pub release_folder: PathBuf,
    /// Top-level project folders scanned for project-local source-backed packages.
    pub package_folders: Vec<PathBuf>,
    /// Whether `package_folders` was explicitly configured in `config.moth`.
    pub has_explicit_package_folders: bool,
    /// Per-loop expansion limit for compile-time template loops.
    pub template_const_loop_iteration_limit: usize,
    pub version: String,
    pub author: String,
    pub license: String,
    /// Custom settings for any project builder to use
    pub settings: HashMap<String, String>,
    /// Source locations for each config key, used for precise error reporting
    pub setting_locations: HashMap<String, SourceLocation>,
}

impl Config {
    pub fn new(user_specified_path: PathBuf) -> Self {
        Config {
            entry_dir: user_specified_path,
            // These Default to the same directory as the project
            entry_root: PathBuf::from(""),
            dev_folder: PathBuf::from("dev"),
            release_folder: PathBuf::from("release"),

            package_folders: vec![PathBuf::from("lib")], // Default convention for project-local source-backed packages
            has_explicit_package_folders: false,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            project_name: String::new(),
            version: String::from("0.1.0"),
            author: String::new(),
            license: String::from("MIT"),

            // Custom settings for any project builder to use
            settings: HashMap::new(),
            setting_locations: HashMap::new(),
        }
    }

    /// Resolve the most specific location for a config key, falling back to `config.moth`.
    ///
    /// WHAT: uses the recorded setting location when available, otherwise creates a file-level
    /// location for the config file itself.
    /// WHY: config parsers should not duplicate fallback logic every time they report a bad value.
    pub fn setting_location_or_config_file(
        &self,
        key: &str,
        string_table: &mut StringTable,
    ) -> SourceLocation {
        self.setting_locations
            .get(key)
            .cloned()
            .unwrap_or_else(|| SourceLocation::from_path(&self.config_file_path(), string_table))
    }

    /// Build a typed project-config diagnostic with the standard setting location.
    ///
    /// WHAT: centralizes config-setting diagnostics on `Config`.
    /// WHY: parsers for routing/document/html settings should only define value semantics, not
    /// duplicate location lookup or boundary aggregation.
    pub fn config_diagnostic(
        &self,
        key: &str,
        reason: InvalidConfigReason,
        string_table: &mut StringTable,
    ) -> CompilerDiagnostic {
        let key_id = string_table.intern(key);
        CompilerDiagnostic::invalid_config_reason(
            Some(key_id),
            reason,
            self.setting_location_or_config_file(key, string_table),
        )
    }

    pub fn config_file_path(&self) -> PathBuf {
        self.entry_dir.join(CONFIG_FILE_NAME)
    }
}

/// Project-specific config validation can report user diagnostics or infrastructure failures.
///
/// WHAT: keeps backend/project config mistakes on the typed diagnostic path while preserving a
/// narrow escape hatch for filesystem/tooling failures discovered during validation.
/// WHY: `BackendBuilder::validate_project_config` is a build boundary, so callers need one result
/// type that can still distinguish normal user config feedback from internal failures.
#[derive(Debug, Clone)]
pub enum ProjectConfigError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

impl ProjectConfigError {
    pub fn into_messages(self, string_table: StringTable) -> CompilerMessages {
        match self {
            ProjectConfigError::Diagnostic(diagnostic) => {
                CompilerMessages::from_diagnostic(*diagnostic, string_table)
            }
            ProjectConfigError::Infrastructure(error) => {
                CompilerMessages::from_error(*error, string_table)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn diagnostic(&self) -> Option<&CompilerDiagnostic> {
        match self {
            ProjectConfigError::Diagnostic(diagnostic) => Some(diagnostic.as_ref()),
            ProjectConfigError::Infrastructure(_) => None,
        }
    }
}

impl From<CompilerDiagnostic> for ProjectConfigError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ProjectConfigError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<CompilerError> for ProjectConfigError {
    fn from(error: CompilerError) -> Self {
        ProjectConfigError::Infrastructure(Box::new(error))
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            entry_dir: PathBuf::new(),
            entry_root: PathBuf::from("src"),
            dev_folder: PathBuf::from("dev"),
            release_folder: PathBuf::from("release"),
            package_folders: vec![PathBuf::from("lib")],
            has_explicit_package_folders: false,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            project_name: String::from("html_project"),
            version: String::from("0.1.0"),
            author: String::new(),
            license: String::from("MIT"),
            settings: HashMap::new(),
            setting_locations: HashMap::new(),
        }
    }
}
