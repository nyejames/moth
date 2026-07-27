//! Type-resolution context, input, and result types.
//!
//! WHAT: holds the shared state that `resolve_type` and its helpers need to turn parsed type
//!       syntax into canonical `TypeId`-based semantic identity.
//! WHY: keeping the context/input/result structs in their own file lets `resolve_type.rs` stay
//!      focused on resolution orchestration and semantic helpers, while this file owns the
//!      shape of the resolution environment itself.
//!
//! Callers build a [`TypeResolutionContext`] from [`TypeResolutionContextInputs`] and then pass it
//! through the resolution functions in [`super::resolve_type`]. The resolved result is returned as
//! a [`ResolvedTypeAnnotation`] that carries both the semantic `TypeId` and the diagnostic
//! spelling, so later stages do not need to re-derive either from the other.

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    ActiveGenericTypeContext, GenericParameterScope,
};
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::import_environment::{
    NamespaceRecord, SourceDeclarationTarget,
};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationMetadata;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

/// Shared state for AST semantic type resolution.
///
/// WHAT: bundles every lookup table and environment that resolution needs in one place.
/// WHY: resolution touches declaration tables, visibility maps, generic scopes, type aliases,
///      namespace records, and the canonical `TypeEnvironment`; passing them individually would
///      make every helper signature noisy and error-prone.
pub(crate) struct TypeResolutionContext<'a> {
    pub declaration_table: &'a Rc<TopLevelDeclarationTable>,
    pub visible_declaration_ids: Option<&'a FxHashSet<InternedPath>>,
    pub visible_external_symbols: Option<&'a FxHashMap<StringId, ExternalSymbolId>>,
    pub visible_source_bindings: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub visible_type_aliases: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    /// Resolved type alias metadata: parsed source ref, diagnostic spelling, and canonical
    /// `TypeId` when available. This is the single owner of alias metadata; callers should use
    /// `annotation.type_id` for semantic checks and `annotation.diagnostic_type` for diagnostics
    /// or for re-resolving the original parsed annotation when needed.
    pub resolved_type_aliases: Option<&'a FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
    pub generic_declarations_by_path:
        Option<&'a FxHashMap<InternedPath, GenericDeclarationMetadata>>,
    pub generic_parameters: Option<&'a GenericParameterScope>,
    pub generic_substitutions: Option<&'a FxHashMap<GenericParameterId, TypeId>>,
    /// Resolved struct fields by canonical path, including generic struct templates.
    /// Required for lazy generic struct instantiation.
    pub resolved_struct_fields_by_path: Option<&'a FxHashMap<InternedPath, Vec<Declaration>>>,
    /// Frontend type environment for canonical type identity.
    /// WHY: enables resolution directly to TypeId instead of going through DataType.
    ///      All production type resolution must have access to the canonical environment.
    pub type_environment: &'a mut TypeEnvironment,
    /// Visible namespace records for resolving namespace-qualified type names.
    pub visible_namespace_records: Option<&'a FxHashMap<StringId, NamespaceRecord>>,
    pub trait_environment: Option<&'a TraitEnvironment>,
    pub trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
    pub visible_trait_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
}

/// Input bundle for constructing a [`TypeResolutionContext`].
///
/// WHAT: captures every borrowed lookup table and environment in one struct, so callers do not
///       have to remember the field order of the context constructor.
pub(crate) struct TypeResolutionContextInputs<'a> {
    pub declaration_table: &'a Rc<TopLevelDeclarationTable>,
    pub visible_declaration_ids: Option<&'a FxHashSet<InternedPath>>,
    pub visible_external_symbols: Option<&'a FxHashMap<StringId, ExternalSymbolId>>,
    pub visible_source_bindings: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub visible_type_aliases: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub resolved_type_aliases: Option<&'a FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
    pub generic_declarations_by_path:
        Option<&'a FxHashMap<InternedPath, GenericDeclarationMetadata>>,
    pub resolved_struct_fields_by_path: Option<&'a FxHashMap<InternedPath, Vec<Declaration>>>,
    pub type_environment: &'a mut TypeEnvironment,
    /// Visible namespace records for resolving namespace-qualified type names.
    pub visible_namespace_records: Option<&'a FxHashMap<StringId, NamespaceRecord>>,
    pub trait_environment: Option<&'a TraitEnvironment>,
    pub trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
    pub visible_trait_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
}

impl<'a> TypeResolutionContext<'a> {
    #[cfg(test)]
    pub(crate) fn from_declaration_table(
        declaration_table: &'a Rc<TopLevelDeclarationTable>,
        type_environment: &'a mut TypeEnvironment,
    ) -> Self {
        Self {
            declaration_table,
            visible_declaration_ids: None,
            visible_external_symbols: None,
            visible_source_bindings: None,
            visible_type_aliases: None,
            resolved_type_aliases: None,
            generic_declarations_by_path: None,
            generic_parameters: None,
            generic_substitutions: None,
            resolved_struct_fields_by_path: None,
            type_environment,
            visible_namespace_records: None,
            trait_environment: None,
            trait_evidence_environment: None,
            visible_trait_names: None,
        }
    }

    pub(crate) fn from_inputs(inputs: TypeResolutionContextInputs<'a>) -> Self {
        Self {
            declaration_table: inputs.declaration_table,
            visible_declaration_ids: inputs.visible_declaration_ids,
            visible_external_symbols: inputs.visible_external_symbols,
            visible_source_bindings: inputs.visible_source_bindings,
            visible_type_aliases: inputs.visible_type_aliases,
            resolved_type_aliases: inputs.resolved_type_aliases,
            generic_declarations_by_path: inputs.generic_declarations_by_path,
            generic_parameters: None,
            generic_substitutions: None,
            resolved_struct_fields_by_path: inputs.resolved_struct_fields_by_path,
            type_environment: inputs.type_environment,
            visible_namespace_records: inputs.visible_namespace_records,
            trait_environment: inputs.trait_environment,
            trait_evidence_environment: inputs.trait_evidence_environment,
            visible_trait_names: inputs.visible_trait_names,
        }
    }

    pub(crate) fn with_generic_parameters(
        mut self,
        generic_parameters: Option<&'a GenericParameterScope>,
    ) -> Self {
        self.generic_parameters = generic_parameters;
        self
    }

    pub(crate) fn with_active_generic_type_context(
        mut self,
        generic_context: Option<&'a ActiveGenericTypeContext>,
    ) -> Self {
        if let Some(generic_context) = generic_context {
            self.generic_parameters = Some(&generic_context.parameter_scope);
            self.generic_substitutions = generic_context.substitutions.as_ref();
        }

        self
    }
}

/// A parsed type annotation after semantic resolution.
///
/// WHAT: carries the original parsed spelling, the resolved diagnostic spelling,
/// and the canonical `TypeId` when the source actually declared a type.
/// WHY: new AST paths should not re-derive semantic identity from `DataType`
/// after resolution. Keeping both values together makes `DataType` a diagnostic
/// companion instead of the semantic source of truth.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedTypeAnnotation {
    /// Kept with the resolved annotation so follow-up refactors can preserve source
    /// spelling through diagnostics without re-parsing or reverse-converting `DataType`.
    pub(crate) source_ref: ParsedTypeRef,
    /// Diagnostic spelling stays attached to the `TypeId` for user-facing type text.
    pub(crate) diagnostic_type: DataType,
    pub(crate) type_id: Option<TypeId>,
}
