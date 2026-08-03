//! AST module environment builder.
//!
//! WHAT: consumes header-built import visibility and resolves declarations, constants, nominal
//! types, function signatures, and receiver catalog data into a stable semantic environment.
//! WHY: after this phase completes, AST emission can parse bodies against a stable environment
//! instead of depending on pass-order-specific accumulator fields.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironment, AstModuleLookups, BuildResolvedPublicTypeRootsInput,
    DeclarationSemanticTable, ResolvedPublicTraitRoot, ResolvedPublicTypeRootTable,
    TopLevelDeclarationTable, build_resolved_public_trait_roots, build_resolved_public_type_roots,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodCatalog;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext, TemplateWrapperReference, TirSlotPlaceholder, summarize_existing_root,
};
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedTypeAnnotation, TypeResolutionContext, TypeResolutionContextInputs,
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
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::datatypes::{DataType, diagnostic_type_spelling};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::folded_value::{
    PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicFoldedField, PublicFoldedValue, PublicTemplateSlotKey,
};
use crate::compiler_frontend::headers::import_environment::{
    FileVisibility, HeaderImportEnvironment,
};
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationMetadata, ModuleSymbols,
};
use crate::compiler_frontend::headers::parse_file_headers::Header;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
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
use crate::{benchmark_timer_log, timer_log};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

pub(in crate::compiler_frontend::ast) mod import_projection;

#[cfg(test)]
pub(crate) use import_projection::imported_nominal_path;

