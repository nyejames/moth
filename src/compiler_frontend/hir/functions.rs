//! HIR function declarations.
//!
//! WHAT: function-level HIR metadata, including entry block, parameters, return type, and semantic
//! origin classification.
//! WHY: backends need to distinguish regular functions from the implicit entry `start` function.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId};
use crate::compiler_frontend::semantic_identity::OriginFunctionId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HirFunctionOrigin {
    /// Regular user-declared function.
    Normal,
    /// Implicit start function for the module entry file.
    EntryStart,
}

/// Transient exact declaration-path seed for one HIR lowering.
///
/// WHAT: carries the retained declaration path and stable origin until HIR assigns a local
/// `FunctionId`. It belongs to the AST-to-HIR stage handoff, not stable semantic identity.
/// WHY: public function joins need exact declaration identity without rendering names or relying
/// on declaration order. The seed is consumed before the completed HIR artefact boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FunctionOriginSeed {
    pub(crate) path: InternedPath,
    pub(crate) origin: OriginFunctionId,
}

/// Provider-independent stable-origin lookup retained only during one HIR lowering.
///
/// WHAT: maps exact declaration paths to stable origins and tracks which seeds a lowered
/// function consumed. The lookup is dropped before the completed HIR artefact boundary.
/// WHY: the HIR lowering owner must reject unused concrete origin seeds at the lowering
/// boundary instead of deferring a silently unmatched seed to public-interface finalization.
#[derive(Clone, Debug, Default)]
pub(crate) struct HirFunctionOriginLookup {
    by_path: FxHashMap<InternedPath, OriginFunctionId>,
    consumed_paths: FxHashSet<InternedPath>,
}

impl HirFunctionOriginLookup {
    pub(crate) fn from_seeds(seeds: Vec<FunctionOriginSeed>) -> Result<Self, CompilerError> {
        let mut by_path = FxHashMap::default();
        let mut origins = FxHashSet::default();

        for seed in seeds {
            if origins.contains(&seed.origin) {
                return Err(CompilerError::compiler_error(format!(
                    "HIR function-origin lowering received duplicate stable origin {:?}",
                    seed.origin
                )));
            }
            if by_path.contains_key(&seed.path) {
                return Err(CompilerError::compiler_error(
                    "HIR function-origin lowering received duplicate declaration paths",
                ));
            }

            origins.insert(seed.origin.clone());
            by_path.insert(seed.path, seed.origin);
        }

        Ok(Self {
            by_path,
            consumed_paths: FxHashSet::default(),
        })
    }

    /// Consume the stable origin seed for one declaration path, marking it matched.
    ///
    /// WHAT: returns the stable origin for `path` and records that a lowered function consumed
    /// the seed. Returns `None` when no seed exists for the path (private functions and the
    /// implicit start).
    /// WHY: tracking consumption lets the lowering boundary detect unused seeds without keeping a
    /// second parallel origin set.
    pub(crate) fn consume_origin_for(&mut self, path: &InternedPath) -> Option<OriginFunctionId> {
        let origin = self.by_path.get(path)?;
        self.consumed_paths.insert(path.clone());
        Some(origin.clone())
    }

    /// Reject any concrete origin seed that no lowered function consumed.
    ///
    /// WHAT: errors when a seed path never matched a lowered function. Call this after every
    /// function has been classified.
    /// WHY: an unmatched seed means a public callable declaration did not lower to local HIR,
    /// which is an internal compiler invariant failure rather than a silent finalization deferral.
    pub(crate) fn validate_all_seeds_consumed(&self) -> Result<(), CompilerError> {
        let unused_seed_count = self
            .by_path
            .keys()
            .filter(|path| !self.consumed_paths.contains(*path))
            .count();
        if unused_seed_count > 0 {
            return Err(CompilerError::compiler_error(format!(
                "HIR function-origin lowering received {unused_seed_count} unused concrete origin seed(s)"
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct HirFunction {
    pub id: FunctionId,
    pub entry: BlockId,
    pub params: Vec<LocalId>,
    pub return_type: TypeId,
    pub return_aliases: Vec<Option<Vec<usize>>>,
}
