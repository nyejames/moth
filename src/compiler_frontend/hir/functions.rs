//! HIR function declarations.
//!
//! WHAT: function-level HIR metadata, including entry block, parameters, result channels and semantic
//! origin classification.
//! WHY: backends need to distinguish regular functions from the implicit entry `start` function.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId};
use crate::compiler_frontend::semantic_identity::{
    ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
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
    pub(crate) path: PathId,
    pub(crate) origin: OriginFunctionId,
}

/// Stable executable seed for a module-private function required by a generated sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PrivateFunctionOriginSeed {
    pub(crate) path: PathId,
    pub(crate) origin: ModulePrivateExecutableIdentity,
}

pub(crate) enum HirStableFunctionOrigin {
    Public(OriginFunctionId),
    ModulePrivate(ModulePrivateExecutableIdentity),
}

/// Provider-independent stable-origin lookup retained only during one HIR lowering.
///
/// WHAT: maps exact declaration paths to stable origins and tracks which seeds a lowered
/// function consumed. The lookup is dropped before the completed HIR artefact boundary.
/// WHY: the HIR lowering owner must reject unused concrete origin seeds at the lowering
/// boundary instead of deferring a silently unmatched seed to public-interface finalization.
#[derive(Clone, Debug, Default)]
pub(crate) struct HirFunctionOriginLookup {
    by_path: FxHashMap<PathId, OriginFunctionId>,
    private_by_path: FxHashMap<PathId, ModulePrivateExecutableIdentity>,
    consumed_paths: FxHashSet<PathId>,
}

impl HirFunctionOriginLookup {
    pub(crate) fn from_seeds(
        seeds: Vec<FunctionOriginSeed>,
    ) -> Result<Self, InfrastructureFailure> {
        let mut by_path = FxHashMap::default();
        let mut origins = FxHashSet::default();

        for seed in seeds {
            if origins.contains(&seed.origin) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    format!(
                        "HIR function-origin lowering received duplicate stable origin {:?}",
                        seed.origin
                    ),
                ));
            }
            if by_path.contains_key(&seed.path) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "HIR function-origin lowering received duplicate declaration paths",
                ));
            }

            origins.insert(seed.origin.clone());
            by_path.insert(seed.path, seed.origin);
        }

        Ok(Self {
            by_path,
            private_by_path: FxHashMap::default(),
            consumed_paths: FxHashSet::default(),
        })
    }

    pub(crate) fn from_public_and_private_seeds(
        public_seeds: Vec<FunctionOriginSeed>,
        private_seeds: Vec<PrivateFunctionOriginSeed>,
    ) -> Result<Self, InfrastructureFailure> {
        let mut lookup = Self::from_seeds(public_seeds)?;
        let mut private_origins = FxHashSet::default();
        for seed in private_seeds {
            if lookup.by_path.contains_key(&seed.path)
                || lookup.private_by_path.contains_key(&seed.path)
            {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "HIR function-origin lowering received duplicate declaration paths",
                ));
            }
            if !private_origins.insert(seed.origin.clone()) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    format!(
                        "HIR function-origin lowering received duplicate stable origin {:?}",
                        seed.origin
                    ),
                ));
            }
            lookup.private_by_path.insert(seed.path, seed.origin);
        }
        Ok(lookup)
    }

    /// Consume the stable origin seed for one declaration path, marking it matched.
    ///
    /// WHAT: returns the stable origin for `path` and records that a lowered function consumed
    /// the seed. Returns `None` when no seed exists for the path (private functions and the
    /// entry).
    /// WHY: tracking consumption lets the lowering boundary detect unused seeds without keeping a
    /// second parallel origin set.
    pub(crate) fn consume_origin_for(&mut self, path: &PathId) -> Option<HirStableFunctionOrigin> {
        if let Some(origin) = self.by_path.get(path) {
            self.consumed_paths.insert(*path);
            return Some(HirStableFunctionOrigin::Public(origin.clone()));
        }
        let origin = self.private_by_path.get(path)?;
        self.consumed_paths.insert(*path);
        Some(HirStableFunctionOrigin::ModulePrivate(origin.clone()))
    }

    /// Reject any concrete origin seed that no lowered function consumed.
    ///
    /// WHAT: errors when a seed path never matched a lowered function. Call this after every
    /// function has been classified.
    /// WHY: an unmatched seed means a public callable declaration did not lower to local HIR,
    /// which is an internal compiler invariant failure rather than a silent finalization deferral.
    pub(crate) fn validate_all_seeds_consumed(&self) -> Result<(), InfrastructureFailure> {
        let unused_seed_count = self
            .by_path
            .keys()
            .chain(self.private_by_path.keys())
            .filter(|path| !self.consumed_paths.contains(*path))
            .count();
        if unused_seed_count > 0 {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                format!(
                    "HIR function-origin lowering received {unused_seed_count} unused concrete origin seed(s)"
                ),
            ));
        }

        Ok(())
    }
}

/// Whole HIR callable return contract.
///
/// WHAT: states whether the callable completes normally with ordinary success/error slots or
///       never returns to its caller.
/// WHY: `Never` is not an empty ordinary return shape. Keeping the variant explicit stops
///      lowering, validation and backends from treating a non-returning function as a function
///      with zero success slots, and keeps the private failure lane from installing an error
///      channel on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HirFunctionReturnContract {
    Returns(CallableReturnShape),
    Never,
}

impl HirFunctionReturnContract {
    /// Wrap one ordinary success-slot and error-channel shape.
    pub(crate) fn returns(shape: CallableReturnShape) -> Self {
        Self::Returns(shape)
    }

    /// Whether the function never returns to its caller.
    pub fn is_never(&self) -> bool {
        matches!(self, Self::Never)
    }

    /// The ordinary shape, or `None` when the function declares Never.
    ///
    /// WHAT: never maps Never onto empty slots.
    /// WHY: callers that own a success or error channel must decide explicitly what a Never
    ///      contract means at their boundary.
    pub fn ordinary(&self) -> Option<&CallableReturnShape> {
        match self {
            Self::Returns(shape) => Some(shape),
            Self::Never => None,
        }
    }

    /// Mutable access to the ordinary shape, or `None` when the function declares Never.
    pub(crate) fn ordinary_mut(&mut self) -> Option<&mut CallableReturnShape> {
        match self {
            Self::Returns(shape) => Some(shape),
            Self::Never => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HirFunction {
    pub id: FunctionId,
    pub entry: BlockId,
    /// Locals defined by entering this function under its call ABI contract.
    pub params: Vec<LocalId>,
    pub return_contract: HirFunctionReturnContract,
}
