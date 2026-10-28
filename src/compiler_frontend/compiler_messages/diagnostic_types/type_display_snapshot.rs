//! Frozen type-display snapshot store for compact diagnostics.
//!
//! WHAT: a set of flat arenas holding one 12-byte `DiagnosticTypeRecord` per type. Each record
//! names its spelling class and the ids that class needs to render. Callers address records by
//! `TypeDisplayId`.
//! WHY: a diagnostic must spell a type after the live `TypeEnvironment` is gone. The store renders
//! through the same `display_view_type` as the live environment, so both spell every type alike.
//! OWNERSHIP: the store owns its arenas. It holds no `Arc`, pointer, identity handle or table
//! reference. Callers pass the string and path tables when they render or remap.
//! EXCLUSIONS: building a store from a live environment is done by `type_display_snapshot_builder`.
//! This module never reads a `TypeEnvironment`.

use super::{TypeDisplayCapacityError, TypeDisplayId};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::display::{
    ChoiceVariantShape, TypeDisplayShape, TypeDisplayView, display_view_type,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    StringId, StringIdRemap, StringTableResolver,
};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Spelling class of one snapshot record. The class selects how the record's data words are read.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiagnosticTypeTag {
    Unknown,
    Bool,
    String,
    Char,
    Range,
    NoneType,
    Number,
    FixedScalar,
    Struct,
    Choice,
    Collection,
    OrderedMap,
    OptionType,
    Function,
    External,
    GenericParameter,
    GenericInstance,
    AnonymousConstRecord,
    /// Root captured without a resolved type spelling. `data_0` holds its source `TypeId`.
    /// Diagnostic rendering preserves that id as `TypeId(N)`.
    UnresolvedTypeId,
}

/// One type's spelling class and payload in 12 bytes.
///
/// The data words depend on `tag`:
/// - `Number`: `data_0` is the `Dec` scale.
/// - `FixedScalar`: `data_0` is the scalar discriminant.
/// - `Struct`: `data_0` is the path id. `flags` holds `CONST_RECORD_FLAG` for const records.
/// - `Choice`, `Collection`, `Function`, `GenericInstance`: `data_0` indexes the matching summary.
/// - `OrderedMap`, `OptionType`: `data_0` and `data_1` are the start and length of an argument range.
/// - `External`: `data_0` is the raw external type id.
/// - `GenericParameter`: `data_0` is the name's string id.
/// - Every other tag leaves both words zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DiagnosticTypeRecord {
    pub(super) tag: DiagnosticTypeTag,
    pub(super) flags: u16,
    pub(super) data_0: u32,
    pub(super) data_1: u32,
}

const _: () = assert!(std::mem::size_of::<DiagnosticTypeRecord>() == 12);

impl DiagnosticTypeRecord {
    /// A record with no flags set.
    pub(super) fn new(tag: DiagnosticTypeTag, data_0: u32, data_1: u32) -> Self {
        Self {
            tag,
            flags: 0,
            data_0,
            data_1,
        }
    }
}

/// Set in `flags` when a `Struct` record names a const record.
pub(super) const CONST_RECORD_FLAG: u16 = 1;

/// A run of items in one arena, addressed by its start index and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChildRange {
    pub(super) start: u32,
    pub(super) len: u32,
}

/// Choice spelling data. Variant shapes live in the snapshot's `variants` arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChoiceSummary {
    pub(super) path: PathId,
    pub(super) variants: ChildRange,
}

/// Function spelling data. Parameter and return ids live in the snapshot's `type_ids` arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallableSummary {
    pub(super) parameters: ChildRange,
    pub(super) returns: ChildRange,
    pub(super) error_return: Option<TypeDisplayId>,
}

/// Generic instance spelling data. Argument ids live in the snapshot's `type_ids` arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GenericInstanceSummary {
    pub(super) base_path: Option<PathId>,
    pub(super) arguments: ChildRange,
}

/// Collection spelling data. Element ids live in the snapshot's `type_ids` arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CollectionSummary {
    pub(super) fixed_capacity: Option<usize>,
    pub(super) arguments: ChildRange,
}

/// Frozen spelling data for a set of types. Built by `TypeDisplaySnapshotBuilder`.
#[derive(Debug, Default)]
pub(crate) struct TypeDisplaySnapshot {
    pub(super) records: Vec<DiagnosticTypeRecord>,
    pub(super) type_ids: Vec<TypeDisplayId>,
    pub(super) variants: Vec<ChoiceVariantShape>,
    pub(super) choices: Vec<ChoiceSummary>,
    pub(super) callables: Vec<CallableSummary>,
    pub(super) generic_instances: Vec<GenericInstanceSummary>,
    pub(super) collections: Vec<CollectionSummary>,
}

