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
use crate::build_system::create_project_modules::source_loading::{
    SourceReadError, registered_source_read_error, retained_source_error,
};
use crate::build_system::output::{OutputRootSettings, ValidatedDirectoryOutputSettings};
use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    project_config, token_lexical,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, InvalidConfigReason, PreparedFailure,
};
use crate::compiler_frontend::module_compilation::{
    ConstExecutionLimits, DEFAULT_MAX_ERRORS_PER_MODULE,
};
use crate::compiler_frontend::single_source_compilation::{
    CompiledConfigSource, ConfigCompilationOutcome, ConfigCompilationRequest, compile_config_source,
};
use crate::compiler_frontend::source::{
    SourceDatabase, SourceDatabaseBuilder, SourceDatabaseError, SourceKind,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{BuilderSettings, Config, DEFAULT_MAX_ERRORS_PER_PROJECT};

use moth_lexical::identifier::is_identifier;
use std::path::Path;
use std::sync::Arc;

pub(crate) enum ProjectConfigFailure {
    Prepared(PreparedFailure),
    Diagnosed(DiagnosticReportSet),
    Infrastructure(InfrastructureErrors),
}

impl From<PreparedFailure> for ProjectConfigFailure {
    fn from(failure: PreparedFailure) -> Self {
        Self::Prepared(failure)
    }
}

fn append_config_source_finalization_failure(
    failure: PreparedFailure,
    finish_error: InfrastructureFailure,
) -> PreparedFailure {
    match failure {
        PreparedFailure::Diagnosed(_) => PreparedFailure::Infrastructure(finish_error.into()),
        PreparedFailure::Infrastructure(mut errors) => {
            errors.push(finish_error);
            PreparedFailure::Infrastructure(errors)
        }
    }
}

fn capture_root_diagnostics(
    diagnostics: Vec<DiagnosticDraft>,
    root_string_table: &StringTable,
) -> PreparedFailure {
    match PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from_diagnostics(diagnostics),
        root_string_table,
        None,
        || None,
    ) {
        Ok(batch) => PreparedFailure::Diagnosed(batch),
        Err(error) => PreparedFailure::from(error),
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
/// Directory projects treat `config.moth` as optional. Single-file compilation keeps its separate
/// no-config policy because `entry_dir` is not a directory.
pub fn load_project_config(
    config: &mut Config,
    services: &ProjectConfigParseServices<'_>,
    project_source_builder: &mut Option<SourceDatabaseBuilder>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, ProjectConfigFailure> {
    let config_path = config.config_file_path();

    // Single-file compilation uses its separate synthetic path and must not probe a
    // `<file>/config.moth` path. Directory projects alone select an authored config.
    if !config.entry_dir.is_dir() {
        reset_project_config_state(config);
        return Ok(None);
    }

    match std::fs::symlink_metadata(&config_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            reset_project_config_state(config);

            if config.entry_dir.is_dir() {
                let defaults_result = {
                    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
                    validation::apply_defaults_without_config(
                        config,
                        &services.frontend_surface.config_schemas,
                        &mut diagnostic_strings,
                    )
                };
                if let Err(ConfigApplyError::Diagnostics(errors)) = defaults_result {
                    return Err(ProjectConfigFailure::Prepared(capture_root_diagnostics(
                        errors,
                        string_table,
                    )));
                }
                if let Err(ConfigApplyError::Compiler(error)) = defaults_result {
                    return Err(ProjectConfigFailure::Prepared(PreparedFailure::from(error)));
                }
                apply_directory_name_default(config, string_table)
                    .map_err(ProjectConfigFailure::Prepared)?;
            }

            return validate_directory_output_settings_if_needed(config, string_table)
                .map_err(ProjectConfigFailure::Prepared);
        }
        Err(error) => {
            return Err(PreparedFailure::from(InfrastructureFailure::filesystem(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                &config_path,
                format!("Failed to inspect config path: {error}"),
            ))
            .into());
        }
    }

    if project_source_builder.is_none() {
        return Err(PreparedFailure::from(InfrastructureFailure::internal(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            "config.moth exists but the project source database is absent",
        ))
        .into());
    }

    let outputs = compile_project_config_file(
        config,
        &config_path,
        services,
        project_source_builder,
        string_table,
    )?;
    if config.entry_dir.is_dir() {
        apply_directory_name_default(config, string_table)
            .map_err(ProjectConfigFailure::Prepared)?;
    }
    Ok(outputs)
}

/// Clear authored project/config state so one reload cannot retain another config's facts.
fn reset_project_config_state(config: &mut Config) {
    config.project_name.clear();
    config.version = None;
    config.author = None;
    config.license = None;
    config.config_resolution_records.clear();
    config.project_field_config_dependencies.clear();
    config.extra_project_fields.clear();
    config.setting_spans.clear();
    config.output_roots = OutputRootSettings::default();
    config.builder_settings = BuilderSettings::default();
    config.max_errors_per_module = Some(DEFAULT_MAX_ERRORS_PER_MODULE);
    config.max_errors_per_project = Some(DEFAULT_MAX_ERRORS_PER_PROJECT);
    config.const_execution_limits = ConstExecutionLimits::default();
}

/// Apply the selected directory name as the default project identity.
///
/// WHAT: when no explicit `project.name` was folded, use the exact final component of the selected
/// directory without rewriting its spelling. An invalid identifier is a user diagnostic rather
/// than a silently sanitised name.
/// WHY: directory compilation is valid without `config.moth`, while stable project identity still
/// needs one canonical name. Scaffolding writes an explicit valid name so this fallback is mostly
/// invisible.
fn apply_directory_name_default(
    config: &mut Config,
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    if !config.project_name.is_empty() {
        return Ok(());
    }

    // The selected directory may be spelled `.` or relatively; canonicalize only to read its
    // final component. The name never derives from an absolute path spelling.
    let selected_directory =
        std::fs::canonicalize(&config.entry_dir).unwrap_or_else(|_| config.entry_dir.clone());
    let directory_name = selected_directory
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty());

    let Some(directory_name) = directory_name else {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
        return Err(capture_root_diagnostics(
            vec![project_config::invalid_config_reason(
                Some(diagnostic_strings.intern("name")),
                InvalidConfigReason::InvalidProjectSettingValue {
                    value: diagnostic_strings.intern("<selected directory>"),
                    expected: diagnostic_strings.intern(
                        "set project.name in config.moth to a valid Moth project identifier",
                    ),
                },
                None,
            )],
            diagnostic_strings.strings(),
        ));
    };

    if !is_identifier(directory_name) {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
        return Err(capture_root_diagnostics(
            vec![project_config::invalid_config_reason(
                Some(diagnostic_strings.intern("name")),
                InvalidConfigReason::InvalidProjectSettingValue {
                    value: diagnostic_strings.intern(directory_name),
                    expected: diagnostic_strings.intern(
                        "set project.name in config.moth to a valid Moth project identifier",
                    ),
                },
                None,
            )],
            diagnostic_strings.strings(),
        ));
    }

    config.project_name = directory_name.to_owned();
    Ok(())
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
    project_source_builder: &mut Option<SourceDatabaseBuilder>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, ProjectConfigFailure> {
    reset_project_config_state(config);
    let source_builder = project_source_builder.as_mut().ok_or_else(|| {
        PreparedFailure::from(InfrastructureFailure::internal(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            "project config source database is absent",
        ))
    })?;
    let canonical_config_path = std::fs::canonicalize(config_path).map_err(|error| {
        PreparedFailure::from(InfrastructureFailure::filesystem(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            config_path,
            format!("Failed to canonicalize config path: {error}"),
        ))
    })?;
    let canonical_project_root = std::fs::canonicalize(&config.entry_dir).map_err(|error| {
        PreparedFailure::from(InfrastructureFailure::filesystem(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            &config.entry_dir,
            format!("Failed to canonicalize selected project directory: {error}"),
        ))
    })?;
    if !canonical_config_path.starts_with(&canonical_project_root) {
        return Err(PreparedFailure::from(InfrastructureFailure::filesystem(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            &canonical_config_path,
            "config.moth resolves outside the selected project directory; project config must not be linked from outside the build root",
        ))
        .into());
    }

    // Register config before tokenization so its retained syntax and diagnostics share the
    // project identity context with every source discovered later. Config lives at the selected
    // project root, so its own directory roots the logical path and yields a bare `config.moth`.
    let config_file_id = source_builder
        .sources_mut()
        .insert(
            canonical_config_path.clone(),
            SourceKind::Compiler(SourceFileKind::Moth),
            &canonical_config_path,
            None,
            string_table,
        )
        .map_err(|error| match error {
            SourceDatabaseError::Capacity(capacity) => capture_root_diagnostics(
                vec![token_lexical::source_span_capacity(
                    0,
                    u32::MAX,
                    capacity.resource(),
                    None,
                )],
                string_table,
            ),
            SourceDatabaseError::Infrastructure(error) => PreparedFailure::from(error),
        })?;

    let mut path_fork = PathInternerFork::empty();
    let source_code = match extract_source_code(&canonical_config_path) {
        Ok(source_code) => source_code,
        Err(SourceReadError::Infrastructure(infrastructure)) => {
            source_builder
                .sources_mut()
                .record_source_load_error(config_file_id, infrastructure.clone())
                .map_err(PreparedFailure::from)?;
            return Err(PreparedFailure::from(infrastructure).into());
        }
        Err(SourceReadError::NonUtf8) => {
            let failure = registered_source_read_error(
                SourceReadError::NonUtf8,
                config_file_id,
                source_builder.sources(),
                &mut path_fork,
            );
            return Err(failure
                .into_failure(string_table, Arc::new(path_fork.shared_table()))
                .into());
        }
    };
    if let Err(error) = source_builder
        .sources_mut()
        .retain_text(config_file_id, source_code)
    {
        let failure = retained_source_error(
            error,
            config_file_id,
            source_builder.sources(),
            &mut path_fork,
        );
        return Err(failure
            .into_failure(string_table, Arc::new(path_fork.shared_table()))
            .into());
    }
    let source_code = source_builder
        .sources()
        .retained_text(config_file_id)
        .ok_or_else(|| {
            PreparedFailure::from(InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                "registered config source lost its retained source text",
            ))
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
                .map_err(ProjectConfigFailure::Prepared),
        ),
        Err(failure) => {
            compiler_failure = Some(failure);
            None
        }
    };

    // The config producer is done. Success keeps the owner live for later project sources; a
    // build-owned config failure returns it to bootstrap for terminal finalization. Compiler
    // diagnostics are terminal immediately and consume the owner below.
    source_builder.retain_span_builder(outcome_file_id, span_builder);
    if let Err(finish_error) = source_builder.finish_source(outcome_file_id) {
        if let Some(failure) = compiler_failure {
            return Err(ProjectConfigFailure::Prepared(
                append_config_source_finalization_failure(failure, finish_error),
            ));
        }
        return match validated_config {
            Some(Err(ProjectConfigFailure::Prepared(failure))) => {
                Err(ProjectConfigFailure::Prepared(
                    append_config_source_finalization_failure(failure, finish_error),
                ))
            }
            Some(Err(failure)) => Err(failure),
            Some(Ok(_)) | None => Err(PreparedFailure::from(finish_error).into()),
        };
    }

    if let Some(failure) = compiler_failure {
        if let Err(finish_error) = source_builder.assemble_finalized_tables() {
            return Err(ProjectConfigFailure::Prepared(
                append_config_source_finalization_failure(failure, finish_error),
            ));
        }
        let source_owner = project_source_builder
            .take()
            .expect("the config source owner remains available through span finalization");
        let source_database = match source_owner.finish() {
            Ok(source_database) => source_database,
            Err(finish_error) => {
                return Err(ProjectConfigFailure::Prepared(
                    append_config_source_finalization_failure(failure, finish_error),
                ));
            }
        };
        return match finalize_compiler_diagnostic_failure(
            failure,
            source_database,
            string_table.take_for_handoff(),
        ) {
            Ok(reports) => Err(ProjectConfigFailure::Diagnosed(reports)),
            Err(errors) => Err(ProjectConfigFailure::Infrastructure(errors)),
        };
    }

    let validated_config =
        validated_config.expect("a successful config compilation has a validation outcome");
    let (compiled_config, validated_output_settings) = validated_config?;

    // Retain compiler-owned provenance until the build boundary projects source providers,
    // receiving fields and the private input-contract records.
    config.config_resolution_records = compiled_config.resolution_records;
    config.project_field_config_dependencies = compiled_config.project_field_dependencies;
    Ok(validated_output_settings)
}

fn finalize_compiler_diagnostic_failure(
    failure: PreparedFailure,
    source_database: SourceDatabase,
    string_table: StringTable,
) -> Result<DiagnosticReportSet, InfrastructureErrors> {
    let source_database = Arc::new(source_database);
    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            accumulator.append_prepared(batch, Some((source_database, None)));
        }
        PreparedFailure::Infrastructure(errors) => {
            accumulator.append_infrastructure(errors);
        }
    }
    accumulator.finish(InstallMode::Frozen)
}

