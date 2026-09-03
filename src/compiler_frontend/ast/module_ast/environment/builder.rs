//! AST module environment builder.
//!
//! WHAT: consumes header-built dependency visibility and resolves declarations, constants, nominal
//! types, function signatures, and receiver catalog data into a stable semantic environment.
//! WHY: after this phase completes, AST emission can parse bodies against a stable environment
//! instead of depending on pass-order-specific accumulator fields.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironment, AstModuleLookups, BuildResolvedPublicTypeRootsInput,
    DeclarationId, DeclarationSemanticTable, ResolvedConstantSet, ResolvedPublicTraitRoot,
    ResolvedPublicTypeRootTable, TopLevelDeclarationTable, build_resolved_public_trait_roots,
    build_resolved_public_type_roots,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodCatalog;
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext, TemplateWrapperReference, TirSlotPlaceholder, summarize_existing_root,
};
use crate::compiler_frontend::ast::type_resolution::{
    GenericParameterScopeBuildInput, ResolvedFunctionSignature, ResolvedTypeAlias,
    TypeResolutionContext, TypeResolutionContextInputs, build_generic_parameter_scope,
    resolve_diagnostic_type_to_type_id_checked,
};
use crate::compiler_frontend::ast::{
    AstChoiceDefinition, AstImportedFunctionContract, AstImportedStructDefinition,
};
use crate::compiler_frontend::builtins::error_type::builtin_error_type_path;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::{
    RegisteredGenericParameterList, TypeEnvironment,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, GenericParameterScope, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{
    FunctionTypeKey, GenericParameterId, NominalTypeId, TypeId, builtin_type_ids,
};
use crate::compiler_frontend::datatypes::{DataType, diagnostic_type_spelling};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::folded_value::{
    PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicFoldedField, PublicFoldedValue, PublicTemplateSlotKey,
};
use crate::compiler_frontend::headers::binding_environment::{
    FileVisibility, HeaderBindingEnvironment,
};
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationMetadata, ModuleSymbols, OrderedSemanticDeclaration,
    OrderedSemanticDeclarationKind,
};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use crate::compiler_frontend::public_interface::{
    PublicChoiceSemantics, PublicConstantSemantics, PublicDeclarationSemantics,
    PublicFunctionCategory, PublicGenericParameterSurface, PublicParameterTypeSlot,
    PublicReceiverMethodCategory, PublicReturnTypeSlot, PublicStructSemantics,
};
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginTypeId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::{
    TraitEvidenceEnvironment, ValidateTraitEvidenceInput, validate_trait_evidence,
};
use crate::compiler_frontend::traits::ids::TraitId;
use crate::compiler_frontend::traits::syntax::TraitReferenceSyntax;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::timing_scope_attributed;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;
use std::sync::Arc;

pub(in crate::compiler_frontend::ast) mod import_projection;

#[cfg(test)]
pub(crate) use import_projection::imported_nominal_path;

/// Collect the set of source declaration paths re-exported through the root's `export:` block
/// from private files in the same module.
///
/// WHAT: iterates the header-built public export maps and collects every
/// `PublicExportTarget::SourceDeclaration`
/// path whose canonical source file belongs to the active module. These are declarations from
/// private files re-exported through the root's public surface.
/// WHY: the resolved public type-root table needs to include re-exported declarations so the
/// public-interface draft builder can project their semantic facts into the published interface.
fn collect_reexport_target_paths(
    active_root_source: &InternedPath,
    module_symbols: &ModuleSymbols,
) -> FxHashSet<InternedPath> {
    let mut paths = FxHashSet::default();

    let Some(module_root) = module_symbols
        .file_module_membership
        .get(active_root_source)
    else {
        return paths;
    };
    for entries in module_symbols.module_root_public_exports.values() {
        for entry in entries {
            let Some(path) = entry.target.source_path() else {
                continue;
            };
            let Some(target_source) = module_symbols.canonical_source_by_symbol_path.get(path)
            else {
                continue;
            };

            if module_symbols.file_module_membership.get(target_source) == Some(module_root) {
                paths.insert(path.clone());
            }
        }
    }

    paths
}

/// Combined transient resolved public-surface outputs built during environment construction.
///
/// WHAT: bundles the type-only root table and the direct trait-root vector so
/// `finish_environment` does not take them as separate positional arguments.
struct ResolvedPublicSurfaceOutputs {
    type_roots: ResolvedPublicTypeRootTable,
    trait_roots: Vec<ResolvedPublicTraitRoot>,
}

/// Dense Stage 3 declaration lanes consumed by ordered AST environment passes.
///
/// The lanes contain only stable IDs. Header indexes are one lookup table keyed by those IDs, so
/// pass order stays explicit without retaining a second set of header vectors.
pub(crate) struct DeclarationPassLanes {
    header_index_by_id: Vec<Option<usize>>,
    pub(crate) ordered: Vec<DeclarationId>,
    pub(crate) aliases: Vec<DeclarationId>,
    pub(crate) nominals: Vec<DeclarationId>,
    pub(crate) structs: Vec<DeclarationId>,
    pub(crate) choices: Vec<DeclarationId>,
    pub(crate) constants: Vec<DeclarationId>,
    pub(crate) traits: Vec<DeclarationId>,
    pub(crate) functions: Vec<DeclarationId>,
}

