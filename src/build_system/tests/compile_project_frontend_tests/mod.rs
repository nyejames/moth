// Shared imports and fixture helpers for focused frontend test modules.
use super::{
    FrontendCompilationMode, compile_project_frontend, compile_project_frontend_with_inputs,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BackendBuilder, Project, ProjectCompilation, build_project};
use crate::build_system::create_project_modules::compiled_boundary::ProjectFrontendCompilation;
use crate::build_system::create_project_modules::create_project_modules_tests::successful_compilation_value;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::CleanupPolicy;
use crate::build_system::project_config::{ProjectConfigParseServices, load_project_config};
use crate::builder_surface::BuilderSurface;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, ResolvedExternalImport,
    RuntimeAssetIdentity,
};
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigValueLocation,
    BuildInputName, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::{
    BoundaryAccumulator, SuccessfulWarningBoundary,
};
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compilation_result_html;
use crate::compiler_frontend::compiler_messages::render::terse;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::external_packages::external_returns;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAbiType, ExternalAccessKind, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalFunctionSpec, ExternalJsLowering, ExternalReturnSlot, ExternalSignatureType,
    ExternalTypeId, ExternalTypeSpec,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::module_compilation::CompiledModuleArtifact;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::public_call_summary::PublicCallMutationEffect;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{SourceDatabase, SourceDatabaseBuilder};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::{Config, ProjectConfigError};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "timers")]
fn boundary_has_timing(
    snapshot: &crate::timing::BenchmarkObservationSnapshot,
    boundary: crate::timing::TimingBoundaryId,
    metric: crate::timing::TimingMetric,
) -> bool {
    snapshot
        .boundaries
        .iter()
        .find(|record| record.id == boundary)
        .is_some_and(|record| {
            record
                .timings
                .iter()
                .any(|aggregate| aggregate.metric == metric && aggregate.samples > 0)
        })
}

#[cfg(feature = "timers")]
fn module_has_timing(
    module: &crate::timing::TimingModuleRecord,
    metric: crate::timing::TimingMetric,
) -> bool {
    module
        .timings
        .iter()
        .any(|aggregate| aggregate.metric == metric && aggregate.samples > 0)
}
fn assert_has_diagnostic_code<T>(result: &CompilationResult<T>, expected_code: &str) {
    let reports = super::create_project_modules_tests::diagnosed_reports(result);
    let actual_codes = reports
        .iter()
        .map(|(_, view)| view.descriptor().code)
        .collect::<Vec<_>>();

    assert!(
        actual_codes.contains(&expected_code),
        "expected diagnostic code {expected_code}, got {actual_codes:?}"
    );
}
const CONFIG_SOURCE: &str = "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n";

/// Writes one directory project from `(relative path, source text)` files.
///
/// The fixture supplies a default `config.moth` unless the row provides its own.
fn write_project_fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
    let temporary_directory = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary_directory.path().to_path_buf();
    fs::create_dir_all(project_root.join("src")).expect("should create project source root");
    if !files
        .iter()
        .any(|(relative_path, _)| *relative_path == "config.moth")
    {
        fs::write(project_root.join("config.moth"), CONFIG_SOURCE)
            .expect("should write project config");
    }
    for (relative_path, source) in files {
        let path = project_root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("should create fixture source directory");
        }
        fs::write(path, source).expect("should write fixture source");
    }
    temporary_directory
}

/// Compiles one directory project from the given `(relative path, source text)` files.
fn compile_fixture(files: &[(&str, &str)]) -> ProjectFrontendCompilation {
    successful_compilation_value(compile_fixture_result(files, false, BuildProfile::Dev))
}

/// Compiles one `.mtf`-bearing directory project through the real HTML builder surface.
///
/// WHY: these fixtures exercise HTML output and slots, so they compile through the same surface a
///      real HTML project uses rather than a bare mandatory-core surface. `.mtf` support itself
///      is compiler-owned and no longer depends on this builder.
fn compile_template_fixture(files: &[(&str, &str)]) -> ProjectFrontendCompilation {
    compile_template_fixture_with_profile(files, BuildProfile::Dev)
}

/// Compiles one template fixture under an explicit build profile.
fn compile_template_fixture_with_profile(
    files: &[(&str, &str)],
    profile: BuildProfile,
) -> ProjectFrontendCompilation {
    successful_compilation_value(compile_fixture_result(files, true, profile))
}

/// Freezes the diagnostics of a `.mtf`-bearing project that must be rejected as user source.
fn compile_template_fixture_reports(files: &[(&str, &str)]) -> DiagnosticReportSet {
    compile_template_fixture_reports_with_profile(files, BuildProfile::Dev)
}

/// Freezes the diagnostics of a template fixture under an explicit build profile.
fn compile_template_fixture_reports_with_profile(
    files: &[(&str, &str)],
    profile: BuildProfile,
) -> DiagnosticReportSet {
    match compile_fixture_result(files, true, profile) {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        Ok(CompilationOutcome::Success { .. }) => {
            panic!("expected qualified-reference diagnostics")
        }
        Err(errors) => {
            panic!("user-authored qualified input must not reach infrastructure errors: {errors:?}")
        }
    }
}