/// Collect the set of source declaration paths re-exported through the root's `export:` block
/// from private files in the same module.
///
/// WHAT: iterates the header-built public export maps and collects every `PublicExportTarget::Source`
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
            let crate::compiler_frontend::headers::module_symbols::PublicExportTarget::Source {
                path,
                ..
            } = &entry.target
            else {
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

    // Header-built import visibility consumed directly; AST does not rebuild import bindings.
    pub(crate) import_environment: HeaderImportEnvironment,

    // Mutable environment-building state.
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    pub(crate) declaration_table: Rc<TopLevelDeclarationTable>,
    pub(crate) module_constants: Vec<Declaration>,
    pub(crate) rendered_path_usages: Rc<RefCell<Vec<RenderedPathUsage>>>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<InternedPath, Vec<Declaration>>,
    pub(crate) struct_source_by_path: FxHashMap<InternedPath, InternedPath>,
    pub(crate) choice_source_by_path: FxHashMap<InternedPath, InternedPath>,
    pub(crate) choice_variant_shells_by_path: FxHashMap<InternedPath, Vec<ChoiceVariant>>,
    pub(crate) resolved_function_signatures_by_path:
        FxHashMap<InternedPath, ResolvedFunctionSignature>,
    pub(crate) generic_function_templates_by_path: FxHashMap<InternedPath, GenericFunctionTemplate>,
    pub(crate) resolved_type_aliases_by_path: FxHashMap<InternedPath, ResolvedTypeAnnotation>,
    pub(crate) generic_parameter_lists_by_path:
        FxHashMap<InternedPath, RegisteredGenericParameterList>,

    // Frontend semantic type identity built during environment construction.
    // WHY: parsed types are resolved into canonical TypeIds as declarations are processed.
    pub(crate) type_environment: TypeEnvironment,

    // Canonical TypeId for each nominal struct/choice registered in type_environment.
    pub(crate) nominal_type_ids_by_path: FxHashMap<InternedPath, TypeId>,
    imported_type_ids_by_origin: FxHashMap<OriginTypeId, TypeId>,
    imported_generic_parameter_type_ids: FxHashMap<
        crate::compiler_frontend::canonical_type_identity::ExportedGenericParameterIdentity,
        TypeId,
    >,
    imported_generic_parameter_registrations: Vec<ImportedGenericParameterRegistration>,
    pub(super) projected_imported_functions_by_local_path:
        FxHashMap<InternedPath, AstImportedFunctionContract>,
    imported_struct_definitions: Vec<AstImportedStructDefinition>,
    imported_choice_definitions: Vec<AstChoiceDefinition>,
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(crate) fn new(context: &'context AstPhaseContext<'services>) -> Self {
        Self {
            context,
            module_symbols: ModuleSymbols::empty(),
            import_environment: HeaderImportEnvironment::default(),
            warnings: Vec::new(),
            declaration_table: Rc::new(TopLevelDeclarationTable::new(Vec::new())),
            module_constants: Vec::new(),
            rendered_path_usages: Rc::new(RefCell::new(Vec::new())),
            builtin_struct_ast_nodes: Vec::new(),
            resolved_struct_fields_by_path: FxHashMap::default(),
            struct_source_by_path: FxHashMap::default(),
            choice_source_by_path: FxHashMap::default(),
            choice_variant_shells_by_path: FxHashMap::default(),
            resolved_function_signatures_by_path: FxHashMap::default(),
            generic_function_templates_by_path: FxHashMap::default(),
            resolved_type_aliases_by_path: FxHashMap::default(),
            generic_parameter_lists_by_path: FxHashMap::default(),
            type_environment: TypeEnvironment::new(),
            nominal_type_ids_by_path: FxHashMap::default(),
            imported_type_ids_by_origin: FxHashMap::default(),
            imported_generic_parameter_type_ids: FxHashMap::default(),
            imported_generic_parameter_registrations: Vec::new(),
            projected_imported_functions_by_local_path: FxHashMap::default(),
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
            import_environment,
        } = input;

        // Move header-owned data into the builder state.
        let declarations = std::mem::take(&mut module_symbols.declarations);
        let builtin_struct_ast_nodes = std::mem::take(&mut module_symbols.builtin_struct_ast_nodes);
        let resolved_struct_fields_by_path =
            std::mem::take(&mut module_symbols.resolved_struct_fields_by_path);
        let struct_source_by_path = std::mem::take(&mut module_symbols.struct_source_by_path);

        self.module_symbols = module_symbols;
        self.import_environment = import_environment;
        self.warnings = self.import_environment.warnings.clone();
        self.declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations));
        self.builtin_struct_ast_nodes = builtin_struct_ast_nodes;
        self.resolved_struct_fields_by_path = resolved_struct_fields_by_path;
        self.struct_source_by_path = struct_source_by_path;

        let environment_start = Instant::now();

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

        // ----------------------
        //  Resolve type aliases
        // ----------------------
        let type_alias_resolution_start = Instant::now();
        self.resolve_type_aliases(sorted_headers, string_table)?;
        timer_log!(
            type_alias_resolution_start,
            "AST/environment/type aliases resolved in: "
        );
        let _ = type_alias_resolution_start;

        // --------------------------------------------
        //  Register nominal struct and choice shells
        // --------------------------------------------
        // WHAT: register identities early so trait requirement signatures and dynamic
        // trait annotations can reference nominal types before fields are resolved.
        let shell_registration_start = Instant::now();
        self.register_nominal_shells(sorted_headers, string_table)?;
        timer_log!(
            shell_registration_start,
            "AST/environment/nominal shells registered in: "
        );
        let _ = shell_registration_start;

        // --------------------------
        //  Resolve trait metadata
        // --------------------------
        // Trait definitions are needed before function signatures so declaration-site
        // generic bounds can be resolved into canonical TraitIds. They are also needed
        // before struct fields, choice payloads, and constants so trait names in ordinary
        // type positions can be rejected with the trait-specific diagnostic.
        // Evidence validation stays after receiver catalog construction because it needs
        // resolved receiver methods.
        let trait_resolution_start = Instant::now();
        let trait_environment = self.resolve_trait_definitions(sorted_headers, string_table)?;
        self.resolve_imported_generic_parameter_bounds(&trait_environment)
            .map_err(|error| self.error_messages(error, string_table))?;
        timer_log!(
            trait_resolution_start,
            "AST/environment/trait definitions resolved in: "
        );
        let _ = trait_resolution_start;

        // -------------------------------------------
        //  Resolve nominal members and constants
        // -------------------------------------------
        // WHAT: resolves constructor shells, constants, struct fields, and choice
        // payload types with trait-aware type resolution.
        // WHY: static trait metadata must be available so ordinary type annotations can
        // reject trait names without falling through to an unknown-type diagnostic.
        let member_resolution_start = Instant::now();
        self.resolve_nominal_members_and_constants(
            sorted_headers,
            &trait_environment,
            string_table,
        )?;
        timer_log!(
            member_resolution_start,
            "AST/environment/nominal members and constants resolved in: "
        );
        let _ = member_resolution_start;

        // --------------------------------------
        //  Resolve nominal generic bound traits
        // --------------------------------------
        let nominal_bound_resolution_start = Instant::now();
        self.resolve_nominal_generic_bounds(sorted_headers, &trait_environment, string_table)?;
        timer_log!(
            nominal_bound_resolution_start,
            "AST/environment/nominal generic bounds resolved in: "
        );
        let _ = nominal_bound_resolution_start;

        // -----------------------------
        //  Resolve function signatures
        // -----------------------------
        let function_signatures_start = Instant::now();
        self.resolve_function_signatures(sorted_headers, &trait_environment, string_table)?;
        timer_log!(
            function_signatures_start,
            "AST/environment/function signatures resolved in: "
        );
        let _ = function_signatures_start;

        // ------------------------
        //  Build receiver catalog
        // ------------------------
        let receiver_catalog_start = Instant::now();
        let receiver_methods = self.build_receiver_catalog(sorted_headers, string_table)?;
        self.validate_receiver_method_visibility_invariants(&receiver_methods, string_table)?;
        timer_log!(
            receiver_catalog_start,
            "AST/environment/receiver catalog built in: "
        );
        let _ = receiver_catalog_start;

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
        let trait_evidence_start = Instant::now();
        validate_trait_evidence(
            ValidateTraitEvidenceInput {
                sorted_headers,
                trait_environment: &trait_environment,
                receiver_methods: receiver_methods.as_ref(),
                type_environment: &self.type_environment,
                import_environment: &self.import_environment,
                nominal_type_ids_by_path: &self.nominal_type_ids_by_path,
                struct_source_by_path: &self.struct_source_by_path,
                choice_source_by_path: &self.choice_source_by_path,
                string_table,
            },
            &mut trait_evidence_environment,
        )
        .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;
        timer_log!(
            trait_evidence_start,
            "AST/environment/trait evidence resolved in: "
        );
        let _ = trait_evidence_start;

        // -----------------------------------------
        //  Validate bounded nominal instantiations
        // -----------------------------------------
        let nominal_bound_surface_start = Instant::now();
        self.validate_nominal_generic_bound_surfaces(
            sorted_headers,
            &trait_environment,
            &trait_evidence_environment,
            string_table,
        )?;
        timer_log!(
            nominal_bound_surface_start,
            "AST/environment/nominal generic bound surfaces validated in: "
        );
        let _ = nominal_bound_surface_start;

        // --------------------------------
        //  Validate public export surfaces
        // --------------------------------
        let public_surface_start = Instant::now();
        self.validate_public_export_surfaces(sorted_headers, &trait_environment, string_table)?;
        timer_log!(
            public_surface_start,
            "AST/environment/public export surfaces validated in: "
        );
        let _ = public_surface_start;

        // ---------------------------------------------
        //  Build resolved public type-root handoff
        // ---------------------------------------------
        // WHAT: retains one transient AST-owned table of directly-defined active-root public
        // type roots and their attached receiver methods from the same already-resolved facts
        // used by public-surface validation. Re-exported declarations from private files targeted
        // by public export entries are also included. Donor-local TypeIds stay inside the Ast
        // handoff and never enter a cross-module artefact.
        let public_type_roots_start = Instant::now();

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
        timer_log!(
            public_type_roots_start,
            "AST/environment/resolved public type roots built in: "
        );
        let _ = public_type_roots_start;

        // Build the transient direct public trait-root vector from the same sorted headers.
        // The type-root table stays type-only; trait-root facts live in their own owner.
        let resolved_public_trait_roots = build_resolved_public_trait_roots(
            sorted_headers,
            &reexport_target_paths,
            &trait_environment,
            string_table,
        )
        .map_err(|error| self.error_messages(error, string_table))?;

        benchmark_timer_log!(
            environment_start,
            "ast_build_environment_ms",
            "AST/build environment completed in: "
        );
        let _ = environment_start;

        // Extract generic declarations before `self` is consumed by `finish_environment`.
        let generic_declarations_by_path =
            std::mem::take(&mut self.module_symbols.generic_declarations_by_path);

        self.finish_environment(
            receiver_methods,
            trait_environment,
            trait_evidence_environment,
            generic_declarations_by_path,
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
        generic_declarations_by_path: FxHashMap<InternedPath, GenericDeclarationMetadata>,
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
        .map_err(|error| {
            self.diagnostic_messages(TemplateError::into_diagnostic(error), string_table)
        })?;

        Ok(AstModuleEnvironment {
            lookups: Rc::new(AstModuleLookups {
                module_symbols: self.module_symbols,
                import_environment: self.import_environment,
                warnings: self.warnings,
                declaration_table: self.declaration_table,
                imported_functions_by_local_path: self.projected_imported_functions_by_local_path,
                imported_struct_definitions: self.imported_struct_definitions,
                imported_choice_definitions: self.imported_choice_definitions,
                module_constants: self.module_constants,
                rendered_path_usages: self.rendered_path_usages,
                builtin_struct_ast_nodes: self.builtin_struct_ast_nodes,

                resolved_struct_fields_by_path: Rc::new(self.resolved_struct_fields_by_path),
                resolved_function_signatures_by_path: Rc::new(
                    self.resolved_function_signatures_by_path,
                ),
                generic_function_templates_by_path: self.generic_function_templates_by_path,
                resolved_type_aliases_by_path: Rc::new(self.resolved_type_aliases_by_path),
                choice_variant_shells_by_path: Rc::new(self.choice_variant_shells_by_path),
                declaration_semantics: Rc::new(declaration_semantics),

                receiver_methods,
                trait_environment: Rc::new(trait_environment),
                trait_evidence_environment: Rc::new(trait_evidence_environment),
                generic_declarations_by_path: Rc::new(generic_declarations_by_path),
                nominal_type_ids_by_path: Rc::new(self.nominal_type_ids_by_path),
                source_nominal_paths: Rc::new(source_nominal_paths),

                external_package_registry: Arc::clone(&self.context.external_package_registry),
                style_directives: self.context.style_directives.clone(),
                build_profile: self.context.build_profile,
                project_path_resolver: self.context.project_path_resolver.clone(),
                path_format_config: self.context.path_format_config.clone(),
            }),
            resolved_public_type_roots: resolved_public_surface_outputs.type_roots,
            resolved_public_trait_roots: resolved_public_surface_outputs.trait_roots,

            type_environment: self.type_environment,
        })
    }

    pub(crate) fn replace_declaration(
        &mut self,
        declaration: Declaration,
    ) -> Result<(), CompilerError> {
        increment_ast_counter(AstCounter::DeclarationTableReplacements);

        if self
            .declaration_table_mut()?
            .replace_by_path(declaration)
            .is_none()
        {
            return Err(CompilerError::compiler_error(
                "Resolved top-level declaration was not registered before AST resolution.",
            ));
        }

        Ok(())
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
            self.nominal_type_ids_by_path
                .insert(path.clone(), struct_type_id);

            // Build a placeholder declaration so the builtin struct is reachable
            // through the declaration table during body parsing.
            let declaration_location = fields
                .first()
                .map(|field| field.value.location.clone())
                .unwrap_or_default();

            self.replace_declaration(Declaration {
                id: path.clone(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    declaration_location,
                    struct_type_id,
                    DataType::runtime_struct(path.clone(), struct_type_id),
                    ValueMode::ImmutableReference,
                ),
            })
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
            generic_declarations_by_path: Some(&self.module_symbols.generic_declarations_by_path),
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
}
