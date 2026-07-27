//! Shared compile-time inputs for HTML module builder paths.
//!
//! WHAT: groups the HIR/analysis data that both the JS-only and HTML+Wasm builder paths need.
//! WHY: both paths take the same 7 module-level parameters — bundling them avoids a long
//!      argument list at every call site and keeps the two paths in sync as fields evolve.

use crate::build_system::build::{ModuleRootActivity, ResolvedConstFragment};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use std::sync::Arc;

/// Module-level inputs shared by all HTML builder compilation paths.
pub(crate) struct HtmlModuleCompileInput<'a> {
    pub hir_module: &'a HirModule,
    pub reachability: &'a HirReachability,
    pub type_environment: &'a TypeEnvironment,
    pub const_fragments: &'a [ResolvedConstFragment],
    pub borrow_analysis: &'a BorrowCheckReport,
    pub project_name: &'a str,
    pub document_config: &'a HtmlDocumentConfig,
    pub release_build: bool,
    pub root_activity: &'a ModuleRootActivity,
    pub external_package_registry: Arc<ExternalPackageRegistry>,
}
