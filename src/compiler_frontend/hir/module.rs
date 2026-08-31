//! HIR module container.
//!
//! WHAT: the executable/semantic IR payload produced for one Moth module.
//! WHY: backends consume `HirModule` as the stable frontend output after AST lowering and borrow
//! validation.
//!
//! Type identity lives in the frontend `TypeEnvironment` carried beside the module at the
//! compiled-module boundary. HIR nodes store compact frontend `TypeId`s and do not own a separate
//! semantic type table.
//!
//! Non-HIR compiler metadata — warnings and resolved documentation fragments — is extracted by
//! HIR lowering into `HirLoweringMetadata` and assembled into `ModuleCompilerMetadata` on the
//! build-system module payload. `HirModule` carries only executable/semantic HIR state.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::const_facts::HirConstFacts;
use crate::compiler_frontend::hir::constants::HirModuleConst;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::structs::HirStruct;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use rustc_hash::FxHashMap;

// -------------------------
//  Choice Layout Metadata
// -------------------------

/// Lowering-local choice layout entry.
///
/// WHY: HIR expressions and backends reference variants by stable `ChoiceId` and flat
/// variant/field indices. Semantic type identity (variant names, payload types) lives in the
/// frontend `TypeEnvironment`; `frontend_type_id` traces this entry back to the canonical type.
#[derive(Debug, Clone)]
pub struct HirChoice {
    /// The stable `ChoiceId` assigned during HIR lowering.
    /// WHY: preserved so diagnostics and backend validation can trust the choice
    ///      layout entry's own identity while walking HIR choices.
    pub id: crate::compiler_frontend::hir::ids::ChoiceId,

    /// Trace to the canonical frontend `TypeId` in `TypeEnvironment`.
    /// WHY: validation and backend checks use this to confirm HIR layout entries
    ///      still map back to canonical frontend type identity.
    pub frontend_type_id: TypeId,

    pub variants: Vec<HirChoiceVariant>,
}

#[derive(Debug, Clone)]
pub struct HirChoiceVariant {
    /// The interned variant name from the frontend `TypeEnvironment`.
    /// WHY: preserved for completeness and future diagnostic rendering.
    ///      Currently not read outside tests.
    #[allow(dead_code)]
    pub name: StringId,
    pub fields: Vec<HirChoiceField>,
}

#[derive(Debug, Clone)]
pub struct HirChoiceField {
    pub name: StringId,
    pub ty: TypeId,
}

// -------------------------
//  HIR Module Container
// -------------------------

#[derive(Debug, Clone)]
pub struct HirModule {
    pub blocks: Vec<HirBlock>,
    pub functions: Vec<HirFunction>,
    pub structs: Vec<HirStruct>,
    pub choices: Vec<HirChoice>,
    pub side_table: HirSideTable,

    /// Compiler-synthesised entry point for normal roots.
    pub start_function: Option<FunctionId>,
    /// Classification for every function in the module.
    ///
    /// WHY: backends/builders need explicit semantic role tagging to keep
    /// entry/runtime-template behavior stable across lowering passes.
    pub function_origins: FxHashMap<FunctionId, HirFunctionOrigin>,

    /// Stable origin-to-local-function lookup for directly exported non-generic functions and
    /// receiver methods that lower to local HIR.
    ///
    /// WHAT: records the explicit stable-origin-to-local-function relationship produced while
    /// lowering, excluding private functions and the implicit start.
    /// WHY: public-interface finalization joins borrow summaries to declaration records through
    /// this side table rather than rendered names, paths or declaration order.
    pub function_ids_by_origin: FxHashMap<OriginFunctionId, FunctionId>,
    pub(crate) function_ids_by_private_origin:
        FxHashMap<ModulePrivateExecutableIdentity, FunctionId>,
    pub(crate) function_ids_by_generated: FxHashMap<GeneratedFunctionIdentity, FunctionId>,
    pub(crate) imported_call_summaries: FxHashMap<OriginFunctionId, PublicCallSummary>,
    pub(crate) module_private_call_summaries:
        FxHashMap<ModulePrivateExecutableIdentity, PublicCallSummary>,
    pub(crate) generated_call_summaries: FxHashMap<GeneratedFunctionIdentity, PublicCallSummary>,

    pub module_constants: Vec<HirModuleConst>,

    /// Region tree
    pub regions: Vec<HirRegion>,

    /// Advisory const facts projected from the AST for future optimization.
    ///
    /// WHAT: records which declarations are compile-time constants, their scope,
    ///       source, value kind, and source location.
    /// WHY: provides metadata for later borrow-checker and lowering optimizations
    ///      without changing HIR semantics today.
    pub const_facts: HirConstFacts,

    /// Direct synthetic compile-time interface provenance for every local function.
    ///
    /// WHAT: one immutable read-only provenance fact per local `FunctionId`, including an explicit
    /// empty fact for functions with no synthetic-interface dependencies. The fact is the sorted,
    /// duplicate-free union of all expression provenance lowered from the function body. It is a
    /// read-only fact and does not alter HIR control flow.
    /// WHY: the per-function link-fact lane described in the compiler design overview needs stable,
    /// deterministic direct provenance. HIR validation rejects missing, extra or out-of-range
    /// coverage as `CompilerError`.
    pub function_provenance: FxHashMap<FunctionId, SyntheticInterfaceProvenance>,
}

impl HirModule {
    pub fn new() -> Self {
        Self {
            blocks: vec![],
            functions: vec![],
            structs: vec![],
            choices: vec![],
            side_table: HirSideTable::default(),
            start_function: None,
            function_origins: FxHashMap::default(),
            function_ids_by_origin: FxHashMap::default(),
            function_ids_by_private_origin: FxHashMap::default(),
            function_ids_by_generated: FxHashMap::default(),
            imported_call_summaries: FxHashMap::default(),
            module_private_call_summaries: FxHashMap::default(),
            generated_call_summaries: FxHashMap::default(),
            module_constants: vec![],
            regions: vec![],
            const_facts: HirConstFacts::default(),
            function_provenance: FxHashMap::default(),
        }
    }

    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for block in &mut self.blocks {
            block.remap_string_ids(remap);
        }

        for choice in &mut self.choices {
            for variant in &mut choice.variants {
                variant.name = remap.get(variant.name);
                for field in &mut variant.fields {
                    field.name = remap.get(field.name);
                }
            }
        }

        // Structural string constants carry interned `Text` handles alongside the
        // executable lane, so they re-bind on the same merge.
        for constant in &mut self.module_constants {
            constant.value.remap_string_ids(remap);
        }
        self.side_table.remap_string_ids(remap);
        self.const_facts.remap_string_ids(remap);
    }

    pub(crate) fn require_start_function(&self, owner: &str) -> Result<FunctionId, CompilerError> {
        self.start_function.ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "{owner} requires a normal module with an implicit start function"
            ))
        })
    }
}
