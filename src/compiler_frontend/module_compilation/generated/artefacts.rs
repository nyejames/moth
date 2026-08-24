//! Compiler-produced generated-function results.
//!
//! WHAT: the sidecar payload for one concrete generic executable, the completed record that pairs
//!       it with its exact borrow summary, its dense index and the delta of everything one
//!       successful module transaction produced.
//! WHY:  materialisation, HIR validation, borrow analysis and summary convergence are compiler
//!       semantics, so their results are compiler values. The build system aggregates,
//!       deduplicates, stores and publishes these records at its compilation boundary; it never
//!       produces or mutates their semantic content.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironmentRemapCache;
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

/// One independently lowered concrete generic executable.
///
/// The stable identity is stored beside, rather than rediscovered from, its HIR module. Base
/// canonical modules and generated sidecars therefore remain distinct project-compilation lanes.
pub(crate) struct GeneratedFunctionSidecar {
    pub(crate) identity: GeneratedFunctionIdentity,
    pub(crate) module: Module,
}

impl GeneratedFunctionSidecar {
    pub(crate) fn new(identity: GeneratedFunctionIdentity, module: Module) -> Self {
        Self { identity, module }
    }

    pub(crate) fn remap_string_ids_with_type_environment_cache(
        &mut self,
        remap: &StringIdRemap,
        type_environment_cache: &mut TypeEnvironmentRemapCache,
    ) {
        self.module
            .remap_string_ids_with_type_environment_cache(remap, type_environment_cache);
    }
}

/// Dense index of one completed generated function inside a boundary store or module transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GeneratedFunctionId(usize);

impl GeneratedFunctionId {
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }
}

/// One completed generated function with its exact identity, summary and sidecar.
///
/// WHAT: keeps the three facts one coherent record so a boundary can never align a summary with
///       the wrong sidecar or leave one of them orphaned.
/// WHY: generated summaries and sidecars are published transactionally; storing them as one row
///      removes separate publication paths and later reconstruction of the generated owner.
pub(crate) struct CompletedGeneratedFunction {
    pub(crate) identity: GeneratedFunctionIdentity,
    pub(crate) summary: PublicCallSummary,
    pub(crate) sidecar: GeneratedFunctionSidecar,
}

/// Every generated function one successful module compilation completed.
///
/// WHAT: the newly completed identities, exact summaries and sidecars produced inside one module
///       transaction, in deterministic completion order.
/// WHY: a diagnosed module publishes nothing, so the delta is the compiler's all-or-nothing
///      handoff. The build boundary remaps it with the rest of the module result and commits it
///      atomically beside the base artefact.
pub(crate) struct GeneratedFunctionDelta {
    records: Vec<CompletedGeneratedFunction>,
}

impl GeneratedFunctionDelta {
    pub(crate) fn from_records(records: Vec<CompletedGeneratedFunction>) -> Self {
        Self { records }
    }

    pub(crate) fn records(&self) -> &[CompletedGeneratedFunction] {
        &self.records
    }

    pub(crate) fn into_records(self) -> Vec<CompletedGeneratedFunction> {
        self.records
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        let mut type_environment_cache = TypeEnvironmentRemapCache::default();
        for record in &mut self.records {
            record
                .sidecar
                .remap_string_ids_with_type_environment_cache(remap, &mut type_environment_cache);
        }
    }
}

/// Validate one completed generated record as a publishable executable row.
///
/// WHAT: proves the sidecar HIR contains exactly one generated root mapping for this identity,
///       that the root `FunctionId` is in range, and that the record's summary is the exact borrow
///       summary of that generated root.
/// WHY: generated summaries and sidecars publish together, so a boundary store must never accept a
///      sidecar whose root identity or summary cannot back the record. The proof reads compiler
///      semantic state, so the compiler owns it and the build boundary calls it during preflight.
pub(crate) fn validate_completed_generated_record(
    record: &CompletedGeneratedFunction,
) -> Result<(), CompilerError> {
    let hir = &record.sidecar.module.executable.hir;
    let mut roots = hir.function_ids_by_generated.iter();
    let Some((root_identity, function_id)) = roots.next() else {
        return Err(CompilerError::compiler_error(format!(
            "Generated sidecar {:?} has no generated root executable identity",
            record.identity
        )));
    };
    if roots.next().is_some() {
        return Err(CompilerError::compiler_error(format!(
            "Generated sidecar {:?} presents more than one generated root identity",
            record.identity
        )));
    }
    if root_identity != &record.identity {
        return Err(CompilerError::compiler_error(format!(
            "Generated sidecar root identity {:?} disagrees with its record identity {:?}",
            root_identity, record.identity
        )));
    }
    if function_id.0 as usize >= hir.functions.len() {
        return Err(CompilerError::compiler_error(format!(
            "Generated sidecar {:?} references out-of-range HIR FunctionId {}",
            record.identity, function_id.0
        )));
    }

    let exact_summary = record
        .sidecar
        .module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .get(function_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated function {:?} has no exact borrow summary",
                record.identity
            ))
        })?;
    if exact_summary != &record.summary {
        return Err(CompilerError::compiler_error(format!(
            "Generated function {:?} summary disagrees with its sidecar borrow summary",
            record.identity
        )));
    }

    Ok(())
}
