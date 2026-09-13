// Shared imports and fixture helpers for focused frontend test modules.
use super::{
    FrontendCompilationMode, compile_project_frontend, compile_project_frontend_with_inputs,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BackendBuilder, Project, ProjectBuilder, ProjectCompilation, build_project,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::CleanupPolicy;
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
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::render::terse;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabelMessage, DiagnosticPayload, DiagnosticSeverity, InvalidConfigReason,
    InvalidDependencyClauseReason, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAbiType, ExternalAccessKind, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalFunctionSpec, ExternalJsLowering, ExternalReturnSlot, ExternalSignatureType,
    ExternalTypeId, ExternalTypeSpec,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::public_call_summary::PublicCallMutationEffect;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_diagnostics::assert_exact_infrastructure_error;
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
fn assert_has_diagnostic_code(messages: &CompilerMessages, expected_code: &str) {
    let actual_codes = messages
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind.code())
        .collect::<Vec<_>>();

    assert!(
        actual_codes.contains(&expected_code),
        "expected diagnostic code {expected_code}, got {actual_codes:?}"
    );
}
#[path = "package_materialisation_tests.rs"]
mod package_materialisation_tests;
#[path = "provider_sources_tests.rs"]
mod provider_sources_tests;
#[path = "single_file_directory_discovery_tests.rs"]
mod single_file_directory_discovery_tests;
#[path = "source_identity_loading_tests.rs"]
mod source_identity_loading_tests;