#[cfg(any(test, feature = "data_layout_memory_probe"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TypeDisplaySnapshotMemoryMetrics {
    pub(crate) record_rows: usize,
    pub(crate) record_capacity_bytes: usize,
    pub(crate) type_id_rows: usize,
    pub(crate) type_id_capacity_bytes: usize,
    pub(crate) variant_rows: usize,
    pub(crate) variant_capacity_bytes: usize,
    pub(crate) choice_rows: usize,
    pub(crate) choice_capacity_bytes: usize,
    pub(crate) callable_rows: usize,
    pub(crate) callable_capacity_bytes: usize,
    pub(crate) generic_instance_rows: usize,
    pub(crate) generic_instance_capacity_bytes: usize,
    pub(crate) collection_rows: usize,
    pub(crate) collection_capacity_bytes: usize,
}

#[cfg(any(test, feature = "data_layout_memory_probe"))]
impl TypeDisplaySnapshot {
    pub(crate) fn memory_ledger_metrics(&self) -> TypeDisplaySnapshotMemoryMetrics {
        TypeDisplaySnapshotMemoryMetrics {
            record_rows: self.records.len(),
            record_capacity_bytes: self
                .records
                .capacity()
                .saturating_mul(std::mem::size_of::<DiagnosticTypeRecord>()),
            type_id_rows: self.type_ids.len(),
            type_id_capacity_bytes: self
                .type_ids
                .capacity()
                .saturating_mul(std::mem::size_of::<super::TypeDisplayId>()),
            variant_rows: self.variants.len(),
            variant_capacity_bytes: self
                .variants
                .capacity()
                .saturating_mul(std::mem::size_of::<ChoiceVariantShape>()),
            choice_rows: self.choices.len(),
            choice_capacity_bytes: self
                .choices
                .capacity()
                .saturating_mul(std::mem::size_of::<ChoiceSummary>()),
            callable_rows: self.callables.len(),
            callable_capacity_bytes: self
                .callables
                .capacity()
                .saturating_mul(std::mem::size_of::<CallableSummary>()),
            generic_instance_rows: self.generic_instances.len(),
            generic_instance_capacity_bytes: self
                .generic_instances
                .capacity()
                .saturating_mul(std::mem::size_of::<GenericInstanceSummary>()),
            collection_rows: self.collections.len(),
            collection_capacity_bytes: self
                .collections
                .capacity()
                .saturating_mul(std::mem::size_of::<CollectionSummary>()),
        }
    }
}

/// Converts an arena length into the `u32` index space that records and ranges use.
pub(super) fn arena_index(len: usize) -> Result<u32, TypeDisplayCapacityError> {
    u32::try_from(len).map_err(|_| TypeDisplayCapacityError)
}

/// Appends `items` to `arena` and returns the range they occupy.
pub(super) fn push_range<Item>(
    arena: &mut Vec<Item>,
    items: impl IntoIterator<Item = Item>,
) -> Result<ChildRange, TypeDisplayCapacityError> {
    let start = arena_index(arena.len())?;
    arena.extend(items);
    let end = arena_index(arena.len())?;
    Ok(ChildRange {
        start,
        len: end - start,
    })
}

/// Appends one summary to `arena` and returns its index.
pub(super) fn push_summary<Item>(
    arena: &mut Vec<Item>,
    item: Item,
) -> Result<u32, TypeDisplayCapacityError> {
    let index = arena_index(arena.len())?;
    arena.push(item);
    Ok(index)
}

/// Encodes a path id as a record data word. Path indices fit in `u32` by construction.
pub(super) fn encode_path(path: PathId) -> u32 {
    u32::try_from(path.index()).expect("path table indices fit in u32 by construction")
}

/// Decodes a data word written by `encode_path`.
fn decode_path(word: u32) -> PathId {
    PathId::try_from_index(word as usize)
        .expect("path data words are written by encode_path from a valid PathId")
}

/// Returns the items a range covers. Ranges come from `push_range` on the same arena.
fn range_slice<Item>(arena: &[Item], range: ChildRange) -> &[Item] {
    let start = range.start as usize;
    let end = start + range.len as usize;
    arena
        .get(start..end)
        .expect("child ranges are written by push_range on the same arena")
}

