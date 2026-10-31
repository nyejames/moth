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
//! a [`ResolvedTypeAnnotation`] carrying only canonical `TypeId` identity when available.

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    ActiveGenericTypeContext, GenericParameterScope,
};
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::{
    NamespaceRecord, SourceDeclarationTarget,
};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;
use std::sync::Arc;

/// The source reason that lets signature parsing retain an unresolved type shape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum TypeResolutionFailureKind {
    #[default]
    Other,
    UnknownTypeName,
    CapacityNotConstant,
}

/// Shared state for AST semantic type resolution.
///
/// WHAT: bundles every lookup table and environment that resolution needs in one place.
/// WHY: resolution touches declaration tables, visibility maps, generic scopes, type aliases,
///      namespace records, and the canonical `TypeEnvironment`; passing them individually would
///      make every helper signature noisy and error-prone.
pub(crate) struct TypeResolutionContext<'a> {
    pub declaration_table: &'a Rc<TopLevelDeclarationTable>,
    pub visible_declaration_ids: Option<&'a Arc<FxHashSet<PathId>>>,
    pub visible_external_symbols: Option<&'a FxHashMap<StringId, ExternalSymbolId>>,
    pub visible_source_bindings: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub visible_type_aliases: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    /// Completed type alias metadata. Every entry has a canonical target `TypeId`.
    ///
    /// Alias consumers use the required target identity directly. Authored syntax remains with
    /// the retained declaration, and declaration location stays attached for diagnostics.
    pub resolved_type_aliases: Option<&'a FxHashMap<PathId, ResolvedTypeAlias>>,
    pub generic_declarations_by_path: Option<&'a FxHashMap<PathId, GenericDeclarationKind>>,
    pub generic_parameters: Option<&'a GenericParameterScope>,
    pub generic_substitutions: Option<&'a FxHashMap<GenericParameterId, TypeId>>,
    /// Resolved struct fields by canonical path, including generic struct templates.
    /// Required for lazy generic struct instantiation.
    pub resolved_struct_fields_by_path: Option<&'a FxHashMap<PathId, Vec<Declaration>>>,
    /// Frontend type environment for canonical type identity.
    /// WHY: supplies the interning authority that resolves authored syntax to canonical `TypeId`s.
    pub type_environment: &'a mut TypeEnvironment,
    pub const_values: &'a ConstValueStore,
    /// Visible namespace records for resolving namespace-qualified type names.
    pub visible_namespace_records: Option<&'a FxHashMap<StringId, NamespaceRecord>>,
    pub trait_environment: Option<&'a TraitEnvironment>,
    pub trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
    pub visible_trait_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    /// Skip collecting unknown-type candidates in this context.
    ///
    /// WHAT: the signature fallback pass always discards `UnknownTypeName`.
    /// WHY: collecting candidates there is wasted work on every valid signature.
    pub skip_unknown_type_name_candidates: bool,
    /// Type name omitted from unknown-type suggestion candidates.
    ///
    /// WHAT: alias-target resolution excludes the alias being declared.
    /// WHY: `UserId as UsrId` must not suggest `UserId` from `visible_type_alias_names`.
    pub excluded_unknown_type_name: Option<StringId>,
}

/// Input bundle for constructing a [`TypeResolutionContext`].
///
/// WHAT: captures every borrowed lookup table and environment in one struct, so callers do not
///       have to remember the field order of the context constructor.
pub(crate) struct TypeResolutionContextInputs<'a> {
    pub declaration_table: &'a Rc<TopLevelDeclarationTable>,
    pub visible_declaration_ids: Option<&'a Arc<FxHashSet<PathId>>>,
    pub visible_external_symbols: Option<&'a FxHashMap<StringId, ExternalSymbolId>>,
    pub visible_source_bindings: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub visible_type_aliases: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub resolved_type_aliases: Option<&'a FxHashMap<PathId, ResolvedTypeAlias>>,
    pub generic_declarations_by_path: Option<&'a FxHashMap<PathId, GenericDeclarationKind>>,
    pub resolved_struct_fields_by_path: Option<&'a FxHashMap<PathId, Vec<Declaration>>>,
    pub type_environment: &'a mut TypeEnvironment,
    pub const_values: &'a ConstValueStore,
    /// Visible namespace records for resolving namespace-qualified type names.
    pub visible_namespace_records: Option<&'a FxHashMap<StringId, NamespaceRecord>>,
    pub trait_environment: Option<&'a TraitEnvironment>,
    pub trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
    pub visible_trait_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
}

impl<'a> TypeResolutionContext<'a> {
    /// Construct a minimal context for isolated type-resolution tests.
    ///
    /// Isolated tests own no physical file, so the context carries the compilation-root identity.
    #[cfg(test)]
    pub(crate) fn from_declaration_table(
        declaration_table: &'a Rc<TopLevelDeclarationTable>,
        type_environment: &'a mut TypeEnvironment,
        const_values: &'a ConstValueStore,
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
            const_values,
            visible_namespace_records: None,
            trait_environment: None,
            trait_evidence_environment: None,
            visible_trait_names: None,
            skip_unknown_type_name_candidates: false,
            excluded_unknown_type_name: None,
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
            const_values: inputs.const_values,
            visible_namespace_records: inputs.visible_namespace_records,
            trait_environment: inputs.trait_environment,
            trait_evidence_environment: inputs.trait_evidence_environment,
            visible_trait_names: inputs.visible_trait_names,
            skip_unknown_type_name_candidates: false,
            excluded_unknown_type_name: None,
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

    pub(crate) fn skip_unknown_type_name_candidates(mut self) -> Self {
        self.skip_unknown_type_name_candidates = true;
        self
    }

    pub(crate) fn excluding_unknown_type_name(mut self, name: Option<StringId>) -> Self {
        self.excluded_unknown_type_name = name;
        self
    }
}

/// A completed top-level type alias after semantic resolution.
///
/// WHAT: carries the canonical target identity and declaration span for one alias.
/// WHY: a completed alias cannot represent an unresolved target. Keeping the required `TypeId`
/// separate from the general annotation result prevents later consumers from mistaking an
/// unresolved annotation placeholder for the builtin `None` type.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedTypeAlias {
    pub(crate) target_type_id: TypeId,
    pub(crate) declaration_span: Option<SourceSpan>,
}

/// A parsed type annotation after semantic resolution.
///
/// WHAT: carries the canonical `TypeId` when the source declared a type.
/// WHY: semantic identity is owned directly by `TypeId`; authored syntax remains with its syntax
/// owner for diagnostics and source-oriented consumers.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedTypeAnnotation {
    pub(crate) type_id: Option<TypeId>,
}