impl DeclarationPassLanes {
    pub(crate) fn from_stage3_order(
        sorted_headers: &[Header],
        ordered_declarations: &[OrderedSemanticDeclaration],
    ) -> Result<Self, CompilerError> {
        let mut lanes = Self {
            header_index_by_id: vec![None; ordered_declarations.len()],
            ordered: Vec::new(),
            aliases: Vec::new(),
            nominals: Vec::new(),
            structs: Vec::new(),
            choices: Vec::new(),
            constants: Vec::new(),
            traits: Vec::new(),
            functions: Vec::new(),
        };
        let mut covered_headers = vec![false; sorted_headers.len()];

        for (expected_index, ordered) in ordered_declarations.iter().enumerate() {
            if ordered.declaration_id.index() != expected_index {
                return Err(missing_declaration_id());
            }
            let header = sorted_headers
                .get(ordered.header_index)
                .ok_or_else(missing_declaration_id)?;
            if header.tokens.src_path != ordered.path
                || !header_matches_ordered_kind(&header.kind, ordered.kind)
                || covered_headers[ordered.header_index]
            {
                return Err(missing_declaration_id());
            }
            covered_headers[ordered.header_index] = true;
            let declaration_id = ordered.declaration_id;
            let lane = match ordered.kind {
                OrderedSemanticDeclarationKind::TypeAlias => &mut lanes.aliases,
                OrderedSemanticDeclarationKind::Struct => {
                    lanes.nominals.push(declaration_id);
                    lanes.structs.push(declaration_id);
                    lanes.ordered.push(declaration_id);
                    lanes.header_index_by_id[declaration_id.index()] = Some(ordered.header_index);
                    continue;
                }
                OrderedSemanticDeclarationKind::Choice => {
                    lanes.nominals.push(declaration_id);
                    lanes.choices.push(declaration_id);
                    lanes.ordered.push(declaration_id);
                    lanes.header_index_by_id[declaration_id.index()] = Some(ordered.header_index);
                    continue;
                }
                OrderedSemanticDeclarationKind::Constant => &mut lanes.constants,
                OrderedSemanticDeclarationKind::Trait => &mut lanes.traits,
                OrderedSemanticDeclarationKind::Function => &mut lanes.functions,
            };

            lane.push(declaration_id);
            lanes.ordered.push(declaration_id);
            lanes.header_index_by_id[declaration_id.index()] = Some(ordered.header_index);
        }

        for (header_index, header) in sorted_headers.iter().enumerate() {
            if header_is_semantic(&header.kind) && !covered_headers[header_index] {
                return Err(missing_declaration_id());
            }
        }

        Ok(lanes)
    }

    pub(crate) fn header<'a>(
        &self,
        declaration_id: DeclarationId,
        sorted_headers: &'a [Header],
    ) -> Result<&'a Header, CompilerError> {
        let header_index = self
            .header_index_by_id
            .get(declaration_id.index())
            .copied()
            .flatten()
            .ok_or_else(missing_declaration_id)?;
        sorted_headers
            .get(header_index)
            .ok_or_else(missing_declaration_id)
    }
}

fn header_matches_ordered_kind(
    header_kind: &HeaderKind,
    ordered_kind: OrderedSemanticDeclarationKind,
) -> bool {
    matches!(
        (header_kind, ordered_kind),
        (
            HeaderKind::TypeAlias { .. },
            OrderedSemanticDeclarationKind::TypeAlias
        ) | (
            HeaderKind::Struct { .. },
            OrderedSemanticDeclarationKind::Struct
        ) | (
            HeaderKind::Choice { .. },
            OrderedSemanticDeclarationKind::Choice
        ) | (
            HeaderKind::Constant { .. },
            OrderedSemanticDeclarationKind::Constant
        ) | (
            HeaderKind::Trait { .. },
            OrderedSemanticDeclarationKind::Trait
        ) | (
            HeaderKind::Function { .. },
            OrderedSemanticDeclarationKind::Function
        )
    )
}

fn header_is_semantic(header_kind: &HeaderKind) -> bool {
    matches!(
        header_kind,
        HeaderKind::TypeAlias { .. }
            | HeaderKind::Struct { .. }
            | HeaderKind::Choice { .. }
            | HeaderKind::Constant { .. }
            | HeaderKind::Trait { .. }
            | HeaderKind::Function { .. }
    )
}

fn missing_declaration_id() -> CompilerError {
    CompilerError::compiler_error(
        "Stage 3 semantic declaration order did not match the AST declaration table.",
    )
}

/// One imported generic list registered before provider trait identities receive local handles.
#[derive(Clone)]
struct ImportedGenericParameterRegistration {
    surfaces: Vec<PublicGenericParameterSurface>,
    list_id: crate::compiler_frontend::datatypes::ids::GenericParameterListId,
    canonical_by_local: FxHashMap<TypeParameterId, GenericParameterId>,
}

pub(crate) struct AstModuleEnvironmentBuilder<'context, 'services> {
    pub(crate) context: &'context AstPhaseContext<'services>,

    // Header-owned module symbol package from the header/dependency-sort phase.
    pub(crate) module_symbols: ModuleSymbols,

    // Header-built dependency visibility is consumed directly; AST does not rebuild dependency bindings.
    pub(crate) binding_environment: HeaderBindingEnvironment,

    // Mutable environment-building state.
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    pub(crate) declaration_table: Rc<TopLevelDeclarationTable>,
    /// Paths of every module constant resolved so far, shared with environment-time scopes.
    ///
    /// WHY: constant-header, nominal-member and function-signature parsing all need to know
    /// which visible declarations are explicit compile-time constants. Sharing one set means
    /// none of those passes copies it per declaration.
    pub(crate) resolved_module_constants: Rc<ResolvedConstantSet>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,