/// Returns the summary at `index`. Indices come from `push_summary` on the same arena.
fn summary<Item>(arena: &[Item], index: u32) -> &Item {
    arena
        .get(index as usize)
        .expect("summary indices are written by push_summary on the same arena")
}

/// Decodes a `Number` data word written from a validated `NumberScale`.
fn decode_number_scale(word: u32) -> NumberScale {
    u16::try_from(word)
        .ok()
        .and_then(NumberScale::new)
        .expect("number data words are written from a validated NumberScale")
}

/// Decodes a `FixedScalar` data word written from a scalar discriminant.
fn decode_fixed_scalar(word: u32) -> FixedScalar {
    FixedScalar::ALL
        .into_iter()
        .find(|scalar| *scalar as u32 == word)
        .expect("fixed scalar data words are written from a FixedScalar discriminant")
}

impl TypeDisplaySnapshot {
    /// Appends every path needed to render the captured type spellings.
    pub(crate) fn required_paths(&self, paths: &mut Vec<PathId>) {
        paths.extend(
            self.records
                .iter()
                .filter(|record| record.tag == DiagnosticTypeTag::Struct)
                .map(|record| decode_path(record.data_0)),
        );
        paths.extend(self.choices.iter().map(|choice| choice.path));
        paths.extend(
            self.generic_instances
                .iter()
                .filter_map(|instance| instance.base_path),
        );
    }

    /// Renders the spelling of `type_id`. The result matches `display_type` for the live type.
    pub(crate) fn display(
        &self,
        type_id: TypeDisplayId,
        table: &dyn StringTableResolver,
        path_table: &PathTable,
    ) -> String {
        display_view_type(self, type_id, table, path_table)
    }

    /// Renders the spelling of `type_id` for a diagnostic. An unresolved root prints its raw `TypeId`;
    /// a resolved type needs the path table to spell its paths.
    pub(crate) fn display_for_render(
        &self,
        type_id: TypeDisplayId,
        table: &dyn StringTableResolver,
        path_table: Option<&PathTable>,
    ) -> String {
        let record = self.record(type_id);
        match record.tag {
            DiagnosticTypeTag::UnresolvedTypeId => format!("TypeId({})", record.data_0),
            _ => self.display(
                type_id,
                table,
                path_table.expect("resolved type spellings need the path table"),
            ),
        }
    }

    fn record(&self, type_id: TypeDisplayId) -> &DiagnosticTypeRecord {
        self.records
            .get(type_id.index())
            .expect("type-display ids are issued by the builder that produced this snapshot")
    }

    fn choice(&self, type_id: TypeDisplayId) -> &ChoiceSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::Choice = record.tag else {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "choice summaries are read only for choice records"
            )
        };
        summary(&self.choices, record.data_0)
    }

    fn callable(&self, type_id: TypeDisplayId) -> &CallableSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::Function = record.tag else {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "callable summaries are read only for function records"
            )
        };
        summary(&self.callables, record.data_0)
    }

    fn collection(&self, type_id: TypeDisplayId) -> &CollectionSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::Collection = record.tag else {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "collection summaries are read only for collection records"
            )
        };
        summary(&self.collections, record.data_0)
    }

    fn generic_instance(&self, type_id: TypeDisplayId) -> &GenericInstanceSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::GenericInstance = record.tag else {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "generic instance summaries are read only for generic instance records"
            )
        };
        summary(&self.generic_instances, record.data_0)
    }
}