fn compile_fixture_result(
    files: &[(&str, &str)],
    template_surface: bool,
    profile: BuildProfile,
) -> CompilationResult<ProjectFrontendCompilation> {
    let temporary_directory = write_project_fixture(files);
    let mut config = Config::new(temporary_directory.path().to_path_buf());
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut frontend_surface = if template_surface {
        HtmlProjectBuilder::new().frontend_surface()
    } else {
        BuilderSurface::with_mandatory_core()
    };
    compile_project_frontend(
        &mut config,
        profile,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    )
}

/// Compiles one fixture and freezes its successful warning/Note lane.
///
/// WHY: the normal fixture helper asserts diagnostics, while a release budget Note is a successful
///      outcome. This helper keeps the same project construction and returns the frozen warning
///      reports beside the compiled value.
fn compile_fixture_with_warnings(
    files: &[(&str, &str)],
    profile: BuildProfile,
) -> (ProjectFrontendCompilation, Option<DiagnosticReportSet>) {
    let temporary_directory = write_project_fixture(files);
    let mut config = Config::new(temporary_directory.path().to_path_buf());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let build_config_inputs = BuildConfigInputSet::new();
    let mut frontend_surface = HtmlProjectBuilder::new().frontend_surface();
    let services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let mut string_table = StringTable::new();
    let mut source_builder = Some(SourceDatabaseBuilder::new(SourceDatabase::empty()));
    let validated_output_settings = load_project_config(
        &mut config,
        &services,
        &mut source_builder,
        &mut string_table,
    )
    .unwrap_or_else(|_| panic!("project config should load"));
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let compilation = compile_project_frontend(
        &mut config,
        profile,
        validated_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    )
    .expect("fixture compilation should not fail infrastructure");
    let compilation = match compilation {
        CompilationOutcome::Success { value, .. } => value,
        CompilationOutcome::Diagnosed(reports) => {
            panic!(
                "the fixture must compile successfully with at most budget Notes: {:?}",
                terse::format_terse_diagnostic_reports(&reports)
            )
        }
    };
    warning_owner.commit_successful_warnings(SuccessfulWarningBoundary::Project, None);
    match warning_owner
        .finish_successful_warnings(compilation)
        .expect("successful warnings should freeze")
    {
        CompilationOutcome::Success { value, warnings } => (value, warnings),
        CompilationOutcome::Diagnosed(_) => {
            panic!("the fixture must compile successfully with at most budget Notes")
        }
    }
}

/// Returns the artifact that owns the fixture's single entry root.
fn entry_artifact(frontend: &ProjectFrontendCompilation) -> &CompiledModuleArtifact {
    let mut entries = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter(|artifact| artifact.module.metadata.entry.is_some());
    let artifact = entries
        .next()
        .expect("the fixture should compile one entry module");
    assert!(
        entries.next().is_none(),
        "the fixture should own exactly one entry root"
    );
    artifact
}

/// Returns every fully folded textual const fragment of the entry root in authored order.
///
/// WHY: a qualified reference must fold before backend lowering, so the retained const fragment is
///      the earliest observable dependent content. A resource-bearing value has no text until the
///      builder assigns URLs and is rejected here because these fixtures are fully textual.
fn entry_folded_fragments(artifact: &CompiledModuleArtifact) -> Vec<String> {
    artifact
        .module
        .metadata
        .entry
        .as_ref()
        .expect("the fixture owns an entry root")
        .const_top_level_fragments
        .iter()
        .map(|fragment| {
            fragment
                .value
                .clone()
                .into_text()
                .expect("the fixture's const fragments must be fully textual")
        })
        .collect()
}

/// Returns the entry root's complete folded output as one string.
fn entry_folded_output(compiled: &ProjectFrontendCompilation) -> String {
    entry_folded_fragments(entry_artifact(compiled)).concat()
}

/// Counts infallible and fallible calls to module-owned functions across HIR statements and
/// terminators.
fn entry_hir_local_call_count(artifact: &CompiledModuleArtifact) -> usize {
    let is_local =
        |target: &CallTarget| matches!(target, CallTarget::Local(_) | CallTarget::ModulePrivate(_));
    let mut calls = 0;
    for block in &artifact.module.executable.hir.blocks {
        calls += block
            .statements
            .iter()
            .filter(|statement| match &statement.kind {
                HirStatementKind::Call { target, .. } => is_local(target),
                _ => false,
            })
            .count();
        if let HirTerminator::Invoke {
            operation: HirFallibleOperation::Call { target, .. },
            ..
        } = &block.terminator
            && is_local(target)
        {
            calls += 1;
        }
    }
    calls
}

#[path = "const_alias_signature_identity_tests.rs"]
mod const_alias_signature_identity_tests;
#[path = "entry_api_stability_tests.rs"]
mod entry_api_stability_tests;
#[path = "package_materialisation_tests.rs"]
mod package_materialisation_tests;
#[path = "provider_sources_tests.rs"]
mod provider_sources_tests;
#[path = "release_speculation_scope_tests.rs"]
mod release_speculation_scope_tests;
#[path = "scheduling_budget_tests.rs"]
mod scheduling_budget_tests;
#[path = "single_file_directory_discovery_tests.rs"]
mod single_file_directory_discovery_tests;
#[path = "source_identity_loading_tests.rs"]
mod source_identity_loading_tests;
