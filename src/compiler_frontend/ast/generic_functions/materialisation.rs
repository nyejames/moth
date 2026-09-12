//! Immutable declaring-module context for generated generic functions.
//!
//! WHAT: retains the resolved, TIR-free semantic tables required to reparse one validated
//! generic body after a consumer emits a concrete request.
//! WHY: generated sidecars must compile in an independent type environment without reopening
//! source or borrowing the mutable requester or declaring-module environment.
//!
//! The lane map is explicit: stable capture contracts are in `stable_types`,
//! frozen artefact reconstruction in `artefact_emit`, visibility/namespace capture in
//! `visibility`, declaring-module freeze in `preparation_freeze`, and sidecar build/evidence in
//! `sidecar_build`. The pre-existing children own syntax, file-reference, nominal-blueprint,
//! semantic-closure and alias projection details.
mod alias_projection;
mod artefact_emit;
mod frozen_file_references;
mod frozen_syntax;
mod nominal_blueprints;
mod preparation_freeze;
mod semantic_closure;
mod sidecar_build;
mod stable_types;
mod visibility;

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::AstBuildResult;
#[cfg(test)]
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_public_folded_value;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(crate) use artefact_emit::ModuleMaterialisationContext;
pub(crate) use preparation_freeze::{
    ModuleMaterialisationEnvironmentInput, ModuleMaterialisationPreparation,
    ModuleMaterialisationPreparationBuilder,
};
pub(crate) use sidecar_build::bootstrap_call_summary_from_signature;

pub(crate) struct MaterialisedGenericAst {
    pub(crate) build_result: AstBuildResult,
    pub(crate) string_table: StringTable,
    pub(crate) instance_path: PathId,
}

/// Active build services and requester facts for one published generic materialisation.
///
/// The published context owns only stable declaration artefacts. Build-lifetime registries,
/// source-path folding policy and the requester-local evidence environment enter through this
/// transient input and are never retained by successful module metadata.
pub(crate) struct ModuleMaterialisationInput<'a> {
    pub(crate) identity: &'a GeneratedFunctionIdentity,
    pub(crate) requester_context: &'a ModuleMaterialisationPreparation,
    pub(crate) requester_call_span: Option<SourceSpan>,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) path_fork: &'a mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) template_const_loop_iteration_limit: usize,
    /// The requesting module owns generated work in current-schema attribution.
    #[cfg(feature = "timers")]
    pub(crate) timing_context: Option<crate::timing::TimingContext>,
}

#[cfg(test)]
#[path = "tests/frozen_body_tests.rs"]
mod frozen_body_tests;

#[cfg(test)]
#[path = "tests/semantic_closure_tests.rs"]
mod semantic_closure_tests;
