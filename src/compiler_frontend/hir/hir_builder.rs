//! Stateful AST-to-HIR lowering builder.
//!
//! WHAT: lowers typed AST nodes into backend-facing HIR by allocating IDs,
//! registering declarations, constructing explicit blocks/regions/locals, and
//! attaching source mappings to the HIR side table.
//! WHY: HIR is the compiler boundary consumed by borrow validation and backend
//! lowering, so this builder owns construction state but not borrow facts,
//! ownership eligibility, or backend-specific output decisions.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only. Normal user-facing source failures
//! must be emitted as `CompilerDiagnostic` from AST or earlier stages.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, SourceLocation};
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::const_facts::HirConstFacts;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOriginLookup};
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::{
    BlockId, ChoiceId, FieldId, FunctionId, HirConstId, HirNodeId, HirValueId, LocalId, RegionId,
    StructId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::validation::validate_hir_module;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_metadata::{HirLoweringMetadata, HirLoweringResult};
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::return_hir_transformation_error;
use rustc_hash::FxHashMap;
use std::{cell::RefCell, rc::Rc};

mod metadata;
mod reactivity;

// -----------
// Entry Point
// -----------
pub(in crate::compiler_frontend) fn lower_module(
    ast: Ast,
    string_table: &mut StringTable,
    function_origin_lookup: HirFunctionOriginLookup,
    module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
) -> Result<HirLoweringResult, CompilerMessages> {
    let type_environment = ast.type_environment.clone();
    let mut ctx = HirBuilder::new(string_table, type_environment, function_origin_lookup);

    ctx.set_module_resources(module_resources);
    ctx.build_hir_module(ast)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LoopTargets {
    pub break_target: BlockId,
    pub continue_target: BlockId,
}

#[cfg(test)]
#[path = "tests/hir_builder_test_support.rs"]
mod hir_builder_test_support;
#[cfg(test)]
pub(crate) use hir_builder_test_support::{
    HirTestChoiceDefinition, assert_no_placeholder_terminators, build_ast_with_choices,
    build_ast_with_registered_types, expressions_to_owned_render_node,
    expressions_to_owned_render_node_with_resources, fixture_resource, lower_ast,
    lower_ast_with_metadata, register_local, runtime_template_expression, setup_builder,
    validate_module_for_tests,
};
// -------------------
// HIR Builder Context
// -------------------
//
// This struct is the main entry point for the HIR builder. It manages the state of the builder
// and provides the lowering logic for each AST node.
//
// The builder is stateful and re-entrant, so it's not safe to use concurrently.

pub struct HirBuilder<'a> {
    // === Result being built ===
    pub(super) module: HirModule,

    // WHAT: resolved documentation fragments pulled from the AST. These are compiler metadata,
    //       not executable HIR state, and are returned through the typed `HirLoweringMetadata`
    //       result boundary.
    pub(super) extracted_metadata: HirLoweringMetadata,

    // === AST warnings kept for error context only ===
    // WHAT: the AST's own warnings, held privately so failed lowering can render them alongside
    //       the failing HIR transformation error. Successful-module warnings are not duplicated
    //       here: frontend orchestration already owns the merged preparation + AST warning
    //       vector and remains the single successful-module warning source.
    ast_warnings: Vec<CompilerDiagnostic>,

    // === For variable name resolution ===
    pub(super) string_table: &'a mut StringTable,

    // === ID Counters ===
    next_block_id: u32,
    next_local_id: u32,
    next_node_id: u32,
    next_value_id: u32,
    next_region_id: u32,
    next_function_id: u32,
    next_struct_id: u32,
    next_field_id: u32,
    next_const_id: u32,
    next_choice_id: u32,
    pub(super) temp_local_counter: u32,

    // === Frontend type environment ===
    /// WHAT: carries the AST-built type environment while lowering one module.
    /// WHY: HIR stores frontend `TypeId`s directly and queries this table for type facts.
    pub(super) type_environment: TypeEnvironment,

    /// Shared module resource table for re-interning handoff resource origins.
    ///
    /// WHAT: the same `Rc` table AST file-value resolution interned origins into, captured so
    ///       owned runtime-template handoff pieces can regain module-local handles.
    /// WHY: the handoff carries `StableResourceOriginId` values, but HIR pieces use module-local
    ///       `ResourceId`s; `intern_origin` is idempotent per origin, so re-interning against
    ///       the issuing table returns the handle AST already minted instead of a duplicate.
    pub(super) module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,

    /// Transient exact-path lookup for public stable function origins.
    ///
    /// The lookup is consumed while lowering declarations. Only the resulting origin/local-ID
    /// maps remain on `HirModule`; donor-local paths never enter completed HIR artefacts.
    pub(super) function_origin_lookup: HirFunctionOriginLookup,

    // === Source / name side table ===
    pub(super) side_table: HirSideTable,

    // === Name resolution tables (filled during declaration pass) ===
    // AST guarantees module-wide unique InternedPath symbol IDs. HIR keys symbol resolution
    // by full paths, never by scope-local leaf strings.
    pub(super) locals_by_name: FxHashMap<InternedPath, LocalId>,
    pub(super) functions_by_name: FxHashMap<InternedPath, FunctionId>,
    pub(super) imported_functions_by_name: FxHashMap<InternedPath, AstImportedFunctionContract>,
    pub(super) imported_fallible_carriers_by_origin: FxHashMap<OriginFunctionId, TypeId>,
    pub(super) module_private_fallible_carriers_by_identity:
        FxHashMap<ModulePrivateExecutableIdentity, TypeId>,
    pub(super) generated_fallible_carriers_by_identity:
        FxHashMap<GeneratedFunctionIdentity, TypeId>,
    pub(super) structs_by_name: FxHashMap<InternedPath, StructId>,
    pub(super) choices_by_name: FxHashMap<InternedPath, ChoiceId>,
    /// Generic struct instantiations keyed by structured identity, not string paths.
    /// WHAT: `Box of Int` and `Box of String` need distinct StructIds.
    pub(super) generic_structs_by_key: FxHashMap<
        crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey,
        StructId,
    >,
    /// Generic choice instantiations keyed by structured identity.
    pub(super) generic_choices_by_key: FxHashMap<
        crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey,
        ChoiceId,
    >,
    pub(super) fields_by_struct_and_name: FxHashMap<(StructId, InternedPath), FieldId>,
    pub(super) module_constants_by_name: FxHashMap<InternedPath, ConstValueId>,
    pub(super) module_const_values: ConstValueStore,

    // === Fast ID -> arena index maps ===
    pub(super) block_index_by_id: FxHashMap<BlockId, usize>,
    pub(super) function_index_by_id: FxHashMap<FunctionId, usize>,
    pub(super) region_index_by_id: FxHashMap<RegionId, usize>,
    pub(super) local_index_by_id: FxHashMap<LocalId, (usize, usize)>,
    pub(super) struct_index_by_id: FxHashMap<StructId, usize>,
    pub(super) field_index_by_id: FxHashMap<FieldId, (usize, usize)>,

    // === Current Function State ===
    current_function: Option<FunctionId>,
    current_block: Option<BlockId>,
    current_region: Option<RegionId>,
    pub(super) loop_targets: Vec<LoopTargets>,

    /// The runtime fragment vec local inside entry start(), if currently lowering it.
    /// Set when entering entry start() and cleared on leave.
    pub(super) entry_fragment_vec_local: Option<LocalId>,

    /// Active target for value-block lowering.
    ///
    /// WHAT: when set, `ThenValue` statements inside the current statement-sequence lowering
    ///       assign their produced values to shared result locals and jump to `merge_block`.
    /// WHY: value-producing `if`, match, and catch branches use `ThenValue` to yield their
    ///      results; HIR lowering needs to intercept those statements and wire them to the
    ///      shared merge locals.
    pub(super) active_value_block_target: Option<ValueBlockTarget>,
}

/// Target state for value-block lowering inside `HirBuilder`.
///
/// WHAT: carries the result locals and merge block that `ThenValue` statements should use
///       when producing values inside a value-producing control-flow block.
/// WHY: multi-return value blocks need one local per slot; single-return keeps one local.
#[derive(Clone, Debug)]
pub(super) struct ValueBlockTarget {
    pub result_locals: Vec<LocalId>,
    pub merge_block: BlockId,
}

// WHAT: generates a typed `allocate_*_id` method for each HIR entity kind.
// WHY: all nine allocators share identical logic — bump a u32 counter, wrap in a newtype, return.
//      A module-level macro eliminates the repetition without changing the public API.
//      To add a new entity type: add the counter field to HirBuilder, then invoke this macro.
macro_rules! allocate_id {
    ($method:ident, $counter_field:ident, $id_type:ident) => {
        pub(crate) fn $method(&mut self) -> $id_type {
            let id = $id_type(self.$counter_field);
            self.$counter_field += 1;
            id
        }
    };
}

impl<'a> HirBuilder<'a> {
    // -------------------------
    //  Constructor & Utilities
    // -------------------------

    pub fn new(
        string_table: &'a mut StringTable,
        type_environment: TypeEnvironment,
        function_origin_lookup: HirFunctionOriginLookup,
    ) -> HirBuilder<'a> {
        HirBuilder {
            module: HirModule::new(),

            extracted_metadata: HirLoweringMetadata::new(),
            ast_warnings: Vec::new(),

            string_table,
            type_environment,
            module_resources: None,
            function_origin_lookup,

            next_block_id: 0,
            next_local_id: 0,
            next_node_id: 0,
            next_value_id: 0,
            next_region_id: 0,
            next_function_id: 0,
            next_struct_id: 0,
            next_field_id: 0,
            next_const_id: 0,
            next_choice_id: 0,
            temp_local_counter: 0,

            side_table: HirSideTable::default(),

            locals_by_name: FxHashMap::default(),
            functions_by_name: FxHashMap::default(),
            imported_functions_by_name: FxHashMap::default(),
            imported_fallible_carriers_by_origin: FxHashMap::default(),
            module_private_fallible_carriers_by_identity: FxHashMap::default(),
            generated_fallible_carriers_by_identity: FxHashMap::default(),
            structs_by_name: FxHashMap::default(),
            choices_by_name: FxHashMap::default(),
            generic_structs_by_key: FxHashMap::default(),
            generic_choices_by_key: FxHashMap::default(),
            fields_by_struct_and_name: FxHashMap::default(),
            module_constants_by_name: FxHashMap::default(),
            module_const_values: ConstValueStore::default(),

            block_index_by_id: FxHashMap::default(),
            function_index_by_id: FxHashMap::default(),
            region_index_by_id: FxHashMap::default(),
            local_index_by_id: FxHashMap::default(),
            struct_index_by_id: FxHashMap::default(),
            field_index_by_id: FxHashMap::default(),

            current_function: None,
            current_block: None,
            current_region: None,
            loop_targets: vec![],
            entry_fragment_vec_local: None,
            active_value_block_target: None,
        }
    }

    fn lower_error_messages(&self, error: CompilerError) -> CompilerMessages {
        CompilerMessages::from_error_with_warnings(
            error,
            self.ast_warnings.clone(),
            self.string_table,
        )
        .with_type_context_for_all_diagnostics(self.type_environment.clone())
    }

    /// Installs the shared module resource table captured by the lowering entry point.
    pub(super) fn set_module_resources(
        &mut self,
        module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    ) {
        self.module_resources = module_resources;
    }

    /// Re-intern one handoff resource origin into the issuing module resource table.
    ///
    /// WHAT: returns the module-local handle for a `StableResourceOriginId` carried by an owned
    ///       runtime-template handoff piece, failing the transform when the table is absent.
    /// WHY: handoff pieces cross the AST/HIR boundary with stable origins while HIR string
    ///       pieces carry `ResourceId`s; `intern_origin` is idempotent per origin, so
    ///       re-interning against the table that issued the handle mints nothing new. An absent
    ///       table mirrors the handoff's own absent-table rule, which already refuses to
    ///       materialize a structural string without the issuing table.
    pub(super) fn intern_handoff_resource_origin(
        &mut self,
        origin: &StableResourceOriginId,
        location: &SourceLocation,
    ) -> Result<ResourceId, CompilerError> {
        let module_resources = self.module_resources.as_ref().ok_or_else(|| {
            CompilerError::compiler_error(
                "HIR lowering reached a structural string piece without the issuing module resource table.",
            )
        })?;

        Ok(module_resources
            .borrow_mut()
            .intern_origin(origin.clone(), location.clone()))
    }

    /// Runs a lowering closure with `active_value_block_target` set to `target`.
    ///
    /// WHAT: scoped installation of the target consumed by `ThenValue` statements inside the
    /// closure.
    /// WHY: value-if, value-match, and catch recovery all use the same target protocol. Keeping
    /// the save/restore path here prevents leaked target state when nested lowering or early
    /// errors occur.
    pub(in crate::compiler_frontend::hir) fn with_active_value_block_target<T>(
        &mut self,
        target: ValueBlockTarget,
        emit: impl FnOnce(&mut HirBuilder<'_>) -> Result<T, CompilerError>,
    ) -> Result<T, CompilerError> {
        let previous_target = self.active_value_block_target.replace(target);

        let result = emit(self);

        self.active_value_block_target = previous_target;

        result
    }

    // -------------------------
    //  Main Build Pipeline
    // -------------------------

    /// Builds an HIR module from an AST.
    /// This is the main entry point for HIR generation.
    pub fn build_hir_module(mut self, mut ast: Ast) -> Result<HirLoweringResult, CompilerMessages> {
        self.module_const_values = std::mem::take(&mut ast.const_values);

        // Keep the AST warnings privately for error-context rendering only. They are not exposed
        // on the successful lowering result; frontend orchestration owns the merged warning vector.
        self.ast_warnings = ast.warnings.to_owned();
        self.module.const_facts = HirConstFacts::from(&ast.const_facts);
        self.imported_functions_by_name = ast.imported_functions_by_local_path.clone();
        self.imported_fallible_carriers_by_origin = self
            .imported_functions_by_name
            .values()
            .filter_map(|contract| match (&contract.target, contract.fallible_carrier_type_id) {
                (
                    crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::Imported {
                        origin,
                        ..
                    },
                    Some(carrier_type_id),
                ) => Some((origin.clone(), carrier_type_id)),
                _ => None,
            })
            .collect();
        self.generated_fallible_carriers_by_identity = self
            .imported_functions_by_name
            .values()
            .filter_map(|contract| match (&contract.target, contract.fallible_carrier_type_id) {
                (
                    crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::Generated {
                        identity,
                        ..
                    },
                    Some(carrier_type_id),
                ) => Some((identity.clone(), carrier_type_id)),
                _ => None,
            })
            .collect();
        self.module_private_fallible_carriers_by_identity = self
            .imported_functions_by_name
            .values()
            .filter_map(|contract| match (&contract.target, contract.fallible_carrier_type_id) {
                (
                    crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::ModulePrivate {
                        identity,
                        ..
                    },
                    Some(carrier_type_id),
                ) => Some((identity.clone(), carrier_type_id)),
                _ => None,
            })
            .collect();
        self.module.imported_call_summaries = self
            .imported_functions_by_name
            .values()
            .filter_map(|contract| match &contract.target {
                crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::Imported { origin, .. } => {
                    Some((origin.clone(), contract.summary.clone()))
                }
                crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::Local(_)
                | crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::Generated { .. }
                | crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::ModulePrivate { .. } => None,
            })
            .collect();
        self.module.module_private_call_summaries = self
            .imported_functions_by_name
            .values()
            .filter_map(|contract| match &contract.target {
                crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::ModulePrivate {
                    identity,
                    ..
                } => Some((identity.clone(), contract.summary.clone())),
                _ => None,
            })
            .collect();
        self.module.generated_call_summaries = self
            .imported_functions_by_name
            .values()
            .filter_map(|contract| match &contract.target {
                crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget::Generated {
                    identity,
                    ..
                } => Some((identity.clone(), contract.summary.clone())),
                _ => None,
            })
            .collect();

        // 1. Prepare declarations (functions, structs, choices)
        if let Err(error) = self.prepare_hir_declarations(&ast) {
            return Err(self.lower_error_messages(error));
        }

        // 2. Lower module-level constants
        if let Err(error) = self.lower_module_constants() {
            return Err(self.lower_error_messages(error));
        }

        // 3. Resolve documentation fragments
        if let Err(error) = self.resolve_doc_fragments(&ast) {
            return Err(self.lower_error_messages(error));
        }

        // 4. Lower AST nodes to HIR expressions/statements
        for node in &ast.nodes {
            if let Err(error) = self.process_ast_node(node) {
                return Err(self.lower_error_messages(error));
            }
        }

        // 5. Assign semantic origins to functions
        if let Err(error) = self.assign_function_origins() {
            return Err(self.lower_error_messages(error));
        }

        let warnings = self.ast_warnings.clone();
        let string_table = &*self.string_table;
        self.module.side_table = self.side_table;

        // 6. Validate the final HIR module. HIR validation checks executable HIR only; non-HIR
        //    compiler metadata (documentation fragments) is validated separately at the module
        //    compilation boundary.
        if let Err(error) = validate_hir_module(&self.module, &self.type_environment) {
            return Err(
                CompilerMessages::from_error_with_warnings(error, warnings, string_table)
                    .with_type_context_for_all_diagnostics(self.type_environment.clone()),
            );
        }

        record_hir_counters(&self.module);

        Ok(HirLoweringResult {
            hir_module: self.module,
            type_environment: self.type_environment,
            metadata: self.extracted_metadata,
        })
    }

    /// Processes a single AST node and generates corresponding HIR.
    fn process_ast_node(&mut self, node: &AstNode) -> Result<(), CompilerError> {
        self.lower_top_level_node(node)
    }

    // -------------------------
    //  ID Allocation
    // -------------------------

    allocate_id!(allocate_block_id, next_block_id, BlockId);
    allocate_id!(allocate_function_id, next_function_id, FunctionId);
    allocate_id!(allocate_region_id, next_region_id, RegionId);
    allocate_id!(allocate_local_id, next_local_id, LocalId);
    allocate_id!(allocate_node_id, next_node_id, HirNodeId);
    allocate_id!(allocate_value_id, next_value_id, HirValueId);
    allocate_id!(allocate_struct_id, next_struct_id, StructId);
    allocate_id!(allocate_field_id, next_field_id, FieldId);
    allocate_id!(allocate_const_id, next_const_id, HirConstId);
    allocate_id!(allocate_choice_id, next_choice_id, ChoiceId);

    // -------------------------
    //  Module Assembly
    // -------------------------

    pub(super) fn push_region(&mut self, region: HirRegion) {
        let index = self.module.regions.len();
        self.region_index_by_id.insert(region.id(), index);
        self.module.regions.push(region);
    }

    pub(super) fn push_block(&mut self, block: HirBlock) {
        let index = self.module.blocks.len();
        self.block_index_by_id.insert(block.id, index);
        self.module.blocks.push(block);
    }

    pub(super) fn push_function(&mut self, function: HirFunction) {
        let index = self.module.functions.len();
        self.function_index_by_id.insert(function.id, index);
        self.module.functions.push(function);
    }

    pub(super) fn push_struct(
        &mut self,
        hir_struct: crate::compiler_frontend::hir::structs::HirStruct,
    ) {
        let struct_index = self.module.structs.len();
        self.struct_index_by_id.insert(hir_struct.id, struct_index);

        for (field_index, field) in hir_struct.fields.iter().enumerate() {
            self.field_index_by_id
                .insert(field.id, (struct_index, field_index));
        }

        self.module.structs.push(hir_struct);
    }

    pub(super) fn register_local_in_block(
        &mut self,
        block_id: BlockId,
        local: crate::compiler_frontend::hir::blocks::HirLocal,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let block_index = self.block_index_or_error(block_id, location)?;
        let local_index = self.module.blocks[block_index].locals.len();
        self.local_index_by_id
            .insert(local.id, (block_index, local_index));
        self.module.blocks[block_index].locals.push(local);
        Ok(())
    }

    // -------------------------
    //  Resolution & Queries
    // -------------------------

    pub(super) fn local_type_id_or_error(
        &self,
        local_id: LocalId,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        let Some((block_index, local_index)) = self.local_index_by_id.get(&local_id).copied()
        else {
            return_hir_transformation_error!(
                format!("Local {:?} is not registered in HIR blocks", local_id),
                location.clone()
            );
        };

        Ok(self.module.blocks[block_index].locals[local_index].ty)
    }

    pub(super) fn field_type_id_or_error(
        &self,
        field_id: FieldId,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        let Some((struct_index, field_index)) = self.field_index_by_id.get(&field_id).copied()
        else {
            return_hir_transformation_error!(
                format!("Field {:?} is not registered in HIR structs", field_id),
                location.clone()
            );
        };

        Ok(self.module.structs[struct_index].fields[field_index].ty)
    }

    pub(super) fn block_index_or_error(
        &self,
        block_id: BlockId,
        location: &SourceLocation,
    ) -> Result<usize, CompilerError> {
        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return_hir_transformation_error!(
                format!("Block {:?} is not registered in HIR module", block_id),
                location.clone()
            );
        };

        Ok(index)
    }

    pub(super) fn function_index_or_error(
        &self,
        function_id: FunctionId,
        location: &SourceLocation,
    ) -> Result<usize, CompilerError> {
        let Some(index) = self.function_index_by_id.get(&function_id).copied() else {
            return_hir_transformation_error!(
                format!("Function {:?} is not registered in HIR module", function_id),
                location.clone()
            );
        };

        Ok(index)
    }

    pub(super) fn block_by_id_or_error(
        &self,
        block_id: BlockId,
        location: &SourceLocation,
    ) -> Result<&HirBlock, CompilerError> {
        let index = self.block_index_or_error(block_id, location)?;
        Ok(&self.module.blocks[index])
    }

    pub(super) fn block_mut_by_id_or_error(
        &mut self,
        block_id: BlockId,
        location: &SourceLocation,
    ) -> Result<&mut HirBlock, CompilerError> {
        let index = self.block_index_or_error(block_id, location)?;
        Ok(&mut self.module.blocks[index])
    }

    pub(super) fn function_by_id_or_error(
        &self,
        function_id: FunctionId,
        location: &SourceLocation,
    ) -> Result<&HirFunction, CompilerError> {
        let index = self.function_index_or_error(function_id, location)?;
        Ok(&self.module.functions[index])
    }

    pub(super) fn function_mut_by_id_or_error(
        &mut self,
        function_id: FunctionId,
        location: &SourceLocation,
    ) -> Result<&mut HirFunction, CompilerError> {
        let index = self.function_index_or_error(function_id, location)?;
        Ok(&mut self.module.functions[index])
    }

    // -------------------------
    //  State Management
    // -------------------------

    pub(crate) fn enter_function(
        &mut self,
        function_id: FunctionId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let entry_block = self.function_by_id_or_error(function_id, location)?.entry;

        self.current_function = Some(function_id);
        self.locals_by_name.clear();
        self.loop_targets.clear();
        self.set_current_block(entry_block, location)
    }

    pub(crate) fn leave_function(&mut self) {
        self.current_function = None;
        self.current_block = None;
        self.current_region = None;
        self.locals_by_name.clear();
        self.loop_targets.clear();
        self.entry_fragment_vec_local = None;
    }

    pub(super) fn with_temporary_local_bindings<T>(
        &mut self,
        bindings: impl IntoIterator<Item = (InternedPath, LocalId)>,
        f: impl FnOnce(&mut Self) -> Result<T, CompilerError>,
    ) -> Result<T, CompilerError> {
        let mut previous_bindings = Vec::new();
        for (path, local_id) in bindings {
            let previous = self.locals_by_name.insert(path.clone(), local_id);
            previous_bindings.push((path, previous));
        }

        let result = f(self);

        for (path, previous) in previous_bindings.into_iter().rev() {
            self.locals_by_name.remove(&path);
            if let Some(local_id) = previous {
                self.locals_by_name.insert(path, local_id);
            }
        }

        result
    }

    pub(crate) fn set_current_block(
        &mut self,
        block_id: BlockId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let region = self.block_by_id_or_error(block_id, location)?.region;
        self.current_block = Some(block_id);
        self.current_region = Some(region);
        Ok(())
    }

    pub(crate) fn current_block_id_or_error(
        &self,
        location: &SourceLocation,
    ) -> Result<BlockId, CompilerError> {
        let Some(block_id) = self.current_block else {
            return_hir_transformation_error!("No current HIR block is active", location.clone());
        };

        Ok(block_id)
    }

    pub(crate) fn current_function_id_or_error(
        &self,
        location: &SourceLocation,
    ) -> Result<FunctionId, CompilerError> {
        let Some(function_id) = self.current_function else {
            return_hir_transformation_error!(
                "No current HIR function is active",
                self.hir_error_location(location)
            );
        };

        Ok(function_id)
    }

    pub(crate) fn current_region_or_error(
        &self,
        location: &SourceLocation,
    ) -> Result<RegionId, CompilerError> {
        let Some(region) = self.current_region else {
            return_hir_transformation_error!(
                "No current HIR region is active",
                self.hir_error_location(location)
            );
        };

        Ok(region)
    }

    // -------------------------
    //  Terminator Management
    // -------------------------

    pub(crate) fn set_block_terminator(
        &mut self,
        block_id: BlockId,
        terminator: HirTerminator,
        source_location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        {
            let block = self.block_mut_by_id_or_error(block_id, source_location)?;
            if !Self::is_placeholder_terminator(&block.terminator) {
                return_hir_transformation_error!(
                    format!("Block {} already has an explicit terminator", block_id),
                    source_location.clone()
                );
            }

            block.terminator = terminator;
        }

        self.side_table.map_terminator(source_location, block_id);
        Ok(())
    }

    pub(crate) fn block_has_explicit_terminator(
        &self,
        block_id: BlockId,
        location: &SourceLocation,
    ) -> Result<bool, CompilerError> {
        let block = self.block_by_id_or_error(block_id, location)?;
        Ok(!Self::is_placeholder_terminator(&block.terminator))
    }

    fn is_placeholder_terminator(terminator: &HirTerminator) -> bool {
        matches!(terminator, HirTerminator::Uninitialized)
    }

    // -------------------------
    //  Diagnostics Support
    // -------------------------

    pub(super) fn symbol_name_for_diagnostics(&self, symbol: &InternedPath) -> String {
        symbol
            .name_str(self.string_table)
            .map(str::to_owned)
            .unwrap_or_else(|| symbol.to_string(self.string_table))
    }
}

fn record_hir_counters(module: &HirModule) {
    add_frontend_counter(FrontendCounter::HirBlockCount, module.blocks.len());
    add_frontend_counter(FrontendCounter::HirFunctionCount, module.functions.len());

    let statement_count = module
        .blocks
        .iter()
        .map(|block| block.statements.len())
        .sum();
    add_frontend_counter(FrontendCounter::HirStatementCount, statement_count);
}
