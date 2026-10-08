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
//! HIR transformation or lowering invariant failure only. The typed construction lane also
//! carries source diagnostics when authored input exceeds compact HIR store capacity; other
//! source failures are emitted by AST or earlier stages.

use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::const_facts::HirConstFacts;
#[cfg(test)]
use crate::compiler_frontend::hir::expression_store::HirExpressionStoreTestLimits;
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirExpressionStore};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOriginLookup};
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::{
    BlockId, ChoiceId, FieldId, FunctionId, HirConstId, HirNodeId, LocalId, RegionId, StructId,
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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::return_hir_transformation_error;
use rustc_hash::FxHashMap;
use std::{cell::RefCell, rc::Rc};

mod metadata;

// -----------
// Entry Point
// -----------
pub(in crate::compiler_frontend) fn lower_module(
    ast: Ast,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    function_origin_lookup: HirFunctionOriginLookup,
    module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    capacity_estimate: FrontendArenaCapacityEstimate,
) -> Result<HirLoweringResult, CompilerMessages> {
    let type_environment = ast.type_environment.clone();
    let mut ctx = HirBuilder::new(
        string_table,
        path_fork,
        type_environment,
        function_origin_lookup,
        capacity_estimate,
    );

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

    // === For variable name resolution ===
    pub(super) string_table: &'a mut StringTable,
    /// Module-local path identity table used to resolve all `PathId` names.
    pub(super) path_fork: &'a mut PathInternerFork,
    // === ID Counters ===
    next_block_id: u32,
    next_local_id: u32,
    next_node_id: u32,
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
    // AST guarantees module-wide unique PathId symbol IDs. HIR keys symbol resolution
    // by full paths, never by scope-local leaf strings.
    pub(super) locals_by_name: FxHashMap<PathId, LocalId>,
    pub(super) functions_by_name: FxHashMap<PathId, FunctionId>,
    pub(super) imported_functions_by_name: FxHashMap<PathId, AstImportedFunctionContract>,
    pub(super) imported_fallible_carriers_by_origin: FxHashMap<OriginFunctionId, TypeId>,
    pub(super) module_private_fallible_carriers_by_identity:
        FxHashMap<ModulePrivateExecutableIdentity, TypeId>,
    pub(super) generated_fallible_carriers_by_identity:
        FxHashMap<GeneratedFunctionIdentity, TypeId>,
    pub(super) structs_by_name: FxHashMap<PathId, StructId>,
    pub(super) choices_by_name: FxHashMap<PathId, ChoiceId>,
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
    pub(super) fields_by_struct_and_name: FxHashMap<(StructId, PathId), FieldId>,
    pub(super) module_constants_by_name: FxHashMap<PathId, ConstValueId>,
    pub(super) module_const_values: ConstValueStore,

    // === Fast ID -> arena index maps ===
    pub(super) block_index_by_id: FxHashMap<BlockId, usize>,
    pub(super) function_index_by_id: FxHashMap<FunctionId, usize>,
    pub(super) region_index_by_id: FxHashMap<RegionId, usize>,
    pub(super) local_index_by_id: FxHashMap<LocalId, (usize, usize)>,

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

    /// Expression-local continuation, installed only while protected work is lowered.
    pub(super) active_catch_handler: Option<CatchHandlerTarget>,

    /// Calls emitted while a catch handler protected the surrounding work.
    ///
    /// WHAT: pending records transferred into the module before compaction.
    /// WHY: the lane installer needs the routing captured at lowering time.
    catch_protected_calls: Vec<CatchProtectedCall>,
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct CatchHandlerTarget {
    pub block: BlockId,
    pub error_local: LocalId,
    pub error_type: TypeId,
}

/// One scalar `Call` statement whose failure edge an active catch owns.
///
/// WHAT: binds the emitted statement id to the handler that protected it and the function
///       under lowering that owns both.
/// WHY: the private-lane installer runs after summary convergence, so lowering can only record
///      where an eligible call's failure edge must go. The installer decides whether the callee
///      actually escapes, so infallible callees record optimistically and stay invisible.
///      Validation treats the recorded handler block as a root of the recorded owner until the
///      installer drains the record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CatchProtectedCall {
    pub statement: HirNodeId,
    pub handler: CatchHandlerTarget,
    pub owner: FunctionId,
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
        path_fork: &'a mut PathInternerFork,
        type_environment: TypeEnvironment,
        function_origin_lookup: HirFunctionOriginLookup,
        capacity_estimate: FrontendArenaCapacityEstimate,
    ) -> HirBuilder<'a> {
        let mut module = HirModule::new();
        module.expressions = HirExpressionStore::with_capacity(
            capacity_estimate.hir_expressions,
            capacity_estimate.expression_items,
        );
        HirBuilder {
            module,

            extracted_metadata: HirLoweringMetadata::default(),

            string_table,
            path_fork,
            type_environment,
            module_resources: None,
            function_origin_lookup,
            next_block_id: 0,
            next_local_id: 0,
            next_node_id: 0,
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

            current_function: None,
            current_block: None,
            current_region: None,
            loop_targets: vec![],
            entry_fragment_vec_local: None,
            active_value_block_target: None,
            active_catch_handler: None,
            catch_protected_calls: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_expression_store_test_limits(
        &mut self,
        limits: HirExpressionStoreTestLimits,
    ) {
        self.module.expressions = HirExpressionStore::with_test_limits(limits);
    }

    fn lower_error_messages(&self, error: impl Into<HirConstructionFailure>) -> CompilerMessages {
        let messages = match error.into() {
            HirConstructionFailure::Diagnosed(diagnostic) => {
                CompilerMessages::from_diagnostic_ref(diagnostic, self.string_table)
            }
            HirConstructionFailure::Infrastructure(error) => {
                CompilerMessages::from_error_ref(error, self.string_table)
            }
        };
        messages.with_type_context_for_all_diagnostics(self.type_environment.clone())
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
        location: &Option<SourceSpan>,
    ) -> Result<ResourceId, CompilerError> {
        let module_resources = self.module_resources.as_ref().ok_or_else(|| {
            CompilerError::compiler_error(
                "HIR lowering reached a structural string piece without the issuing module resource table.",
            )
        })?;

        Ok(module_resources
            .borrow_mut()
            .intern_origin(origin.clone(), *location))
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
        emit: impl FnOnce(&mut HirBuilder<'_>) -> Result<T, HirConstructionFailure>,
    ) -> Result<T, HirConstructionFailure> {
        let previous_target = self.active_value_block_target.replace(target);

        let result = emit(self);

        self.active_value_block_target = previous_target;

        result
    }

    pub(super) fn with_active_catch_handler<T>(
        &mut self,
        target: CatchHandlerTarget,
        emit: impl FnOnce(&mut HirBuilder<'_>) -> Result<T, HirConstructionFailure>,
    ) -> Result<T, HirConstructionFailure> {
        let previous_handler = self.active_catch_handler.replace(target);
        let result = emit(self);
        self.active_catch_handler = previous_handler;
        result
    }

    // -------------------------
    //  Main Build Pipeline
    // -------------------------

    /// Builds an HIR module from an AST.
    /// This is the main entry point for HIR generation.
    pub fn build_hir_module(mut self, mut ast: Ast) -> Result<HirLoweringResult, CompilerMessages> {
        self.module_const_values = std::mem::take(&mut ast.const_values);

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

        // 5. Project semantic failure facts and join exact function origins.
        if let Err(error) = self.project_function_failure_facts(&ast) {
            return Err(self.lower_error_messages(error));
        }

        if let Err(error) = self.assign_function_origins() {
            return Err(self.lower_error_messages(error));
        }

        let string_table = &*self.string_table;
        self.module.side_table = self.side_table;
        // Scalar calls recorded under catch handlers keep their handler routes for the lane
        // installer; block ids are remapped like every other terminator target.
        self.module.catch_protected_calls = std::mem::take(&mut self.catch_protected_calls);
        // Construction may remove unused scaffolding blocks. Later passes index blocks by id.
        self.module.compact_block_ids();

        // 6. Validate the final HIR module. HIR validation checks executable HIR only; non-HIR
        //    compiler metadata (documentation fragments) is validated separately at the module
        //    compilation boundary.
        if let Err(error) = validate_hir_module(&self.module, &self.type_environment) {
            return Err(CompilerMessages::from_error_ref(error, string_table)
                .with_type_context_for_all_diagnostics(self.type_environment.clone()));
        }

        record_hir_counters(&self.module);

        Ok(HirLoweringResult {
            hir_module: self.module,
            type_environment: self.type_environment,
            metadata: self.extracted_metadata,
        })
    }

    /// Processes a single AST node and generates corresponding HIR.
    fn process_ast_node(&mut self, node: &AstNode) -> Result<(), HirConstructionFailure> {
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

    /// Records one scalar call emitted while a catch handler protected the work.
    ///
    /// WHAT: binds the statement to the active handler and the function under lowering.
    ///       Infallible callees record optimistically; the lane installer skips records whose
    ///       callee never escapes and prunes handlers no error edge reaches.
    /// WHY: the installer runs after summary convergence and cannot re-derive protection from
    ///      the HIR alone. Validation treats recorded handler blocks as roots of the recorded
    ///      owner until the installer drains the records.
    pub(super) fn record_catch_protected_call(&mut self, statement: HirNodeId) {
        if let (Some(handler), Some(owner)) = (self.active_catch_handler, self.current_function) {
            self.catch_protected_calls.push(CatchProtectedCall {
                statement,
                handler,
                owner,
            });
        }
    }

    pub(super) fn push_struct(
        &mut self,
        hir_struct: crate::compiler_frontend::hir::structs::HirStruct,
    ) {
        self.module.structs.push(hir_struct);
    }

    pub(super) fn register_local_in_block(
        &mut self,
        block_id: BlockId,
        local: crate::compiler_frontend::hir::blocks::HirLocal,
        location: &Option<SourceSpan>,
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
        location: &Option<SourceSpan>,
    ) -> Result<TypeId, CompilerError> {
        let Some((block_index, local_index)) = self.local_index_by_id.get(&local_id).copied()
        else {
            return_hir_transformation_error!(
                format!("Local {:?} is not registered in HIR blocks", local_id),
                *location
            );
        };

        Ok(self.module.blocks[block_index].locals[local_index].ty)
    }

    pub(super) fn block_index_or_error(
        &self,
        block_id: BlockId,
        location: &Option<SourceSpan>,
    ) -> Result<usize, CompilerError> {
        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return_hir_transformation_error!(
                format!("Block {:?} is not registered in HIR module", block_id),
                *location
            );
        };

        Ok(index)
    }

    pub(super) fn function_index_or_error(
        &self,
        function_id: FunctionId,
        location: &Option<SourceSpan>,
    ) -> Result<usize, CompilerError> {
        let Some(index) = self.function_index_by_id.get(&function_id).copied() else {
            return_hir_transformation_error!(
                format!("Function {:?} is not registered in HIR module", function_id),
                *location
            );
        };

        Ok(index)
    }

    pub(super) fn block_by_id_or_error(
        &self,
        block_id: BlockId,
        location: &Option<SourceSpan>,
    ) -> Result<&HirBlock, CompilerError> {
        let index = self.block_index_or_error(block_id, location)?;
        Ok(&self.module.blocks[index])
    }

    pub(super) fn block_mut_by_id_or_error(
        &mut self,
        block_id: BlockId,
        location: &Option<SourceSpan>,
    ) -> Result<&mut HirBlock, CompilerError> {
        let index = self.block_index_or_error(block_id, location)?;
        Ok(&mut self.module.blocks[index])
    }

    pub(super) fn function_by_id_or_error(
        &self,
        function_id: FunctionId,
        location: &Option<SourceSpan>,
    ) -> Result<&HirFunction, CompilerError> {
        let index = self.function_index_or_error(function_id, location)?;
        Ok(&self.module.functions[index])
    }

    pub(super) fn function_mut_by_id_or_error(
        &mut self,
        function_id: FunctionId,
        location: &Option<SourceSpan>,
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
        location: &Option<SourceSpan>,
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
        bindings: impl IntoIterator<Item = (PathId, LocalId)>,
        f: impl FnOnce(&mut Self) -> Result<T, HirConstructionFailure>,
    ) -> Result<T, HirConstructionFailure> {
        let mut previous_bindings = Vec::new();
        for (path, local_id) in bindings {
            let previous = self.locals_by_name.insert(path, local_id);
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
        location: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let region = self.block_by_id_or_error(block_id, location)?.region;
        self.current_block = Some(block_id);
        self.current_region = Some(region);
        Ok(())
    }

    pub(crate) fn current_block_id_or_error(
        &self,
        location: &Option<SourceSpan>,
    ) -> Result<BlockId, CompilerError> {
        let Some(block_id) = self.current_block else {
            return_hir_transformation_error!("No current HIR block is active", *location);
        };

        Ok(block_id)
    }

    pub(crate) fn current_function_id_or_error(
        &self,
        location: &Option<SourceSpan>,
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
        location: &Option<SourceSpan>,
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
        source_location: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        {
            let block = self.block_mut_by_id_or_error(block_id, source_location)?;
            if !Self::is_placeholder_terminator(&block.terminator) {
                return_hir_transformation_error!(
                    format!("Block {} already has an explicit terminator", block_id),
                    *source_location
                );
            }

            block.terminator = terminator;
        }

        self.side_table.map_terminator(*source_location, block_id);
        Ok(())
    }

    pub(crate) fn block_has_explicit_terminator(
        &self,
        block_id: BlockId,
        location: &Option<SourceSpan>,
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
    pub(super) fn symbol_name_for_diagnostics(&self, symbol: &PathId) -> String {
        self.path_fork
            .component(*symbol)
            .map(|component| self.string_table.resolve(component).to_owned())
            .unwrap_or_else(|| format!("{symbol:?}"))
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
