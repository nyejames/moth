//! Always-on HIR invariant validation.
//!
//! WHAT: checks freshly lowered HIR for definition integrity, frontend type
//! links, CFG ownership, metadata mappings, and expression/place consistency.
//! WHY: borrow validation and backend lowering should consume a coherent IR
//! contract and should not need defensive checks for builder bugs.
//!
//! This module reports `HirTransformation` infrastructure errors only. Normal
//! user-facing source diagnostics must be produced before HIR validation runs.
mod blocks;
mod definitions;
mod expressions;
mod functions;
mod metadata;
mod structure;
mod support;

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, HirNodeId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::HirModule;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;

pub(crate) fn validate_hir_module(
    module: &HirModule,
    type_environment: &TypeEnvironment,
) -> Result<(), CompilerError> {
    let mut validator = HirValidator::new(module, type_environment);
    validator.validate()
}

struct HirValidator<'a> {
    module: &'a HirModule,
    type_environment: &'a TypeEnvironment,

    block_ids: FxHashSet<BlockId>,
    block_index_by_id: FxHashMap<BlockId, usize>,
    block_owner_by_id: FxHashMap<BlockId, FunctionId>,
    statement_block_by_id: FxHashMap<HirNodeId, BlockId>,
    function_ids: FxHashSet<FunctionId>,
    struct_types: FxHashMap<StructId, TypeId>,
    field_ids: FxHashSet<FieldId>,
    region_ids: FxHashSet<RegionId>,
    parameter_local_ids: FxHashSet<LocalId>,

    local_types: FxHashMap<LocalId, TypeId>,
    local_block_by_id: FxHashMap<LocalId, BlockId>,
    field_types: FxHashMap<FieldId, TypeId>,
    field_owner: FxHashMap<FieldId, StructId>,
    active_expression_rows: Vec<Cell<bool>>,
}

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Constructor
    // -------------------------

    fn new(module: &'a HirModule, type_environment: &'a TypeEnvironment) -> Self {
        let expression_count = module.expressions.expression_count();
        Self {
            module,
            type_environment,
            block_ids: FxHashSet::default(),
            block_index_by_id: FxHashMap::default(),
            block_owner_by_id: FxHashMap::default(),
            statement_block_by_id: FxHashMap::default(),
            function_ids: FxHashSet::default(),
            struct_types: FxHashMap::default(),
            field_ids: FxHashSet::default(),
            region_ids: FxHashSet::default(),
            parameter_local_ids: FxHashSet::default(),
            local_types: FxHashMap::default(),
            local_block_by_id: FxHashMap::default(),
            field_types: FxHashMap::default(),
            field_owner: FxHashMap::default(),
            active_expression_rows: vec![Cell::new(false); expression_count],
        }
    }

    // -------------------------
    //  Main Validation Pipeline
    // -------------------------

    fn validate(&mut self) -> Result<(), CompilerError> {
        // 1. Collect all definition IDs to build lookup tables.
        self.collect_definition_ids()?;

        // 2. Validate that HIR struct/choice frontend_type_ids trace to real
        // TypeEnvironment entries.
        self.validate_frontend_type_ids()?;

        // 3. Validate structural graphs and function-level metadata.
        self.validate_region_graph()?;
        self.validate_start_function()?;
        self.validate_function_origins()?;
        self.validate_function_provenance()?;
        self.validate_function_cfg_ownership()?;

        // 4. Validate module-level metadata.
        self.validate_module_constants()?;

        // 5. Validate core HIR entities.
        self.validate_functions()?;
        self.validate_blocks()?;

        Ok(())
    }
}
