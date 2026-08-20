//! Shared inputs for compiling one ready module.
//!
//! WHAT: the provider interfaces, capability surface and compiler options every step of one module
//!       compilation reads.
//! WHY: Stage 0 decides when a module is ready and what it may see. It builds this value once, then
//!      the compiler sequences its own semantic stages against it. Every field is immutable for the
//!      duration of the call, so semantic analysis can never write back into build-system state.

use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::module_compilation::generated::ProviderMaterialisationRegistry;
use crate::compiler_frontend::module_compilation::options::FrontendOptions;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;

use std::sync::Arc;

/// Everything one module compilation needs beyond its own prepared source.
pub(crate) struct ModuleCompilationContext<'a> {
    /// Compiler-owned settings, already projected from whatever configuration the caller uses.
    pub(crate) options: FrontendOptions,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) external_packages: Arc<ExternalPackageRegistry>,
    pub(crate) external_dependency_resolution_table: &'a ExternalImportResolutionTable,
    /// Completed provider interfaces this module is allowed to bind.
    pub(crate) source_provider_dependencies: &'a SourceProviderDependencySet<'a>,
    /// Declaring-module generic templates already published in this compilation boundary.
    pub(crate) provider_materialisations: &'a ProviderMaterialisationRegistry,
    pub(crate) builder_runtime_packages: &'a [BuilderRuntimePackageMetadata],
}