    // Copy-on-write side tables shared with every environment-time `ScopeContext`.
    //
    // WHY: nominal member and constant parsing build one `ScopeContext` per header, and each one
    // needs to read these tables. Owning them behind `Rc` makes taking a handle free, so the cost
    // of the member passes follows the number of headers instead of headers times module size.
    // Writers go through `Rc::make_mut`: the scope that borrowed a handle is always dropped before
    // the next write, so the builder is the sole owner at write time and no copy is made.
    pub(crate) resolved_struct_fields_by_path: Rc<FxHashMap<InternedPath, Vec<Declaration>>>,
    pub(crate) choice_variant_shells_by_path: Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>,
    pub(crate) resolved_type_aliases_by_path: Rc<FxHashMap<InternedPath, ResolvedTypeAlias>>,
    /// Generic declaration metadata, moved out of `module_symbols` when the builder starts.
    ///
    /// WHY: it is read by every environment pass and written by none of them, so the builder owns
    /// the single shared handle rather than copying the map out of `module_symbols` per header.
    pub(crate) generic_declarations_by_path:
        Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>,

    pub(crate) struct_source_by_path: FxHashMap<InternedPath, InternedPath>,
    pub(crate) choice_source_by_path: FxHashMap<InternedPath, InternedPath>,
    pub(crate) resolved_function_signatures_by_path:
        FxHashMap<InternedPath, ResolvedFunctionSignature>,
    pub(crate) generic_function_templates_by_path: FxHashMap<InternedPath, GenericFunctionTemplate>,
    pub(crate) generic_parameter_lists_by_path:
        FxHashMap<InternedPath, RegisteredGenericParameterList>,

    // Frontend semantic type identity built during environment construction.
    // WHY: parsed types are resolved into canonical TypeIds as declarations are processed.
    pub(crate) type_environment: TypeEnvironment,

    // Canonical TypeId for each nominal struct/choice registered in type_environment.
    // Copy-on-write for the same reason as the side tables above.
    pub(crate) nominal_type_ids_by_path: Rc<FxHashMap<InternedPath, TypeId>>,
    imported_type_ids_by_origin: FxHashMap<OriginTypeId, TypeId>,
    imported_generic_parameter_type_ids: FxHashMap<
        crate::compiler_frontend::canonical_type_identity::ExportedGenericParameterIdentity,
        TypeId,
    >,
    imported_generic_parameter_registrations: Vec<ImportedGenericParameterRegistration>,
    pub(super) projected_imported_functions_by_local_path:
        FxHashMap<InternedPath, AstImportedFunctionContract>,
    /// Every imported receiver-method path, including generic methods without a concrete
    /// summary. The category-neutral table feeds the receiver catalog; the origin index below
    /// gives imported evidence one deterministic path without scanning concrete contracts.
    pub(super) projected_imported_receiver_methods_by_local_path:
        FxHashMap<InternedPath, crate::compiler_frontend::semantic_identity::OriginFunctionId>,
    pub(super) imported_receiver_method_paths_by_origin:
        FxHashMap<crate::compiler_frontend::semantic_identity::OriginFunctionId, InternedPath>,
    imported_struct_definitions: Vec<AstImportedStructDefinition>,
    imported_choice_definitions: Vec<AstChoiceDefinition>,
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(crate) fn new(context: &'context AstPhaseContext<'services>) -> Self {
        Self {
            context,
            module_symbols: ModuleSymbols::empty(),
            binding_environment: HeaderBindingEnvironment::default(),
            warnings: Vec::new(),
            declaration_table: Rc::new(TopLevelDeclarationTable::empty()),
            resolved_module_constants: Rc::new(ResolvedConstantSet::default()),
            builtin_struct_ast_nodes: Vec::new(),
            resolved_struct_fields_by_path: Rc::new(FxHashMap::default()),
            choice_variant_shells_by_path: Rc::new(FxHashMap::default()),
            resolved_type_aliases_by_path: Rc::new(FxHashMap::default()),
            generic_declarations_by_path: Rc::new(FxHashMap::default()),
            struct_source_by_path: FxHashMap::default(),
            choice_source_by_path: FxHashMap::default(),
            resolved_function_signatures_by_path: FxHashMap::default(),
            generic_function_templates_by_path: FxHashMap::default(),
            generic_parameter_lists_by_path: FxHashMap::default(),
            type_environment: TypeEnvironment::new(),
            nominal_type_ids_by_path: Rc::new(FxHashMap::default()),
            imported_type_ids_by_origin: FxHashMap::default(),
            imported_generic_parameter_type_ids: FxHashMap::default(),
            imported_generic_parameter_registrations: Vec::new(),
            projected_imported_functions_by_local_path: FxHashMap::default(),
            projected_imported_receiver_methods_by_local_path: FxHashMap::default(),
            imported_receiver_method_paths_by_origin: FxHashMap::default(),
            imported_struct_definitions: Vec::new(),
            imported_choice_definitions: Vec::new(),
        }
    }