fn validate_and_apply_compiled_config(
    config: &mut Config,
    compiled_config: &CompiledConfigSource,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, PreparedFailure> {
    let mut errors = Vec::new();
    let validation_result = {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
        validation::validate_and_apply_config_declarations(
            config,
            compiled_config,
            &services.frontend_surface.config_schemas,
            &mut diagnostic_strings,
        )
    };
    match validation_result {
        Ok(()) => {}
        Err(ConfigApplyError::Diagnostics(mut validation_errors)) => {
            errors.append(&mut validation_errors);
        }
        Err(ConfigApplyError::Compiler(error)) => {
            return Err(PreparedFailure::from(error));
        }
    }

    // Output policy needs the completed config values, even when another field was diagnosed.
    let validated_output_settings = if config.entry_dir.is_dir() {
        let output_settings = {
            let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
            validate_directory_output_settings(config, &mut diagnostic_strings)
        };
        match output_settings {
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
        Err(capture_root_diagnostics(errors, string_table))
    }
}

fn validate_directory_output_settings_if_needed(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<Option<ValidatedDirectoryOutputSettings>, PreparedFailure> {
    if !config.entry_dir.is_dir() {
        return Ok(None);
    }

    let validation_result = {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
        validate_directory_output_settings(config, &mut diagnostic_strings)
    };
    validation_result
        .map(Some)
        .map_err(|diagnostics| capture_root_diagnostics(diagnostics, string_table))
}
