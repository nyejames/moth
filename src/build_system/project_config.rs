//! Stage 0 config loading and settings application for Moth projects.
//!
//! WHAT: owns the public entry points for loading `config.moth` before compilation starts: locating
//! the file, reading it, calling the compiler's config compilation service and applying the folded
//! values it returns.
//! WHY: which file is the project's config, what its keys mean and how accepted values reach
//! [`Config`] is build policy. The stage sequence that turns config source into folded values is
//! compiler-owned, so this module composes no frontend stage itself.
mod validation;

pub(crate) use validation::validate_directory_output_settings;

use crate::build_system::create_project_modules::extract_source_code;
use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::single_source_compilation::{
    ConfigCompilationRequest, compile_config_source,
};
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
/// it through config parsing keeps selected capability metadata available while config dependency
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
/// Config files are optional. When present this compiles the source through the compiler's config
/// service, then validates and applies all accepted settings directly to `config`.
pub fn load_project_config(
    config: &mut Config,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    let config_path = config.config_file_path();

    if !config_path.exists() {
        return validate_directory_output_settings_if_needed(config, string_table);
    }

    compile_project_config_file(config, &config_path, services, string_table)
}

// -------------------------
//  Internal Orchestration
// -------------------------

/// Compile `config.moth` and extract top-level constant declarations into the `Config` struct.
///
/// WHY: config uses normal Moth syntax, so Stage 0 hands the source to the compiler's config
/// compilation service and then applies a dedicated config-only validation pass to folded values.
pub(crate) fn compile_project_config_file(
    config: &mut Config,
    config_path: &Path,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    // 1. Compile the config source to folded values.
    let canonical_config_path = std::fs::canonicalize(config_path).map_err(|error| {
        CompilerMessages::from_error(
            CompilerError::file_error(
                config_path,
                format!("Failed to canonicalize config path: {error}"),
                string_table,
            ),
            string_table.clone(),
        )
    })?;
    let source_code = extract_source_code(&canonical_config_path, string_table)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
    let compiled_config = compile_config_source(
        ConfigCompilationRequest {
            authored_path: config_path,
            canonical_path: &canonical_config_path,
            source_code: &source_code,
            style_directives: services.style_directives,
            binding_packages: &services.frontend_surface.binding_packages,
        },
        string_table,
    )?;

    // 2. Validate and apply the folded AST to the live Config object.
    let mut errors = Vec::new();
    if let Err(mut validation_errors) = validation::validate_and_apply_config_ast(
        config,
        &compiled_config,
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