    pub(crate) fn build(
        mut self,
        sorted_headers: &[Header],
        input: AstEnvironmentInput,
        string_table: &mut StringTable,
    ) -> Result<AstModuleEnvironment, CompilerMessages> {
        let AstEnvironmentInput {
            mut module_symbols,
            binding_environment,
        } = input;

        // Move header-owned data into the builder state.
        let ordered_semantic_declarations =
            std::mem::take(&mut module_symbols.ordered_semantic_declarations);
        let compiler_owned_declarations =
            std::mem::take(&mut module_symbols.compiler_owned_declarations);
        let builtin_struct_ast_nodes = std::mem::take(&mut module_symbols.builtin_struct_ast_nodes);
        let resolved_struct_fields_by_path =
            std::mem::take(&mut module_symbols.resolved_struct_fields_by_path);
        let struct_source_by_path = std::mem::take(&mut module_symbols.struct_source_by_path);
        // Generic declaration metadata has one owner from here on. Import projection adds imported
        // generic nominals to it and every environment pass reads it, so taking it now keeps one
        // map behind one handle: the per-header scopes borrow it instead of copying it, and no
        // writer is left holding a different map from the readers.
        let generic_declarations_by_path =
            std::mem::take(&mut module_symbols.generic_declarations_by_path);

        self.module_symbols = module_symbols;
        self.binding_environment = binding_environment;
        self.warnings = self.binding_environment.warnings.clone();
        let declaration_lanes =
            DeclarationPassLanes::from_stage3_order(sorted_headers, &ordered_semantic_declarations)
                .map_err(|error| self.error_messages(error, string_table))?;
        let declaration_table = TopLevelDeclarationTable::from_stage3_order(
            ordered_semantic_declarations,
            compiler_owned_declarations,
        )
        .map_err(|error| self.error_messages(error, string_table))?;
        self.declaration_table = Rc::new(declaration_table);
        self.builtin_struct_ast_nodes = builtin_struct_ast_nodes;
        self.resolved_struct_fields_by_path = Rc::new(resolved_struct_fields_by_path);
        self.generic_declarations_by_path = Rc::new(generic_declarations_by_path);
        self.struct_source_by_path = struct_source_by_path;

        timing_scope_attributed!(
            timing_guard,
            self.context.timing_metric_family.environment(),
            self.context.timing_context
        );

        // ------------------------------------
        //  Register builtin semantic types
        // ------------------------------------
        self.register_builtin_structs_in_type_environment(string_table)?;
        self.project_imported_nominal_declarations(string_table)
            .map_err(|error| self.error_messages(error, string_table))?;
        self.project_imported_alias_declarations()
            .map_err(|error| self.error_messages(error, string_table))?;
        self.project_imported_constant_declarations(string_table)
            .map_err(|error| self.error_messages(error, string_table))?;
        self.project_imported_function_declarations(string_table)?;
        self.project_imported_receiver_method_declarations(string_table)?;

        // -------------------------------------
        //  Register local nominal identities
        // -------------------------------------
        // WHAT: give every local struct and choice a canonical TypeId before aliases resolve.
        // WHY: local aliases may target those nominals (`TaskList as {Task}`), while nominal
        // members may still use aliases (`id TaskId`). Identity must precede aliases; member
        // shells must follow them.
        //
        // ```text
        // nominal identity -> local aliases may target the nominal
        // resolved aliases -> nominal members may use aliases
        // ```
        self.register_nominal_identities(&declaration_lanes, sorted_headers, string_table)?;

        // ----------------------
        //  Resolve type aliases
        // ----------------------
        // Aliases whose targets fold a `#capacity` constant (and aliases naming those) wait for
        // the constant pass, which publishes them at their own Stage 3 position. No consumer ever
        // observes a provisional alias target.
        let aliases_waiting_for_constants =
            self.aliases_waiting_for_constants(&declaration_lanes, sorted_headers, string_table)?;
        self.resolve_type_aliases(
            &declaration_lanes,
            sorted_headers,
            &aliases_waiting_for_constants,
            string_table,
        )?;

        // -----------------------------------
        //  Prepare nominal member shells
        // -----------------------------------
        self.prepare_nominal_member_shells(&declaration_lanes, sorted_headers, string_table)?;

        // ----------------------------------------------
        //  Publish constant-dependent alias targets
        // ----------------------------------------------
        // Trait requirements may name an alias whose target folds a `#capacity` constant, so the
        // Stage 3 walk runs as far as the last waiting alias before user traits are resolved. The
        // main constant pass below continues the same sequence with full trait metadata. Core
        // traits are registered first so member shells reached by this prefix keep rejecting
        // trait names in ordinary type positions.
        let core_traits = self.register_core_traits(string_table)?;
        self.resolve_constant_dependent_aliases(
            &declaration_lanes,
            sorted_headers,
            &core_traits,
            &aliases_waiting_for_constants,
            string_table,
        )?;

        // Every alias-lane declaration now has a published target, either from the alias pass or
        // from the bounded prefix walk above. Establish that here, before trait resolution reads
        // the first alias, so every consumer reads a local alias row as a fact.
        self.validate_resolved_alias_completeness(
            &declaration_lanes,
            sorted_headers,
            string_table,
        )?;

        // --------------------------
        //  Resolve trait metadata
        // --------------------------
        // Trait definitions are needed before function signatures so declaration-site
        // generic bounds can be resolved into canonical TraitIds. They are also needed
        // before struct fields, choice payloads, and constants so trait names in ordinary
        // type positions can be rejected with the trait-specific diagnostic.
        // Evidence validation stays after receiver catalog construction because it needs
        // resolved receiver methods.
        let trait_environment = self.resolve_trait_definitions(
            &declaration_lanes,
            sorted_headers,
            core_traits,
            string_table,
        )?;
        self.resolve_dependencyed_generic_parameter_bounds(&trait_environment)
            .map_err(|error| self.error_messages(error, string_table))?;

        // -------------------------------------------
        //  Resolve nominal members and constants
        // -------------------------------------------
        // WHAT: resolves constructor shells, constants, struct fields, and choice
        // payload types with trait-aware type resolution.
        // WHY: static trait metadata must be available so ordinary type annotations can
        // reject trait names without falling through to an unknown-type diagnostic.
        self.resolve_nominal_members_and_constants(
            &declaration_lanes,
            sorted_headers,
            &trait_environment,
            &aliases_waiting_for_constants,
            string_table,
        )?;

        // --------------------------------------
        //  Resolve nominal generic bound traits
        // --------------------------------------
        self.resolve_nominal_generic_bounds(
            &declaration_lanes,
            sorted_headers,
            &trait_environment,
            string_table,
        )?;

        // -----------------------------
        //  Resolve function signatures
        // -----------------------------
        self.resolve_function_signatures(
            &declaration_lanes,
            sorted_headers,
            &trait_environment,
            string_table,
        )?;

        // ------------------------
        //  Build receiver catalog
        // ------------------------
        let receiver_methods = self.build_receiver_catalog(sorted_headers, string_table)?;
        self.validate_receiver_method_visibility_invariants(&receiver_methods, string_table)?;

        // Register compiler-owned builtin evidence rows for every initial
        // (source, target) row in the cast plan. Must run before
        // `validate_trait_evidence` so user-declared conformances that would
        // override builtin evidence or conflict with incompatible builtin
        // evidence are rejected while trait ids are already stable.
        let mut trait_evidence_environment = TraitEvidenceEnvironment::new();
        Self::register_builtin_cast_evidence(
            &trait_environment,
            &mut trait_evidence_environment,
            &self.type_environment,
            string_table,
        )?;
        self.project_imported_trait_evidence(
            &trait_environment,
            &mut trait_evidence_environment,
            string_table,
        )
        .map_err(|error| self.error_messages(error, string_table))?;

        // ---------------------------
        //  Validate trait evidence
        // ---------------------------
        validate_trait_evidence(
            ValidateTraitEvidenceInput {
                sorted_headers,
                trait_environment: &trait_environment,
                receiver_methods: receiver_methods.as_ref(),
                type_environment: &self.type_environment,
                binding_environment: &self.binding_environment,
                nominal_type_ids_by_path: &self.nominal_type_ids_by_path,
                struct_source_by_path: &self.struct_source_by_path,
                choice_source_by_path: &self.choice_source_by_path,
                string_table,
            },
            &mut trait_evidence_environment,
        )
        .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

        // -----------------------------------------
        //  Validate bounded nominal instantiations
        // -----------------------------------------
        self.validate_nominal_generic_bound_surfaces(
            &declaration_lanes,
            sorted_headers,
            &trait_environment,
            &trait_evidence_environment,
            string_table,
        )?;

        // --------------------------------
        //  Validate public export surfaces
        // --------------------------------
        self.validate_public_export_surfaces(sorted_headers, &trait_environment, string_table)?;

        // ---------------------------------------------
        //  Build resolved public type-root handoff
        // ---------------------------------------------
        // WHAT: retains one transient AST-owned table of directly-defined active-root public
        // type roots and their attached receiver methods from the same already-resolved facts
        // used by public-surface validation. Re-exported declarations from private files targeted
        // by public export entries are also included. Donor-local TypeIds stay inside the Ast
        // handoff and never enter a cross-module artefact.

        // Build the set of re-exported source declaration paths targeted by public export entries.
        // These are declarations from private files re-exported through the root's `export:`
        // block. They are not in the active root file, so the directly-defined pass excludes them.
        let reexport_target_paths =
            collect_reexport_target_paths(&self.context.entry_dir, &self.module_symbols);

        let resolved_public_type_roots =
            build_resolved_public_type_roots(BuildResolvedPublicTypeRootsInput {
                sorted_headers,
                resolved_struct_fields_by_path: &self.resolved_struct_fields_by_path,
                resolved_function_signatures_by_path: &self.resolved_function_signatures_by_path,
                nominal_type_ids_by_path: &self.nominal_type_ids_by_path,
                resolved_type_aliases_by_path: &self.resolved_type_aliases_by_path,
                declaration_table: self.declaration_table.as_ref(),
                generic_function_templates_by_path: &self.generic_function_templates_by_path,
                receiver_methods: receiver_methods.as_ref(),
                trait_environment: &trait_environment,
                type_environment: &self.type_environment,
                string_table,
                reexport_target_paths: &reexport_target_paths,
            })
            .map_err(|error| self.error_messages(error, string_table))?;

        // Build the transient direct public trait-root vector from the same sorted headers.
        // The type-root table stays type-only; trait-root facts live in their own owner.
        let resolved_public_trait_roots = build_resolved_public_trait_roots(
            sorted_headers,
            &reexport_target_paths,
            &trait_environment,
            string_table,
        )
        .map_err(|error| self.error_messages(error, string_table))?;

        self.finish_environment(
            receiver_methods,
            trait_environment,
            trait_evidence_environment,
            ResolvedPublicSurfaceOutputs {
                type_roots: resolved_public_type_roots,
                trait_roots: resolved_public_trait_roots,
            },
            string_table,
        )
    }

