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
use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{BoundaryFailure, CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidConfigReason, PreparedFailure,
};
use crate::compiler_frontend::single_source_compilation::{
    CompiledConfigSource, ConfigCompilationOutcome, ConfigCompilationRequest, compile_config_source,
};
use crate::compiler_frontend::source::{SourceDatabase, SourceDatabaseError, SourceKind};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) enum ProjectConfigFailure {
    Legacy(CompilerMessages),
    Boundary(BoundaryFailure),
}

impl From<CompilerMessages> for ProjectConfigFailure {
    fn from(messages: CompilerMessages) -> Self {
        Self::Legacy(messages)
    }
}

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
    project_source_files: &mut Option<SourceDatabase>,
) -> Result<Option<ValidatedDirectoryOutputSettings>, ProjectConfigFailure> {
    let config_path = config.config_file_path();

    if !config_path.exists() {
        // A failed reload must not leave prior project-global/config provenance visible to the
        // next frontend invocation, even when the missing file is fatal for a directory project.
        config.project_config_loaded = false;
        config.config_resolution_records.clear();
        config.project_field_config_dependencies.clear();
        config.extra_project_fields.clear();
        if config.entry_dir.is_dir() {
            return Err(CompilerMessages::from_diagnostic(
                CompilerDiagnostic::invalid_config_reason(
                    Some(string_table.intern("config.moth")),
                    InvalidConfigReason::MissingConfigFile,
                    None,
                ),
                string_table.clone(),
            )
            .into());
        }

        return validate_directory_output_settings_if_needed(config, string_table)
            .map_err(Into::into);
    }
    if project_source_files.is_none() {
        return Err(CompilerMessages::from_error(
            CompilerError::compiler_error(
                "config.moth exists but the project source database is absent",
            ),
            string_table.clone(),
        )
        .into());
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
    project_source_files: &mut Option<SourceDatabase>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, ProjectConfigFailure> {
    config.project_config_loaded = false;
    config.config_resolution_records.clear();
    config.project_field_config_dependencies.clear();
    config.extra_project_fields.clear();
    config.setting_spans.clear();
    config.html_section = crate::projects::settings::HtmlSectionConfig::default();
    let source_database = project_source_files.as_mut().ok_or_else(|| {
        CompilerMessages::from_error(
            CompilerError::compiler_error("project config source database is absent"),
            string_table.clone(),
        )
    })?;
    let canonical_config_path = std::fs::canonicalize(config_path).map_err(|error| {
        CompilerMessages::from_error(
            CompilerError::file_error(
                config_path,
                format!("Failed to canonicalize config path: {error}"),
            ),
            string_table.clone(),
        )
    })?;

    // Register config before tokenization so its retained syntax and diagnostics share the
    // project identity context with every source discovered later. Config sits outside the entry
    // root, so its own directory roots the logical path and yields a bare `config.moth`.
    let config_file_id = source_database
        .insert(
            canonical_config_path.clone(),
            SourceKind::Compiler(SourceFileKind::Moth),
            &canonical_config_path,
            None,
            string_table,
        )
        .map_err(|error| match error {
            SourceDatabaseError::Capacity(capacity) => CompilerMessages::from_diagnostic(
                CompilerDiagnostic::source_table_capacity(capacity.resource()),
                string_table.clone(),
            ),
            SourceDatabaseError::Infrastructure(error) => {
                CompilerMessages::from_error_ref(error, string_table)
            }
        })?;

    let source_code = match extract_source_code(&canonical_config_path, string_table) {
        Ok(source_code) => source_code,
        Err(error) => {
            source_database
                .record_source_load_error(config_file_id, error.clone())
                .map_err(|slot_error| {
                    CompilerMessages::from_error(slot_error, string_table.clone())
                })?;
            return Err(CompilerMessages::from_error(error, string_table.clone()).into());
        }
    };
    source_database
        .retain_text(config_file_id, source_code)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
    let source_code = source_database
        .retained_text(config_file_id)
        .ok_or_else(|| {
            CompilerMessages::from_error(
                CompilerError::compiler_error(
                    "registered config source lost its retained source text",
                ),
                string_table.clone(),
            )
        })?;
    let ConfigCompilationOutcome {
        result,
        file_id: outcome_file_id,
        span_builder,
    } = compile_config_source(
        ConfigCompilationRequest {
            authored_path: config_path,
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
    );
    let mut compiler_failure = None;
    let validated_config = match result {
        Ok(compiled_config) => Some(
            validate_and_apply_compiled_config(config, &compiled_config, services, string_table)
                .map(|settings| (compiled_config, settings))
                .map_err(ProjectConfigFailure::Legacy),
        ),
        Err(failure) => {
            compiler_failure = Some(failure);
            None
        }
    };

    // Both compiler and build-owned diagnostics are complete before the source becomes immutable.
    source_database
        .install_extended_spans(outcome_file_id, span_builder.freeze())
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    if let Some(failure) = compiler_failure {
        let source_database = project_source_files
            .take()
            .expect("the config source owner remains available through span finalization");
        return Err(ProjectConfigFailure::Boundary(
            finalize_compiler_diagnostic_failure(failure, source_database, string_table),
        ));
    }

    let validated_config =
        validated_config.expect("a non-AST config result has a validation outcome");
    let (compiled_config, validated_output_settings) = validated_config?;

    // Retain compiler-owned provenance until the build boundary projects source providers,
    // receiving fields and the private input-contract records.
    config.config_resolution_records = compiled_config.resolution_records;
    config.project_field_config_dependencies = compiled_config.project_field_dependencies;
    config.project_config_loaded = true;
    Ok(validated_output_settings)
}

fn finalize_compiler_diagnostic_failure(
    failure: PreparedFailure,
    source_database: SourceDatabase,
    string_table: &StringTable,
) -> BoundaryFailure {
    let source_database = Arc::new(source_database);
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            let mut accumulator = BoundaryAccumulator::new();
            accumulator.append_prepared(batch, Some((source_database, None)));
            accumulator.finish(InstallMode::Frozen)
        }
        PreparedFailure::Infrastructure(errors) => {
            let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
            accumulator.append_infrastructure(errors, Some(source_database));
            accumulator.finish(InstallMode::Frozen)
        }
        PreparedFailure::Mixed { batch, error } => {
            let mut accumulator = BoundaryAccumulator::new();
            accumulator.append_prepared(batch, Some((Arc::clone(&source_database), None)));
            accumulator.append_infrastructure(error, Some(source_database));
            accumulator.finish(InstallMode::Frozen)
        }
    }
}

fn validate_and_apply_compiled_config(
    config: &mut Config,
    compiled_config: &CompiledConfigSource,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, CompilerMessages> {
    let mut errors = Vec::new();
    match validation::validate_and_apply_config_declarations(
        config,
        compiled_config,
        &services.frontend_surface.config_schemas,
        string_table,
    ) {
        Ok(()) => {}
        Err(ConfigApplyError::Diagnostics(mut validation_errors)) => {
            errors.append(&mut validation_errors);
        }
        Err(ConfigApplyError::Compiler(error)) => {
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    }

    // Output policy needs the completed config values, even when another field was diagnosed.
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