impl TypeDisplaySnapshot {
    /// Moves another snapshot after this one and returns its zero-based display-word offset.
    pub(crate) fn append(
        &mut self,
        mut other: TypeDisplaySnapshot,
    ) -> Result<u32, TypeDisplayCapacityError> {
        let record_base = arena_index(self.records.len())?;
        checked_arena_end(self.records.len(), other.records.len())?;
        let type_ids_base = arena_index(self.type_ids.len())?;
        checked_arena_end(self.type_ids.len(), other.type_ids.len())?;
        let variants_base = arena_index(self.variants.len())?;
        checked_arena_end(self.variants.len(), other.variants.len())?;
        let choices_base = arena_index(self.choices.len())?;
        checked_arena_end(self.choices.len(), other.choices.len())?;
        let callables_base = arena_index(self.callables.len())?;
        checked_arena_end(self.callables.len(), other.callables.len())?;
        let generic_instances_base = arena_index(self.generic_instances.len())?;
        checked_arena_end(self.generic_instances.len(), other.generic_instances.len())?;
        let collections_base = arena_index(self.collections.len())?;
        checked_arena_end(self.collections.len(), other.collections.len())?;

        for record in &mut other.records {
            match record.tag {
                DiagnosticTypeTag::Choice => {
                    record.data_0 = offset_index(record.data_0, choices_base)?;
                }
                DiagnosticTypeTag::Collection => {
                    record.data_0 = offset_index(record.data_0, collections_base)?;
                }
                DiagnosticTypeTag::Function => {
                    record.data_0 = offset_index(record.data_0, callables_base)?;
                }
                DiagnosticTypeTag::GenericInstance => {
                    record.data_0 = offset_index(record.data_0, generic_instances_base)?;
                }
                DiagnosticTypeTag::OrderedMap | DiagnosticTypeTag::OptionType => {
                    record.data_0 = offset_index(record.data_0, type_ids_base)?;
                }
                DiagnosticTypeTag::Unknown
                | DiagnosticTypeTag::Bool
                | DiagnosticTypeTag::String
                | DiagnosticTypeTag::Char
                | DiagnosticTypeTag::Range
                | DiagnosticTypeTag::NoneType
                | DiagnosticTypeTag::Number
                | DiagnosticTypeTag::FixedScalar
                | DiagnosticTypeTag::Struct
                | DiagnosticTypeTag::External
                | DiagnosticTypeTag::GenericParameter
                | DiagnosticTypeTag::AnonymousConstRecord
                | DiagnosticTypeTag::UnresolvedTypeId => {}
            }
        }
        for type_id in &mut other.type_ids {
            *type_id = offset_type_display_id(*type_id, record_base)?;
        }
        for choice in &mut other.choices {
            choice.variants = offset_range(choice.variants, variants_base)?;
        }
        for callable in &mut other.callables {
            callable.parameters = offset_range(callable.parameters, type_ids_base)?;
            callable.returns = offset_range(callable.returns, type_ids_base)?;
            callable.error_return = callable
                .error_return
                .map(|type_id| offset_type_display_id(type_id, record_base))
                .transpose()?;
        }
        for generic_instance in &mut other.generic_instances {
            generic_instance.arguments = offset_range(generic_instance.arguments, type_ids_base)?;
        }
        for collection in &mut other.collections {
            collection.arguments = offset_range(collection.arguments, type_ids_base)?;
        }

        self.records.extend(other.records);
        self.type_ids.extend(other.type_ids);
        self.variants.extend(other.variants);
        self.choices.extend(other.choices);
        self.callables.extend(other.callables);
        self.generic_instances.extend(other.generic_instances);
        self.collections.extend(other.collections);
        Ok(record_base)
    }
}

/// Checks that appending `additional` items keeps this arena in the `u32` index space.
fn checked_arena_end(current: usize, additional: usize) -> Result<u32, TypeDisplayCapacityError> {
    let current = arena_index(current)?;
    let additional = arena_index(additional)?;
    current
        .checked_add(additional)
        .ok_or(TypeDisplayCapacityError)
}

/// Offsets a child-arena index without allowing it to wrap.
fn offset_index(index: u32, base: u32) -> Result<u32, TypeDisplayCapacityError> {
    index.checked_add(base).ok_or(TypeDisplayCapacityError)
}

/// Offsets the start of a child-arena range. Lengths stay unchanged.
fn offset_range(range: ChildRange, base: u32) -> Result<ChildRange, TypeDisplayCapacityError> {
    Ok(ChildRange {
        start: offset_index(range.start, base)?,
        len: range.len,
    })
}

/// Offsets a type-display id by the number of records already in the destination snapshot.
fn offset_type_display_id(
    type_id: TypeDisplayId,
    base: u32,
) -> Result<TypeDisplayId, TypeDisplayCapacityError> {
    let index = u32::try_from(type_id.index()).map_err(|_| TypeDisplayCapacityError)?;
    let index = offset_index(index, base)?;
    TypeDisplayId::from_index(usize::try_from(index).map_err(|_| TypeDisplayCapacityError)?)
}