    /// Assemble the completed immutable environment package consumed by body emission.
    ///
    /// WHAT: moves the builder's resolved side tables into `AstModuleLookups`
    /// and pairs them with the canonical `TypeEnvironment`.
    /// WHY: keeping final assembly in one helper makes `build` read as the
    /// semantic phase pipeline instead of ending with a large structural move.
    fn finish_environment(
        self,
        receiver_methods: Rc<ReceiverMethodCatalog>,
        trait_environment: TraitEnvironment,
        trait_evidence_environment: TraitEvidenceEnvironment,
        resolved_public_surface_outputs: ResolvedPublicSurfaceOutputs,
        string_table: &StringTable,
    ) -> Result<AstModuleEnvironment, CompilerMessages> {
        let source_nominal_paths = self
            .struct_source_by_path
            .keys()
            .chain(self.choice_source_by_path.keys())
            .cloned()
            .collect();
        let declaration_semantics = DeclarationSemanticTable::from_environment(
            self.declaration_table.as_ref(),
            &self.resolved_function_signatures_by_path,
            &self.nominal_type_ids_by_path,
            &self.type_environment,
            &self.context.template_ir_store,
        )
        .map_err(|error| match error {
            TemplateError::Diagnostic(diagnostic) => {
                self.diagnostic_messages(*diagnostic, string_table)
            }
            TemplateError::Infrastructure(error) => self.error_messages(*error, string_table),
        })?;

        Ok(AstModuleEnvironment {
            lookups: Rc::new(AstModuleLookups {
                module_symbols: self.module_symbols,
                binding_environment: self.binding_environment,
                warnings: self.warnings,
                declaration_table: self.declaration_table,
                imported_functions_by_local_path: self.projected_imported_functions_by_local_path,
                imported_struct_definitions: self.imported_struct_definitions,
                imported_choice_definitions: self.imported_choice_definitions,
                resolved_module_constants: self.resolved_module_constants,
                builtin_struct_ast_nodes: self.builtin_struct_ast_nodes,

                resolved_struct_fields_by_path: self.resolved_struct_fields_by_path,
                resolved_function_signatures_by_path: Rc::new(
                    self.resolved_function_signatures_by_path,
                ),
                generic_function_templates_by_path: self.generic_function_templates_by_path,
                resolved_type_aliases_by_path: self.resolved_type_aliases_by_path,
                choice_variant_shells_by_path: self.choice_variant_shells_by_path,
                declaration_semantics: Rc::new(declaration_semantics),

                receiver_methods,
                trait_environment: Rc::new(trait_environment),
                trait_evidence_environment: Rc::new(trait_evidence_environment),
                generic_declarations_by_path: self.generic_declarations_by_path,
                nominal_type_ids_by_path: self.nominal_type_ids_by_path,
                source_nominal_paths: Rc::new(source_nominal_paths),

                external_package_registry: Arc::clone(&self.context.external_package_registry),
                style_directives: self.context.style_directives.clone(),
                build_profile: self.context.build_profile,
            }),
            generated_evidence_pairs: Rc::new(FxHashSet::default()),
            resolved_public_type_roots: resolved_public_surface_outputs.type_roots,
            resolved_public_trait_roots: resolved_public_surface_outputs.trait_roots,

            type_environment: self.type_environment,
        })
    }

