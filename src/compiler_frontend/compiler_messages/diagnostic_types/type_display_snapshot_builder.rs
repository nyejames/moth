//! Builds a frozen type-display snapshot from a live `TypeEnvironment`.
//!
//! WHAT: encodes each requested `TypeId` as a snapshot record and returns its `TypeDisplayId`.
//! Arguments of functions, collections, maps, options and generic instances are encoded first.
//! WHY: diagnostics render type spellings after the live environment is gone. The snapshot stores
//! the same spelling facts the live formatter reads, so both render identically.
//! OWNERSHIP: the builder borrows the environment for the build. It owns the snapshot under
//! construction, memos from `TypeId` to `TypeDisplayId` for resolved types and unresolved roots,
//! and a scratch stack of child ids. `finish` consumes the builder, so the memos and scratch stack
//! cannot outlive the build.
//! EXCLUSIONS: string and path ids are copied as they are. The builder never remaps or merges them,
//! and it does not deduplicate distinct `TypeId`s that share a spelling. Struct and choice records
//! hold their path, and a choice also holds its variant names, but never member types. Recursive
//! definitions therefore stay finite.
//! FAILURE: `display_id` returns `TypeDisplayCapacityError` when an arena or the id space exceeds
//! `u32`. Discard the builder after any error, because the snapshot may hold a half-encoded record.

use super::type_display_snapshot::{
    CONST_RECORD_FLAG, CallableSummary, ChildRange, ChoiceSummary, CollectionSummary,
    DiagnosticTypeRecord, DiagnosticTypeTag, GenericInstanceSummary, TypeDisplaySnapshot,
    encode_path, push_range, push_summary,
};
use super::{TypeDisplayCapacityError, TypeDisplayId};
use crate::compiler_frontend::datatypes::display::{TypeDisplayShape, TypeDisplayView};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use rustc_hash::FxHashMap;

#[cfg(test)]
thread_local! {
    /// Test-only: while set on this thread, every top-level capture reports capacity exhaustion.
    static FORCED_TYPE_DISPLAY_EXHAUSTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only guard that forces top-level type-display capture to fail on this thread until dropped.
#[cfg(test)]
pub(crate) struct ForcedTypeDisplayExhaustionGuard;

#[cfg(test)]
impl ForcedTypeDisplayExhaustionGuard {
    pub(crate) fn new() -> Self {
        FORCED_TYPE_DISPLAY_EXHAUSTION.with(|forced| forced.set(true));
        Self
    }
}

#[cfg(test)]
impl Drop for ForcedTypeDisplayExhaustionGuard {
    fn drop(&mut self) {
        FORCED_TYPE_DISPLAY_EXHAUSTION.with(|forced| forced.set(false));
    }
}

pub(crate) struct TypeDisplaySnapshotBuilder<'environment> {
    environment: Option<&'environment TypeEnvironment>,
    snapshot: TypeDisplaySnapshot,
    memo: FxHashMap<TypeId, TypeDisplayId>,
    /// Separate from `memo`: unresolved roots and encoded unknown children have different shapes.
    unresolved_memo: FxHashMap<TypeId, TypeDisplayId>,
    /// Display ids of the child frames being collected, innermost frame last.
    scratch: Vec<TypeDisplayId>,
}

impl<'environment> TypeDisplaySnapshotBuilder<'environment> {
    /// Starts an empty snapshot. Without an environment every top-level type becomes an unresolved root.
    pub(crate) fn new(environment: Option<&'environment TypeEnvironment>) -> Self {
        Self {
            environment,
            snapshot: TypeDisplaySnapshot::default(),
            memo: FxHashMap::default(),
            unresolved_memo: FxHashMap::default(),
            scratch: Vec::new(),
        }
    }

