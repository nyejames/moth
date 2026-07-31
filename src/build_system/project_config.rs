//! Stage 0 config loading, parsing, and validation for Moth projects.
//!
//! WHAT: owns the public entry points for loading `config.moth` before compilation starts.
//! WHY: callers only need one stable surface while parsing and validation details stay split by
//! concern in dedicated helpers.

mod parsing;
mod validation;

use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use std::path::Path;

// -------------------------
//  Config Parse Services
// -------------------------

/// Focused frontend services passed into config parsing so `config.moth` can import from core and
/// Core or Builder packages.
///
/// WHAT: bundles the style directives and the complete builder surface (external packages,
/// source-backed packages, and config keys) that config parsing needs.
/// WHY: `bootstrap_project_build` already computes `BuilderSurface` before config loading; threading
/// it through config parsing lets imports resolve against builder/core surfaces instead of an
/// empty default registry.
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
) -> Result<(), CompilerMessages> {
    let load_total_start = crate::timing::start_pipeline_timing();

    let config_path = config.config_file_path();

    let file_exists_start = crate::timing::start_pipeline_timing();
    let config_exists = config_path.exists();
    log_stage_timing("config.file_exists_check", file_exists_start);

    if !config_exists {
        log_stage_timing("config.load_total", load_total_start);
        return Ok(());
    }

    let parse_start = crate::timing::start_pipeline_timing();
    let result = parse_project_config_file(config, &config_path, services, string_table);
    log_stage_timing("config.parse_project_config_file", parse_start);

    log_stage_timing("config.load_total", load_total_start);
    result
}

/// Record a config-stage timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY:  config loading and parsing use dotted `config.*` metric names through the
///      concise `timers` substrate. The start token is zero-sized when `timers`
///      is off, so regular builds do not read clocks for instrumentation-only
///      measurements.
fn log_stage_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
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
) -> Result<(), CompilerMessages> {
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
    if let Err(mut output_errors) =
        validation::validate_directory_output_settings(config, string_table)
    {
        errors.append(&mut output_errors);
    }

    // 4. Aggregate all errors into one CompilerMessages payload.
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CompilerMessages::from_diagnostics(
            errors,
            string_table.clone(),
        ))
    }
}
