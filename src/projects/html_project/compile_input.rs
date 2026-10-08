//! Shared compile-time inputs for HTML module builder paths.
//!
//! WHAT: groups module data shared by the JS-only and HTML+Wasm builder paths.
//! WHY: common facts stay aligned while backend-specific facts are carried only to their consumer.

use crate::build_system::BuildProfile;
use crate::build_system::build::ProjectEntry;
use crate::compiler_frontend::analysis::borrow_checker::BorrowFacts;
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::compiler_frontend::module_compilation::{ModuleRootActivity, ResolvedConstFragment};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::output_plan::CanonicalPageRoute;
use crate::projects::html_project::page_metadata::HtmlPageMetadataPlan;
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use moth_lexical::numeric::profile::NumericProfile;
use std::sync::Arc;

/// Module-level inputs shared by all HTML builder compilation paths.
pub(crate) struct HtmlModuleCompileInput<'a> {
    pub hir_module: &'a HirModule,
    pub path_table: &'a PathTable,
    pub resource_table: &'a ModuleResourceTable,
    pub reachability: &'a HirReachability,
    pub type_environment: &'a TypeEnvironment,
    pub const_fragments: &'a [ResolvedConstFragment],
    pub page_metadata_plan: &'a HtmlPageMetadataPlan,
    pub borrow_facts: &'a BorrowFacts,
    /// Conservative bounded-integer proof facts paired with `hir_module`'s executable.
    ///
    /// WHY: the table is computed per executable inside the compiler service and published with
    ///      it, so both lowering paths query facts that match the exact HIR and profile here.
    pub numeric_proofs: &'a NumericProofs,
    pub project_name: &'a str,
    pub document_config: &'a HtmlDocumentConfig,
    pub build_profile: BuildProfile,
    pub root_activity: &'a ModuleRootActivity,
    pub external_package_registry: Arc<ExternalPackageRegistry>,
    /// Compiler-owned numeric widths for this compilation boundary, settled at bootstrap and
    /// carried on the project compilation.
    pub numeric_profile: NumericProfile,
}

/// Builder-owned context for compiling one selected HTML module entry.
///
/// The entry owns the module, reachability, linked modules, and generated-name maps as one
/// owner-bound unit. Keeping those values together prevents a route from combining facts from
/// different module assemblies.
pub(crate) struct HtmlModuleCompileContext<'a> {
    pub(crate) entry: ProjectEntry<'a>,
    pub(crate) page_metadata_plan: &'a HtmlPageMetadataPlan,
    /// Canonical route projections shared by JS output, Wasm placement and the HTML shell.
    pub(crate) route: &'a CanonicalPageRoute,
    pub(crate) structural_url_renderer: &'a StructuralUrlRenderer<'a>,
    pub(crate) project_name: &'a str,
    pub(crate) document_config: &'a HtmlDocumentConfig,
    pub(crate) build_profile: BuildProfile,
    pub(crate) wasm_enabled: bool,
    /// The boundary numeric profile settled at bootstrap, carried on the compilation.
    ///
    /// WHY: the backend must lower the settled value instead of re-asking the builder.
    pub(crate) numeric_profile: NumericProfile,
}
