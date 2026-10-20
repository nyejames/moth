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
use crate::compiler_frontend::datatypes::display::{
    ChoiceVariantShape, TypeDisplayShape, TypeDisplayView, display_view_type,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
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
    /// Transitional root for a top-level type that legacy rendering prints as its raw `TypeId`.
    /// `data_0` holds the raw id. P4.5 deletes this tag with the transitional fallback.
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
    /// Renders the spelling of `type_id`. The result matches `display_type` for the live type.
    pub(crate) fn display(
        &self,
        type_id: TypeDisplayId,
        table: &dyn StringTableResolver,
        path_table: &PathTable,
    ) -> String {
        display_view_type(self, type_id, table, path_table)
    }

    /// Renders the spelling of `type_id` for a diagnostic. An unresolved root prints its raw `TypeId`, which is the
    /// legacy spelling when the environment or path table is missing or the type is unknown.
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

    /// Transitional: serves only the `TypeId(N)` fallback; P4.5 deletes it with that fallback.
    pub(crate) fn unresolved_type_id(&self, type_id: TypeDisplayId) -> Option<TypeId> {
        let record = self.record(type_id);
        (record.tag == DiagnosticTypeTag::UnresolvedTypeId).then_some(TypeId(record.data_0))
    }

    fn record(&self, type_id: TypeDisplayId) -> &DiagnosticTypeRecord {
        self.records
            .get(type_id.index())
            .expect("type-display ids are issued by the builder that produced this snapshot")
    }

    fn choice(&self, type_id: TypeDisplayId) -> &ChoiceSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::Choice = record.tag else {
            unreachable!("choice summaries are read only for choice records")
        };
        summary(&self.choices, record.data_0)
    }

    fn callable(&self, type_id: TypeDisplayId) -> &CallableSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::Function = record.tag else {
            unreachable!("callable summaries are read only for function records")
        };
        summary(&self.callables, record.data_0)
    }

    fn collection(&self, type_id: TypeDisplayId) -> &CollectionSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::Collection = record.tag else {
            unreachable!("collection summaries are read only for collection records")
        };
        summary(&self.collections, record.data_0)
    }

    fn generic_instance(&self, type_id: TypeDisplayId) -> &GenericInstanceSummary {
        let record = self.record(type_id);
        let DiagnosticTypeTag::GenericInstance = record.tag else {
            unreachable!("generic instance summaries are read only for generic instance records")
        };
        summary(&self.generic_instances, record.data_0)
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "P4.5 canonical merge remaps type-display snapshots"
    )
)]
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
            DiagnosticTypeTag::UnresolvedTypeId => {
                unreachable!("unresolved roots render through display_for_render")
            }
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
