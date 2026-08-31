//! Shared compile-time inputs for HTML module builder paths.
//!
//! WHAT: groups the HIR/analysis data that both the JS-only and HTML+Wasm builder paths need.
//! WHY: both paths share one module-level input, keeping those facts synchronised as fields evolve.

use crate::build_system::BuildProfile;
use crate::build_system::build::ProjectEntry;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::compiler_frontend::module_compilation::{ModuleRootActivity, ResolvedConstFragment};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::page_metadata::HtmlPageMetadataPlan;
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use std::path::Path;
use std::sync::Arc;

/// Module-level inputs shared by all HTML builder compilation paths.
pub(crate) struct HtmlModuleCompileInput<'a> {
    pub hir_module: &'a HirModule,
    pub resource_table: &'a ModuleResourceTable,
    pub reachability: &'a HirReachability,
    pub type_environment: &'a TypeEnvironment,
    pub const_fragments: &'a [ResolvedConstFragment],
    pub page_metadata_plan: &'a HtmlPageMetadataPlan,
    pub borrow_analysis: &'a BorrowCheckReport,
    pub project_name: &'a str,
    pub document_config: &'a HtmlDocumentConfig,
    pub build_profile: BuildProfile,
    pub root_activity: &'a ModuleRootActivity,
    pub external_package_registry: Arc<ExternalPackageRegistry>,
}

/// Builder-owned context for compiling one selected HTML module entry.
///
/// The entry owns the module, reachability, linked modules, and generated-name maps as one
/// owner-bound unit. Keeping those values together prevents a route from combining facts from
/// different module assemblies.
pub(crate) struct HtmlModuleCompileContext<'a> {
    pub(crate) entry: ProjectEntry<'a>,
    pub(crate) page_metadata_plan: &'a HtmlPageMetadataPlan,
    pub(crate) logical_html_output_path: &'a Path,
    pub(crate) structural_url_renderer: &'a StructuralUrlRenderer<'a>,
    pub(crate) project_name: &'a str,
    pub(crate) document_config: &'a HtmlDocumentConfig,
    pub(crate) build_profile: BuildProfile,
    pub(crate) wasm_enabled: bool,
}
