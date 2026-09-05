//! Stage 0 config loading and settings application for Moth projects.
//!
//! WHAT: owns the public entry points for loading `config.moth` before compilation starts: locating
//! the file, reading it, calling the compiler's config compilation service and applying the folded
//! values it returns.
//! WHY: which file is the project's config, what its keys mean and how accepted values reach
//! [`Config`] is build policy. The stage sequence that turns config source into folded values is
//! compiler-owned, so this module composes no frontend stage itself.
mod validation;

use validation::ConfigApplyError;
pub(crate) use validation::validate_directory_output_settings;

use crate::build_system::create_project_modules::extract_source_code;
use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::single_source_compilation::{
    ConfigCompilationRequest, compile_config_source,
};
use crate::compiler_frontend::source::SourceDatabase;
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
    /// Typed explicit inputs supplied by the command or programmatic build caller.
    pub build_config_inputs: &'a BuildConfigInputSet,
}

// -------------------------
//  Public API
// -------------------------

/// Load and validate the project config from `config.moth` before compilation begins (Stage 0).
///
/// Directory projects require `config.moth`. Single-file compilation keeps its separate
/// no-config policy because `entry_dir` is not a directory.
pub fn load_project_config(
    config: &mut Config,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
    project_source_files: Option<&mut SourceDatabase>,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    let config_path = config.config_file_path();

    if !config_path.exists() {
        // A failed reload must not leave prior project-global/config provenance visible to the
        // next frontend invocation, even when the missing file is fatal for a directory project.
        config.project_config_loaded = false;
        config.config_resolution_records.clear();
        config.extra_project_fields.clear();
        if config.entry_dir.is_dir() {
            return Err(CompilerMessages::from_diagnostic(
                config.config_diagnostic(
                    "config.moth",
                    InvalidConfigReason::MissingConfigFile,
                    string_table,
                ),
                string_table.clone(),
            ));
        }

        return validate_directory_output_settings_if_needed(config, string_table);
    }

    compile_project_config_file(
        config,
        &config_path,
        services,
        project_source_files,
        string_table,
    )
}

// -------------------------
//  Internal Orchestration
// -------------------------

/// Compile `config.moth` and apply folded top-level constant declarations to the `Config` struct.
///
/// WHY: config uses normal Moth syntax, so Stage 0 hands the source to the compiler's config
/// compilation service and then applies a dedicated config-only validation pass to the folded
/// declarations it returns.
pub(crate) fn compile_project_config_file(
    config: &mut Config,
    config_path: &Path,
    services: &ProjectConfigParseServices<'_>,
    mut project_source_files: Option<&mut SourceDatabase>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    // A failed reload must not leave prior project-global/config provenance visible to the next
    // frontend invocation.
    config.project_config_loaded = false;
    config.config_resolution_records.clear();
    config.extra_project_fields.clear();
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

    // Register config before tokenization so its retained syntax and diagnostics share the
    // project identity context with every source discovered later. Config sits outside the entry
    // root, so its own directory roots the logical path and yields a bare `config.moth`.
    let config_file_id = project_source_files
        .as_deref_mut()
        .map(|source_files| {
            source_files.insert(
                canonical_config_path.clone(),
                &canonical_config_path,
                None,
                string_table,
            )
        })
        .transpose()
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    let source_code = match extract_source_code(&canonical_config_path, string_table) {
        Ok(source_code) => source_code,
        Err(error) => {
            if let (Some(source_files), Some(config_file_id)) =
                (project_source_files.as_deref_mut(), config_file_id)
            {
                source_files
                    .record_source_load_error(config_file_id, error.clone())
                    .map_err(|slot_error| {
                        CompilerMessages::from_error(slot_error, string_table.clone())
                    })?;
            }
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };
    let source_code = match (project_source_files, config_file_id) {
        (Some(source_files), Some(config_file_id)) => {
            source_files
                .retain_text(config_file_id, source_code)
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
            source_files.retained_text(config_file_id).ok_or_else(|| {
                CompilerMessages::from_error(
                    CompilerError::compiler_error(
                        "registered config source lost its retained source text",
                    ),
                    string_table.clone(),
                )
            })?
        }
        (_, _) => source_code.as_str(),
    };
    let compiled_config = compile_config_source(
        ConfigCompilationRequest {
            authored_path: config_path,
            canonical_path: &canonical_config_path,
            file_id: config_file_id,
            source_code,
            style_directives: services.style_directives,
            binding_packages: &services.frontend_surface.binding_packages,
            build_config_inputs: services.build_config_inputs,
            builder_config_globals: services.frontend_surface.config_globals(),
            project_field_config_policies: services
                .frontend_surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        string_table,
    )?;

    let mut errors = Vec::new();
    // 2. Validate and apply the folded declarations to the live Config object.
    match validation::validate_and_apply_config_declarations(
        config,
        &compiled_config,
        &services.frontend_surface.config_schemas,
        string_table,
    ) {
        Ok(()) => {}
        Err(ConfigApplyError::Diagnostics(mut validation_errors)) => {
            errors.append(&mut validation_errors);
        }
        Err(ConfigApplyError::Compiler(error)) => {
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
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
        // Keep compiler-owned resolution provenance with the live bootstrap config until the
        // build boundary projects source providers and `@project`. Successful build results clear
        // this transient handoff after frontend compilation.
        config.config_resolution_records = compiled_config.resolution_records;
        config.project_config_loaded = true;
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