impl TypeDisplaySnapshot {
    /// Rewrites every string id after the string table that issued them merged into another table.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if remap.is_identity() {
            return;
        }
        for record in &mut self.records {
            if record.tag == DiagnosticTypeTag::GenericParameter {
                let name = remap.get(StringId::from_index(record.data_0));
                record.data_0 = name.index();
            }
        }
        for variant in &mut self.variants {
            variant.name = remap.get(variant.name);
        }
    }

    /// Rewrites every path id after the path table that issued them merged into another table.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        for record in &mut self.records {
            if record.tag == DiagnosticTypeTag::Struct {
                record.data_0 = encode_path(remap.get(decode_path(record.data_0)));
            }
        }
        for choice in &mut self.choices {
            choice.path = remap.get(choice.path);
        }
        for instance in &mut self.generic_instances {
            instance.base_path = instance.base_path.map(|path| remap.get(path));
        }
    }
}

impl TypeDisplayView for TypeDisplaySnapshot {
    type Id = TypeDisplayId;

    fn shape(&self, type_id: TypeDisplayId) -> TypeDisplayShape<TypeDisplayId> {
        let record = self.record(type_id);
        match record.tag {
            DiagnosticTypeTag::Unknown => TypeDisplayShape::Unknown,
            DiagnosticTypeTag::Bool => TypeDisplayShape::Bool,
            DiagnosticTypeTag::String => TypeDisplayShape::String,
            DiagnosticTypeTag::Char => TypeDisplayShape::Char,
            DiagnosticTypeTag::Range => TypeDisplayShape::Range,
            DiagnosticTypeTag::NoneType => TypeDisplayShape::NoneType,
            DiagnosticTypeTag::Number => {
                TypeDisplayShape::Number(decode_number_scale(record.data_0))
            }
            DiagnosticTypeTag::FixedScalar => {
                TypeDisplayShape::FixedScalar(decode_fixed_scalar(record.data_0))
            }
            DiagnosticTypeTag::Struct => TypeDisplayShape::Struct {
                path: decode_path(record.data_0),
                const_record: (record.flags & CONST_RECORD_FLAG) != 0,
            },
            DiagnosticTypeTag::Choice => TypeDisplayShape::Choice {
                path: self.choice(type_id).path,
            },
            DiagnosticTypeTag::Collection => TypeDisplayShape::Collection {
                fixed_capacity: self.collection(type_id).fixed_capacity,
            },
            DiagnosticTypeTag::OrderedMap => TypeDisplayShape::OrderedMap,
            DiagnosticTypeTag::OptionType => TypeDisplayShape::OptionType,
            DiagnosticTypeTag::Function => TypeDisplayShape::Function {
                error_return: self.callable(type_id).error_return,
            },
            DiagnosticTypeTag::External => TypeDisplayShape::External(record.data_0),
            DiagnosticTypeTag::GenericParameter => TypeDisplayShape::GenericParameter {
                name: StringId::from_index(record.data_0),
            },
            DiagnosticTypeTag::GenericInstance => TypeDisplayShape::GenericInstance {
                base_path: self.generic_instance(type_id).base_path,
            },
            DiagnosticTypeTag::AnonymousConstRecord => TypeDisplayShape::AnonymousConstRecord,
            DiagnosticTypeTag::UnresolvedTypeId => TypeDisplayShape::Unknown,
        }
    }

    fn argument_ids(&self, type_id: TypeDisplayId) -> &[TypeDisplayId] {
        let record = self.record(type_id);
        match record.tag {
            DiagnosticTypeTag::OrderedMap | DiagnosticTypeTag::OptionType => range_slice(
                &self.type_ids,
                ChildRange {
                    start: record.data_0,
                    len: record.data_1,
                },
            ),
            DiagnosticTypeTag::Collection => {
                range_slice(&self.type_ids, self.collection(type_id).arguments)
            }
            DiagnosticTypeTag::GenericInstance => {
                range_slice(&self.type_ids, self.generic_instance(type_id).arguments)
            }
            _ => &[],
        }
    }

    fn function_parameter_ids(
        &self,
        type_id: TypeDisplayId,
    ) -> impl Iterator<Item = TypeDisplayId> {
        range_slice(&self.type_ids, self.callable(type_id).parameters)
            .iter()
            .copied()
    }

    fn function_return_ids(&self, type_id: TypeDisplayId) -> impl Iterator<Item = TypeDisplayId> {
        range_slice(&self.type_ids, self.callable(type_id).returns)
            .iter()
            .copied()
    }

    fn choice_variants(&self, type_id: TypeDisplayId) -> impl Iterator<Item = ChoiceVariantShape> {
        range_slice(&self.variants, self.choice(type_id).variants)
            .iter()
            .copied()
    }
}
