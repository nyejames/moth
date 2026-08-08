//! Stage 0 config loading, parsing, and validation for Moth projects.
//!
//! WHAT: owns the public entry points for loading `config.moth` before compilation starts.
//! WHY: callers only need one stable surface while parsing and validation details stay split by
//! concern in dedicated helpers.
mod parsing;
mod validation;

pub(crate) use validation::validate_directory_output_settings;

use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use std::path::Path;

// -------------------------
//  Config Parse Services
// -------------------------

/// Focused frontend services passed into config parsing so `config.moth` can use the selected
/// compiler and builder capability metadata.
///
/// WHAT: bundles the style directives and the complete builder surface (external packages,
/// source-backed packages, and config keys) that config parsing needs.
/// WHY: `bootstrap_project_build` already computes `BuilderSurface` before config loading. Threading
/// it through config parsing keeps selected capability metadata available while config import
/// syntax remains rejected by the config parser.
pub(crate) struct ProjectConfigParseServices<'a> {
    pub style_directives: &'a StyleDirectiveRegistry,
    pub frontend_surface: &'a BuilderSurface,
}

// -------------------------
//  Public API
// -------------------------

/// Load and validate the project config from `config.moth` before compilation begins (Stage 0).
///
/// Config files are optional. When present this delegates to the parser/validator pipeline and
/// applies all accepted settings directly to `config`.
pub fn load_project_config(
    config: &mut Config,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    let config_path = config.config_file_path();

    if !config_path.exists() {
        return validate_directory_output_settings_if_needed(config, string_table);
    }

    parse_project_config_file(config, &config_path, services, string_table)
}

// -------------------------
//  Internal Orchestration
// -------------------------

/// Parse `config.moth` and extract top-level constant declarations into the `Config` struct.
///
/// WHY: config uses normal Moth syntax, so Stage 0 keeps the tokenizer/header parser in the
/// loop and then applies a dedicated config-only validation pass.
pub(crate) fn parse_project_config_file(
    config: &mut Config,
    config_path: &Path,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    // 1. Run the specialized config parser.
    let mut parsed_config = parsing::parse_config_file(config_path, services, string_table)?;
    let mut errors = std::mem::take(&mut parsed_config.errors);

    // 2. Validate and apply the folded AST to the live Config object.
    if let Err(mut validation_errors) = validation::validate_and_apply_config_ast(
        config,
        &parsed_config,
        &services.frontend_surface.config_keys,
        string_table,
    ) {
        errors.append(&mut validation_errors);
    }

    // 3. Validate directory output settings after all config values are applied.
    let validated_output_settings = if config.entry_dir.is_dir() {
        match validate_directory_output_settings(config, string_table) {
            Ok(settings) => Some(settings),
            Err(mut output_errors) => {
                errors.append(&mut output_errors);
                None
            }
        }
    } else {
        None
    };

    // 4. Aggregate all errors into one CompilerMessages payload.
    if errors.is_empty() {
        Ok(validated_output_settings)
    } else {
        Err(CompilerMessages::from_diagnostics(
            errors,
            string_table.clone(),
        ))
    }
}

fn validate_directory_output_settings_if_needed(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    if !config.entry_dir.is_dir() {
        return Ok(None);
    }

    validate_directory_output_settings(config, string_table)
        .map(Some)
        .map_err(|diagnostics| {
            CompilerMessages::from_diagnostics(diagnostics, string_table.clone())
        })
}