    /// Returns the id of `type_id`, encoding it and its arguments on first request.
    ///
    /// The same `TypeId` always returns the same id. Distinct `TypeId`s get distinct ids even when
    /// their spellings are equal.
    pub(crate) fn display_id(
        &mut self,
        type_id: TypeId,
    ) -> Result<TypeDisplayId, TypeDisplayCapacityError> {
        if let Some(existing) = self.memo.get(&type_id) {
            return Ok(*existing);
        }

        let id = TypeDisplayId::from_index(self.snapshot.records.len())?;
        self.snapshot
            .records
            .push(DiagnosticTypeRecord::new(DiagnosticTypeTag::Unknown, 0, 0));
        // Record the id before encoding arguments, so a cyclic argument graph reuses this id.
        self.memo.insert(type_id, id);

        let record = self.encode(type_id)?;
        let slot = self
            .snapshot
            .records
            .get_mut(id.index())
            .expect("the placeholder record was pushed above");
        *slot = record;
        Ok(id)
    }

    /// Returns the id of a top-level type. A type the environment does not define, and every type without an
    /// environment, gets a transitional unresolved root. That root prints its raw `TypeId`, the legacy spelling.
    pub(crate) fn display_top_level(
        &mut self,
        type_id: TypeId,
    ) -> Result<TypeDisplayId, TypeDisplayCapacityError> {
        #[cfg(test)]
        if FORCED_TYPE_DISPLAY_EXHAUSTION.with(std::cell::Cell::get) {
            return Err(TypeDisplayCapacityError);
        }
        match self.environment {
            Some(environment) if environment.get(type_id).is_some() => self.display_id(type_id),
            _ => self.push_unresolved(type_id),
        }
    }

    /// Pushes or reuses an unresolved root, separate from any encoded unknown child record.
    fn push_unresolved(
        &mut self,
        type_id: TypeId,
    ) -> Result<TypeDisplayId, TypeDisplayCapacityError> {
        if let Some(existing) = self.unresolved_memo.get(&type_id) {
            return Ok(*existing);
        }

        let id = TypeDisplayId::from_index(self.snapshot.records.len())?;
        self.snapshot.records.push(DiagnosticTypeRecord::new(
            DiagnosticTypeTag::UnresolvedTypeId,
            type_id.0,
            0,
        ));
        self.unresolved_memo.insert(type_id, id);
        Ok(id)
    }