    /// The header-built file visibility package for one header's source file.
    ///
    /// WHAT: one shared handle to the visibility the header/binding phase already computed.
    /// WHY: every environment pass opens by fetching this, and the fetch is a fallible lookup with
    /// a diagnostic conversion, so spelling it out per pass put five lines of error plumbing in
    /// front of the work each pass actually does.
    pub(crate) fn header_visibility(
        &self,
        header: &Header,
        string_table: &StringTable,
    ) -> Result<Arc<FileVisibility>, CompilerMessages> {
        self.binding_environment
            .visibility_for(&header.source_file)
            .map(Arc::clone)
            .map_err(|error| self.error_messages(error, string_table))
    }

    /// Resolve the declaration-site generic parameter scope for one declaration.
    ///
    /// WHAT: names every generic parameter the declaration introduces, gated by the file's
    /// visibility so a parameter cannot shadow a visible declaration.
    /// WHY: six environment passes were spelling out the same eight-field input, differing only in
    /// which parameter list and canonical map they pass. The five fields they always agree on -
    /// the three visibility maps, the declaration table and the generic metadata - belong to the
    /// builder, so it supplies them.
    pub(crate) fn generic_parameter_scope(
        &self,
        generic_parameters: &GenericParameterList,
        canonical_by_local: Option<&FxHashMap<TypeParameterId, GenericParameterId>>,
        visibility: &FileVisibility,
        string_table: &StringTable,
    ) -> Result<Option<GenericParameterScope>, CompilerMessages> {
        build_generic_parameter_scope(GenericParameterScopeBuildInput {
            generic_parameters,
            canonical_by_local,
            visible_source_bindings: &visibility.visible_source_names,
            visible_type_aliases: &visibility.visible_type_alias_names,
            visible_external_symbols: &visibility.visible_external_symbols,
            declaration_table: self.declaration_table.as_ref(),
            generic_declarations_by_path: &self.generic_declarations_by_path,
            string_table,
        })
        .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))
    }

    /// The same scope, taking the canonical parameter map from the header's registered list.
    ///
    /// Nominal passes register a canonical generic parameter list per header before they resolve
    /// members, so they look the map up by path rather than carrying it.
    pub(crate) fn generic_parameter_scope_for_header(
        &self,
        header: &Header,
        generic_parameters: &GenericParameterList,
        visibility: &FileVisibility,
        string_table: &StringTable,
    ) -> Result<Option<GenericParameterScope>, CompilerMessages> {
        self.generic_parameter_scope(
            generic_parameters,
            self.generic_parameter_lists_by_path
                .get(&header.tokens.src_path)
                .map(|registered| &registered.canonical_by_local),
            visibility,
            string_table,
        )
    }

    /// Build the scope base every environment-time pass parses one header in.
    ///
    /// WHAT: a `ConstantHeader` scope carrying the module services and the four side tables that
    /// nominal member shells, trait requirement signatures and function signatures all read.
    /// WHY: those three passes were each assembling this by hand, and the copies were identical
    /// down to the setter order. What they genuinely differ on - file visibility, the resolved
    /// constant set, choice variant shells - the caller adds on top, so a divergence has to be
    /// written at the call site rather than hiding inside a copied chain.
    ///
    /// MUST NOT: copy the side tables. This runs once per header per pass, so a copy here costs
    /// the whole module per header and makes the environment passes quadratic in module size.
    /// Every table below is taken as a shared handle; the builder writes through `Rc::make_mut`
    /// once the scope has been dropped.
    pub(crate) fn environment_header_scope(
        &self,
        header: &Header,
        string_table: &mut StringTable,
    ) -> ScopeContext {
        let source_file_scope = header.canonical_source_file(string_table);

        let mut context = ScopeContext::new(
            ContextKind::ConstantHeader,
            header.tokens.src_path.to_owned(),
            Rc::clone(&self.declaration_table),
            Arc::clone(&self.context.external_package_registry),
            vec![],
            0,
            Rc::clone(&self.context.template_ir_store),
        )
        .with_style_directives(self.context.style_directives)
        .with_build_profile(self.context.build_profile)
        .with_resolved_type_aliases(Rc::clone(&self.resolved_type_aliases_by_path))
        .with_generic_declarations(Rc::clone(&self.generic_declarations_by_path))
        .with_resolved_struct_fields_by_path(Rc::clone(&self.resolved_struct_fields_by_path))
        .with_nominal_type_ids_by_path(Rc::clone(&self.nominal_type_ids_by_path))
        .with_source_file_scope(source_file_scope)
        .with_declaring_file_id(header.tokens.file_id);
        if let Some(services) = &self.context.file_value_resolution {
            context = context.with_file_value_resolution(Rc::clone(services));
        }
        context
    }

    pub(crate) fn replace_declaration(
        &mut self,
        declaration_id: DeclarationId,
        declaration: Declaration,
    ) -> Result<(), CompilerError> {
        if !self
            .declaration_table_mut()?
            .replace_by_id(declaration_id, declaration)
        {
            return Err(CompilerError::compiler_error(
                "Resolved top-level declaration was not registered before AST resolution.",
            ));
        }

        Ok(())
    }

    /// Publish one dependency-ordered module constant to later environment passes.
    pub(crate) fn publish_resolved_module_constant(&mut self, declaration_id: DeclarationId) {
        Rc::make_mut(&mut self.resolved_module_constants).insert(declaration_id);
    }

    pub(crate) fn declaration_table_mut(
        &mut self,
    ) -> Result<&mut TopLevelDeclarationTable, CompilerError> {
        Rc::get_mut(&mut self.declaration_table).ok_or_else(|| {
            CompilerError::compiler_error(
                "AST declaration table was still shared while environment construction tried to mutate it.",
            )
        })
    }

    /// Register builtin struct definitions in the TypeEnvironment and update their
    /// declaration-table entries with real TypeIds.
    ///
    /// WHAT: builtin structs are created programmatically during header parsing with
    /// `TypeId(0)` placeholders. They must be canonicalised in `TypeEnvironment` before
    /// any expression parsing that touches their fields (e.g. `error.message`).
    /// WHY: body parsing queries `TypeEnvironment` via the `ScopeContext` environment;
    /// unregistered builtins return empty field lists and break field access.
    pub(crate) fn register_builtin_structs_in_type_environment(
        &mut self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let builtin_paths = [builtin_error_type_path(string_table)];

        for path in &builtin_paths {
            let Some(fields) = self.resolved_struct_fields_by_path.get(path).cloned() else {
                continue;
            };

            let field_definitions =
                self.field_definitions_from_declarations(&fields, string_table)?;

            let struct_def = StructTypeDefinition {
                id: NominalTypeId(0),
                path: path.clone(),
                fields: field_definitions,
                generic_parameters: None,
                const_record: false,
            };
            let (_, struct_type_id) = self.type_environment.register_nominal_struct(struct_def);
            self.type_environment
                .register_canonical_identity(
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
                    struct_type_id,
                )
                .map_err(|error| self.error_messages(error, string_table))?;
            Rc::make_mut(&mut self.nominal_type_ids_by_path).insert(path.clone(), struct_type_id);

            // Build a placeholder declaration so the builtin struct is reachable
            // through the declaration table during body parsing.
            let declaration_location = fields
                .first()
                .map(|field| field.value.location.clone())
                .unwrap_or_default();

            let declaration_id = self
                .declaration_table
                .declaration_id_by_path(path)
                .ok_or_else(|| {
                    self.error_messages(
                        CompilerError::compiler_error(
                            "Builtin declaration was not registered before type resolution.",
                        ),
                        string_table,
                    )
                })?;
            self.replace_declaration(
                declaration_id,
                Declaration {
                    id: path.clone(),
                    value: Expression::new(
                        ExpressionKind::NoValue,
                        declaration_location,
                        struct_type_id,
                        DataType::runtime_struct(path.clone(), struct_type_id),
                        ValueMode::ImmutableReference,
                    ),
                    config_qualifier: None,
                },
            )
            .map_err(|error| self.error_messages(error, string_table))?;
        }

        Ok(())
    }

    /// Build a `TypeResolutionContext` from the current environment state and file visibility.
    ///
    /// WHAT: centralizes the repeated `TypeResolutionContext::from_inputs(...)` construction
    /// across type alias, struct field, choice variant, and function signature resolution.
    /// WHY: avoids duplicating the same 8-field initialization in four different files.
    pub(crate) fn type_resolution_context_for<'a>(
        &'a mut self,
        visibility: &'a FileVisibility,
        generic_parameters: Option<&'a GenericParameterScope>,
    ) -> TypeResolutionContext<'a> {
        self.type_resolution_context_for_with_traits(visibility, generic_parameters, None)
    }

    pub(crate) fn type_resolution_context_for_with_traits<'a>(
        &'a mut self,
        visibility: &'a FileVisibility,
        generic_parameters: Option<&'a GenericParameterScope>,
        trait_environment: Option<&'a TraitEnvironment>,
    ) -> TypeResolutionContext<'a> {
        let mut context = TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
            declaration_table: &self.declaration_table,
            visible_declaration_ids: Some(&visibility.visible_declaration_paths),
            visible_external_symbols: Some(&visibility.visible_external_symbols),
            visible_source_bindings: Some(&visibility.visible_source_names),
            visible_type_aliases: Some(&visibility.visible_type_alias_names),
            resolved_type_aliases: Some(&self.resolved_type_aliases_by_path),
            generic_declarations_by_path: Some(&self.generic_declarations_by_path),
            resolved_struct_fields_by_path: Some(&self.resolved_struct_fields_by_path),
            type_environment: &mut self.type_environment,
            visible_namespace_records: Some(&visibility.visible_namespace_records),
            trait_environment,
            trait_evidence_environment: None,
            visible_trait_names: Some(&visibility.visible_trait_names),
        });
        if let Some(gp) = generic_parameters {
            context = context.with_generic_parameters(Some(gp));
        }
        context
    }

    pub(in crate::compiler_frontend::ast) fn resolve_generic_parameter_bounds(
        &self,
        generic_parameters: &GenericParameterList,
        visibility: &FileVisibility,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<FxHashMap<TypeParameterId, Vec<TraitId>>, CompilerMessages> {
        let mut resolved_bounds_by_local = FxHashMap::default();

        for parameter in &generic_parameters.parameters {
            if parameter.trait_bounds.is_empty() {
                continue;
            }

            let mut resolved_bounds = Vec::with_capacity(parameter.trait_bounds.len());
            for trait_bound in &parameter.trait_bounds {
                let trait_ref = TraitReferenceSyntax {
                    name: trait_bound.trait_name,
                    location: trait_bound.location.clone(),
                };
                let trait_id = self.resolve_visible_trait_reference(
                    &trait_ref,
                    visibility,
                    trait_environment,
                    string_table,
                )?;
                resolved_bounds.push(trait_id);
            }

            resolved_bounds_by_local.insert(parameter.id, resolved_bounds);
        }

        Ok(resolved_bounds_by_local)
    }

    pub(in crate::compiler_frontend::ast) fn validate_public_generic_bounds(
        &self,
        owner_name: StringId,
        generic_parameters: &GenericParameterList,
        resolved_bounds_by_local: &FxHashMap<TypeParameterId, Vec<TraitId>>,
        public_root_file: &InternedPath,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for parameter in &generic_parameters.parameters {
            let Some(resolved_bounds) = resolved_bounds_by_local.get(&parameter.id) else {
                continue;
            };

            for (trait_bound, trait_id) in parameter.trait_bounds.iter().zip(resolved_bounds) {
                let Some(trait_definition) = trait_environment.get(*trait_id) else {
                    return Err(self.error_messages(
                        CompilerError::compiler_error(
                            "Generic bound resolved to missing trait definition.",
                        ),
                        string_table,
                    ));
                };

                // Public generic signatures are consumed through the public export surface alone,
                // so every bound trait must be available from that same surface.
                if self.public_trait_definition_is_nameable(
                    trait_definition,
                    public_root_file,
                    trait_environment,
                ) {
                    continue;
                }

                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::generic_bound_private_surface_leak(
                        owner_name,
                        trait_definition.name,
                        trait_bound.location.clone(),
                    ),
                    string_table,
                ));
            }
        }

        Ok(())
    }

    /// Convert resolved AST member declarations into canonical type-environment fields.
    ///
    /// WHAT: struct fields and choice payload fields are resolved as AST `Declaration`s first,
    /// then written into `TypeEnvironment` as compact semantic member definitions.
    /// WHY: keeping the conversion on the environment builder centralizes diagnostic mapping
    /// at the AST environment boundary and avoids repeated large-error iterator closures.
    pub(crate) fn field_definitions_from_declarations(
        &mut self,
        fields: &[Declaration],
        string_table: &StringTable,
    ) -> Result<Box<[FieldDefinition]>, CompilerMessages> {
        let mut definitions = Vec::with_capacity(fields.len());

        for field in fields {
            let type_id = match resolve_diagnostic_type_to_type_id_checked(
                &field.value.diagnostic_type,
                &mut self.type_environment,
                &field.value.location,
            ) {
                Ok(type_id) => type_id,
                Err(diagnostic) => {
                    return Err(self.diagnostic_messages(*diagnostic, string_table));
                }
            };

            definitions.push(FieldDefinition {
                name: field.id.clone(),
                type_id,
                location: field.value.location.clone(),
            });
        }

        Ok(definitions.into_boxed_slice())
    }

    pub(crate) fn error_messages(
        &self,
        error: CompilerError,
        string_table: &StringTable,
    ) -> CompilerMessages {
        CompilerMessages::from_error_with_warnings(error, self.warnings.clone(), string_table)
            .with_type_context_for_all_diagnostics(self.type_environment.clone())
    }

    pub(crate) fn diagnostic_messages(
        &self,
        diagnostic: CompilerDiagnostic,
        string_table: &StringTable,
    ) -> CompilerMessages {
        CompilerMessages::from_diagnostic_with_warnings(
            diagnostic,
            self.warnings.clone(),
            string_table,
        )
        .with_type_context_for_all_diagnostics(self.type_environment.clone())
    }

    /// Preserve the internal lane produced by AST parsers that materialise frozen token slices.
    pub(crate) fn expression_error_messages(
        &self,
        error: ExpressionParseError,
        string_table: &StringTable,
    ) -> CompilerMessages {
        match error {
            ExpressionParseError::Diagnostic(diagnostic) => {
                self.diagnostic_messages(*diagnostic, string_table)
            }
            ExpressionParseError::Infrastructure(error) => {
                self.error_messages(*error, string_table)
            }
        }
    }
}
