//! Global compiler and project constants.
//!
//! WHAT: defines file extensions, reserved names, heuristic capacity constants, and project
//!       configuration structures shared across the compiler and build system.
//! WHY: keeping these values in one module prevents magic literals from spreading through the
//!      codebase and makes capacity tuning explicit.

use crate::compiler_frontend::build_config::ConfigResolutionRecord;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidConfigReason};
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::module_compilation::{
    DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS, FrontendOptions,
};
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

pub const INDEX_PAGE_NAME: &str = "index.html";
pub const CONFIG_FILE_NAME: &str = "config.moth";

/// Special reserved names for functions and variables created by the compiler
pub const TOP_LEVEL_TEMPLATE_NAME: &str = "#template";
pub const TOP_LEVEL_CONST_TEMPLATE_NAME: &str = "#const_template";
pub const IMPLICIT_START_FUNC_NAME: &str = "start";
pub const TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY: &str = "template_const_loop_iteration_limit";
pub const MAX_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 1_000_000;

// This is a guess about how much should be initially allocated for vecs in the compiler.
// This should be a rough guess to help avoid too many allocations
// and is just a heuristic based on tests with rudimentary small snippets of code.
// Should be recalculated at a later point.
pub const MINIMUM_STRING_TABLE_CAPACITY: usize = 32;
pub const SRC_TO_TOKEN_RATIO: usize = 5; // (Maybe) About 1/6 source code to tokens observed
pub const EXPORTS_CAPACITY: usize = 6; // (No Idea atm)
pub const TOKEN_TO_HEADER_RATIO: usize = 35; // (Maybe) About 1/35 tokens to AstNode ratio
pub const TOKEN_TO_DECLARATION_RATIO: usize = 20; // (Maybe) About 1/20 tokens for each new declaration symbol
pub const TOKEN_TO_NODE_RATIO: usize = 10; // (Maybe) About 1/10 tokens to AstNode ratio
pub const MINIMUM_LIKELY_DECLARATIONS: usize = 10; // (Maybe) How many symbols the smallest common Ast blocks will likely have

/// Typed results of one validated `html #= |...|` builder section.
///
/// WHAT: each html section field in its validated form. Fields the record omits stay `None`
/// unless their schema declares a default, which then stands.
/// WHY: this struct is the HTML builder settings store; the routing and document-shell readers
/// consume it directly. Directory HTML projects require the grouped `html` section; omitted
/// fields still receive schema defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlSectionConfig {
    /// Output roots owned by the builder section. The section schema's defaults (`dev` and
    /// `release`) land here whenever a section validated, so output resolution can consume
    /// them from the typed section.
    pub dev_output: Option<String>,
    pub release_output: Option<String>,
    pub origin: Option<String>,
    pub page_url_style: Option<String>,
    pub redirect_index_html: Option<bool>,
    pub html_lang: Option<String>,
    pub html_title_prefix: Option<String>,
    pub html_title_postfix: Option<String>,
    pub html_favicon: Option<String>,
    pub html_inject_charset: Option<bool>,
    pub html_inject_viewport: Option<bool>,
    pub html_inject_color_scheme: Option<bool>,
    pub html_inject_core_css: Option<bool>,
    pub html_body_style: Option<String>,
}

impl HtmlSectionConfig {
    /// Directory output roots a directory project uses when its `html` section omits them.
    /// The html section schema registers the same defaults, so an authored-but-omitted field
    /// resolves identically.
    pub const DEFAULT_DEV_OUTPUT: &'static str = "dev";
    pub const DEFAULT_RELEASE_OUTPUT: &'static str = "release";
}

/// One additional authored `project` field retained after compiler-owned fields are applied.
///
/// WHAT: the field's folded value plus the canonical type and initializer location needed
///       later by `@project`. Nested record values keep field types on `PublicFoldedField`;
///       exact nested field-name spans stay deferred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectMetadataField {
    pub name: String,
    pub type_identity: CanonicalTypeIdentity,
    pub value: PublicFoldedValue,
    pub location: SourceLocation,
}

/// WHAT: project configuration loaded from config.moth that controls build behavior.
/// WHY: config is the control plane for the build system; it must be validated early
///      and provide precise error locations for all settings.
#[derive(Clone)]
pub struct Config {
    pub project_name: String,
    pub entry_dir: PathBuf,
    pub entry_root: PathBuf,
    /// Per-loop expansion limit for compile-time template loops.
    pub template_const_loop_iteration_limit: usize,
    pub version: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,

    /// Source locations for each config key, used for precise error reporting
    pub setting_locations: HashMap<String, SourceLocation>,

    /// Validated results of the grouped `html` builder section when one was authored.
    pub html_section: HtmlSectionConfig,

    /// Additional open `project` fields retained for later `@project` publication.
    pub(crate) extra_project_fields: Vec<ProjectMetadataField>,
    /// Whether this config was loaded from an actual `config.moth` file. Synthetic single-file
    /// defaults must not become fixed project providers for source build-config contracts.
    pub(crate) project_config_loaded: bool,
    /// Direct project `#Config` resolution records retained only until build-boundary projection.
    /// Successful build results clear this transient bootstrap handoff.
    pub(crate) config_resolution_records: Vec<ConfigResolutionRecord>,
}

impl Config {
    pub fn new(user_specified_path: PathBuf) -> Self {
        Config {
            project_name: String::new(),
            entry_dir: user_specified_path,
            entry_root: PathBuf::from(""),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            version: None,
            author: None,
            license: None,
            setting_locations: HashMap::new(),
            html_section: HtmlSectionConfig::default(),
            extra_project_fields: Vec::new(),
            project_config_loaded: false,
            config_resolution_records: Vec::new(),
        }
    }

    /// Project the settings the compiler frontend consumes.
    ///
    /// WHY: the frontend must not read this configuration container. The project tool owns the
    ///      translation, so only the template loop ceiling crosses the boundary.
    pub(crate) fn frontend_options(&self) -> FrontendOptions {
        FrontendOptions {
            template_const_loop_iteration_limit: self.template_const_loop_iteration_limit,
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
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            project_name: String::from("html_project"),
            version: None,
            author: None,
            license: None,

            setting_locations: HashMap::new(),
            html_section: HtmlSectionConfig::default(),
            extra_project_fields: Vec::new(),
            project_config_loaded: false,
            config_resolution_records: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "tests/settings_tests.rs"]
mod tests;