    /// Encodes the record for `type_id`. Argument ids are resolved before the record is returned.
    fn encode(
        &mut self,
        type_id: TypeId,
    ) -> Result<DiagnosticTypeRecord, TypeDisplayCapacityError> {
        let environment = self
            .environment
            .expect("nested types are encoded only from a resolved environment");
        let leaf = |tag| DiagnosticTypeRecord::new(tag, 0, 0);
        let record = match environment.shape(type_id) {
            TypeDisplayShape::Unknown => leaf(DiagnosticTypeTag::Unknown),
            TypeDisplayShape::Bool => leaf(DiagnosticTypeTag::Bool),
            TypeDisplayShape::String => leaf(DiagnosticTypeTag::String),
            TypeDisplayShape::Char => leaf(DiagnosticTypeTag::Char),
            TypeDisplayShape::Range => leaf(DiagnosticTypeTag::Range),
            TypeDisplayShape::NoneType => leaf(DiagnosticTypeTag::NoneType),
            TypeDisplayShape::Number(scale) => {
                DiagnosticTypeRecord::new(DiagnosticTypeTag::Number, u32::from(scale.get()), 0)
            }
            TypeDisplayShape::FixedScalar(scalar) => {
                DiagnosticTypeRecord::new(DiagnosticTypeTag::FixedScalar, scalar as u32, 0)
            }
            TypeDisplayShape::Struct { path, const_record } => {
                let mut record =
                    DiagnosticTypeRecord::new(DiagnosticTypeTag::Struct, encode_path(path), 0);
                if const_record {
                    record.flags = CONST_RECORD_FLAG;
                }
                record
            }
            TypeDisplayShape::Choice { path } => {
                let variants = push_range(
                    &mut self.snapshot.variants,
                    environment.choice_variants(type_id),
                )?;
                let index =
                    push_summary(&mut self.snapshot.choices, ChoiceSummary { path, variants })?;
                DiagnosticTypeRecord::new(DiagnosticTypeTag::Choice, index, 0)
            }
            TypeDisplayShape::Collection { fixed_capacity } => {
                let arguments =
                    self.child_range(environment.argument_ids(type_id).iter().copied())?;
                let index = push_summary(
                    &mut self.snapshot.collections,
                    CollectionSummary {
                        fixed_capacity,
                        arguments,
                    },
                )?;
                DiagnosticTypeRecord::new(DiagnosticTypeTag::Collection, index, 0)
            }
            TypeDisplayShape::OrderedMap => self.inline_arguments(
                DiagnosticTypeTag::OrderedMap,
                environment.argument_ids(type_id),
            )?,
            TypeDisplayShape::OptionType => self.inline_arguments(
                DiagnosticTypeTag::OptionType,
                environment.argument_ids(type_id),
            )?,
            TypeDisplayShape::Function { error_return } => {
                let parameters = self.child_range(environment.function_parameter_ids(type_id))?;
                let returns = self.child_range(environment.function_return_ids(type_id))?;
                let error_return = error_return
                    .map(|error| self.display_id(error))
                    .transpose()?;
                let index = push_summary(
                    &mut self.snapshot.callables,
                    CallableSummary {
                        parameters,
                        returns,
                        error_return,
                    },
                )?;
                DiagnosticTypeRecord::new(DiagnosticTypeTag::Function, index, 0)
            }
            TypeDisplayShape::External(external) => {
                DiagnosticTypeRecord::new(DiagnosticTypeTag::External, external, 0)
            }
            TypeDisplayShape::GenericParameter { name } => {
                DiagnosticTypeRecord::new(DiagnosticTypeTag::GenericParameter, name.index(), 0)
            }
            TypeDisplayShape::GenericInstance { base_path } => {
                let arguments =
                    self.child_range(environment.argument_ids(type_id).iter().copied())?;
                let index = push_summary(
                    &mut self.snapshot.generic_instances,
                    GenericInstanceSummary {
                        base_path,
                        arguments,
                    },
                )?;
                DiagnosticTypeRecord::new(DiagnosticTypeTag::GenericInstance, index, 0)
            }
            TypeDisplayShape::AnonymousConstRecord => leaf(DiagnosticTypeTag::AnonymousConstRecord),
        };
        Ok(record)
    }

    /// Encodes `arguments` into the `type_ids` arena. The record holds the range they occupy.
    fn inline_arguments(
        &mut self,
        tag: DiagnosticTypeTag,
        arguments: &[TypeId],
    ) -> Result<DiagnosticTypeRecord, TypeDisplayCapacityError> {
        let range = self.child_range(arguments.iter().copied())?;
        Ok(DiagnosticTypeRecord::new(tag, range.start, range.len))
    }

    /// Encodes `type_ids` as one contiguous range of the `type_ids` arena and returns that range.
    ///
    /// Each child id is pushed onto `scratch` above this frame's start. A nested frame releases
    /// its own ids before it returns, so the ids of this frame stay contiguous. The frame is
    /// released on error too, so a failed build leaves no ids behind on `scratch`.
    fn child_range(
        &mut self,
        type_ids: impl IntoIterator<Item = TypeId>,
    ) -> Result<ChildRange, TypeDisplayCapacityError> {
        let frame_start = self.scratch.len();
        let range = match self.push_child_ids(type_ids) {
            Ok(()) => push_range(
                &mut self.snapshot.type_ids,
                self.scratch[frame_start..].iter().copied(),
            ),
            Err(error) => Err(error),
        };
        self.scratch.truncate(frame_start);
        range
    }

    /// Encodes each type in order and pushes its display id onto `scratch`.
    fn push_child_ids(
        &mut self,
        type_ids: impl IntoIterator<Item = TypeId>,
    ) -> Result<(), TypeDisplayCapacityError> {
        for type_id in type_ids {
            let display_id = self.display_id(type_id)?;
            self.scratch.push(display_id);
        }
        Ok(())
    }

    /// Returns the completed snapshot. The memo is dropped with the builder.
    pub(crate) fn finish(self) -> TypeDisplaySnapshot {
        self.snapshot
    }
}
