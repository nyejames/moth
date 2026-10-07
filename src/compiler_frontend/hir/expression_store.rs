//! Dense expression rows and typed HIR side ranges.
//!
//! WHAT: owns the module-local expression graph and its variable-sized edge payloads.
//! WHY: store positions issue `HirValueId`s, and one append/freeze owner keeps every graph edge
//!      valid across growth and before immutable HIR publication.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, HirCapacityResource};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirMapEntry, HirVariantField};
use crate::compiler_frontend::hir::ids::{FieldId, HirValueId, RegionId};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;

/// A source-caused HIR capacity diagnostic or an infrastructure/invariant failure.
#[derive(Debug)]
pub(crate) enum HirConstructionFailure {
    Diagnosed(CompilerDiagnostic),
    Infrastructure(CompilerError),
}

impl From<CompilerDiagnostic> for HirConstructionFailure {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::Diagnosed(diagnostic)
    }
}

impl From<CompilerError> for HirConstructionFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }
}

macro_rules! define_hir_range {
    ($name:ident $(, $method:ident)*) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name {
            start: u32,
            length: u32,
        }

        impl $name {
            fn from_checked_parts(start: u32, length: u32) -> Self {
                Self { start, length }
            }
            $(define_hir_range!(@method $method);)*
        }
    };

    (@method empty) => {
        pub const fn empty() -> Self {
            Self {
                start: 0,
                length: 0,
            }
        }
    };

    (@method len) => {
        pub const fn len(self) -> usize {
            self.length as usize
        }
    };

    (@method is_empty) => {
        pub const fn is_empty(self) -> bool {
            self.length == 0
        }
    };
}

define_hir_range!(HirValueRange, empty, len);
define_hir_range!(HirStructFieldRange, len);
define_hir_range!(HirVariantFieldRange, empty, len);
define_hir_range!(HirMapEntryRange, len);
define_hir_range!(HirStringPieceRange);
define_hir_range!(HirProjectionRange, empty, len, is_empty);

/// One root-outward place projection. Index expressions remain graph IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HirProjection {
    Field(FieldId),
    Index(HirValueId),
}

/// Immutable measurement values used by focused tests and benchmark counters.
#[cfg(any(test, feature = "benchmark_counters"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HirExpressionBufferMeasurements {
    pub len: usize,
    /// A frozen boxed slice retains exactly its length; a building vector reports its capacity.
    pub retained_capacity: usize,
    pub initial_capacity: usize,
    pub growth_events: usize,
}

/// Per-buffer measurements for the one authoritative expression store.
#[cfg(any(test, feature = "benchmark_counters"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HirExpressionStoreMeasurements {
    pub rows: HirExpressionBufferMeasurements,
    pub values: HirExpressionBufferMeasurements,
    pub struct_fields: HirExpressionBufferMeasurements,
    pub variant_fields: HirExpressionBufferMeasurements,
    pub map_entries: HirExpressionBufferMeasurements,
    pub string_pieces: HirExpressionBufferMeasurements,
    pub projections: HirExpressionBufferMeasurements,
}

/// The module's append-only graph owner until its final structural producer completes.
#[derive(Debug, Clone)]
pub struct HirExpressionStore {
    storage: HirExpressionStoreStorage,
    #[cfg(test)]
    test_limits: Option<HirExpressionStoreTestLimits>,
}

#[derive(Debug, Clone)]
enum HirExpressionStoreStorage {
    Building(BuildingHirExpressionStore),
    Frozen(FrozenHirExpressionStore),
}

#[derive(Debug, Clone, Default)]
struct BuildingHirExpressionStore {
    rows: Vec<HirExpression>,
    values: Vec<HirValueId>,
    struct_fields: Vec<(FieldId, HirValueId)>,
    variant_fields: Vec<HirVariantField>,
    map_entries: Vec<HirMapEntry>,
    string_pieces: Vec<ConstStringPiece>,
    projections: Vec<HirProjection>,
    #[cfg(any(test, feature = "benchmark_counters"))]
    accounting: HirExpressionStoreAccounting,
}

#[derive(Debug, Clone)]
struct FrozenHirExpressionStore {
    rows: Box<[HirExpression]>,
    values: Box<[HirValueId]>,
    struct_fields: Box<[(FieldId, HirValueId)]>,
    variant_fields: Box<[HirVariantField]>,
    map_entries: Box<[HirMapEntry]>,
    string_pieces: Box<[ConstStringPiece]>,
    projections: Box<[HirProjection]>,
    #[cfg(any(test, feature = "benchmark_counters"))]
    accounting: HirExpressionStoreAccounting,
}

#[cfg(any(test, feature = "benchmark_counters"))]
#[derive(Debug, Clone, Copy, Default)]
struct HirExpressionStoreAccounting {
    initial: [usize; 7],
    growths: [usize; 7],
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HirExpressionStoreTestLimits {
    pub rows: usize,
    pub values: usize,
    pub struct_fields: usize,
    pub variant_fields: usize,
    pub map_entries: usize,
    pub string_pieces: usize,
    pub projections: usize,
}

impl Default for HirExpressionStore {
    fn default() -> Self {
        Self::with_capacity(0, 0)
    }
}

impl HirExpressionStore {
    /// Start a module-local store with estimates for the two common dense buffers.
    pub(crate) fn with_capacity(row_capacity: usize, value_capacity: usize) -> Self {
        let rows = Vec::with_capacity(row_capacity);
        let values = Vec::with_capacity(value_capacity);
        let struct_fields = Vec::new();
        let variant_fields = Vec::new();
        let map_entries = Vec::new();
        let string_pieces = Vec::new();
        let projections = Vec::new();

        #[cfg(feature = "benchmark_counters")]
        {
            add_frontend_counter(FrontendCounter::EstimatedHirExpressions, row_capacity);
            add_frontend_counter(FrontendCounter::EstimatedHirValueItems, value_capacity);
        }

        #[cfg(any(test, feature = "benchmark_counters"))]
        let accounting = HirExpressionStoreAccounting {
            initial: [
                rows.capacity(),
                values.capacity(),
                struct_fields.capacity(),
                variant_fields.capacity(),
                map_entries.capacity(),
                string_pieces.capacity(),
                projections.capacity(),
            ],
            growths: [0; 7],
        };

        Self {
            storage: HirExpressionStoreStorage::Building(BuildingHirExpressionStore {
                rows,
                values,
                struct_fields,
                variant_fields,
                map_entries,
                string_pieces,
                projections,
                #[cfg(any(test, feature = "benchmark_counters"))]
                accounting,
            }),
            #[cfg(test)]
            test_limits: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_limits(limits: HirExpressionStoreTestLimits) -> Self {
        Self {
            test_limits: Some(limits),
            ..Self::default()
        }
    }

    pub(crate) fn is_frozen(&self) -> bool {
        matches!(self.storage, HirExpressionStoreStorage::Frozen(_))
    }

    /// Freeze every append buffer to exact-sized boxed storage after the final structural rewrite.
    pub(crate) fn freeze(&mut self) {
        if self.is_frozen() {
            return;
        }

        #[cfg(feature = "benchmark_counters")]
        let construction_measurements = self.measurements();

        let previous = std::mem::replace(
            &mut self.storage,
            HirExpressionStoreStorage::Building(BuildingHirExpressionStore::default()),
        );
        self.storage = match previous {
            HirExpressionStoreStorage::Building(building) => {
                HirExpressionStoreStorage::Frozen(FrozenHirExpressionStore {
                    rows: building.rows.into_boxed_slice(),
                    values: building.values.into_boxed_slice(),
                    struct_fields: building.struct_fields.into_boxed_slice(),
                    variant_fields: building.variant_fields.into_boxed_slice(),
                    map_entries: building.map_entries.into_boxed_slice(),
                    string_pieces: building.string_pieces.into_boxed_slice(),
                    projections: building.projections.into_boxed_slice(),
                    #[cfg(any(test, feature = "benchmark_counters"))]
                    accounting: building.accounting,
                })
            }
            frozen @ HirExpressionStoreStorage::Frozen(_) => frozen,
        };

        #[cfg(feature = "benchmark_counters")]
        self.record_freeze_measurements(construction_measurements);
    }

    pub fn expression(&self, id: HirValueId) -> &HirExpression {
        self.get_expression(id)
            .expect("validated HIR value ID must identify a dense expression row")
    }

    pub fn get_expression(&self, id: HirValueId) -> Option<&HirExpression> {
        self.row_slice().get(id.0 as usize)
    }

    pub(crate) fn expression_count(&self) -> usize {
        self.row_slice().len()
    }

    /// Reuse an expression row when its scalar metadata is unchanged; otherwise append a copy.
    ///
    /// Expression rows are immutable once they have an ID because multiple HIR edges may share
    /// one row. Metadata updates therefore create a new root while retaining the existing child
    /// and side-range references.
    pub(crate) fn copy_expression_with_metadata(
        &mut self,
        id: HirValueId,
        span: Option<SourceSpan>,
        ty: crate::compiler_frontend::datatypes::ids::TypeId,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let Some(expression) = self.get_expression(id) else {
            return Err(Self::infrastructure_failure(
                format!("HIR expression ID {:?} is outside its dense row store", id),
                span,
            ));
        };
        if expression.span == span && expression.ty == ty && expression.region == region {
            return Ok(id);
        }

        let row_id = self.prepare_expression_append(span)?;
        let Some(mut replacement) = self.get_expression(id).cloned() else {
            return Err(Self::infrastructure_failure(
                format!("HIR expression ID {:?} is outside its dense row store", id),
                span,
            ));
        };
        replacement.span = span;
        replacement.ty = ty;
        replacement.region = region;
        self.building_mut(span)?.rows.push(replacement);
        Ok(row_id)
    }

    pub fn values(&self, range: HirValueRange) -> &[HirValueId] {
        self.get_values(range)
            .expect("validated HIR value range must fit the dense value side store")
    }

    pub fn get_values(&self, range: HirValueRange) -> Option<&[HirValueId]> {
        slice_for_range(self.value_slice(), range)
    }

    pub fn struct_fields(&self, range: HirStructFieldRange) -> &[(FieldId, HirValueId)] {
        self.get_struct_fields(range)
            .expect("validated struct field range must fit its side store")
    }

    pub fn get_struct_fields(
        &self,
        range: HirStructFieldRange,
    ) -> Option<&[(FieldId, HirValueId)]> {
        slice_for_range(self.struct_field_slice(), range)
    }

    pub fn variant_fields(&self, range: HirVariantFieldRange) -> &[HirVariantField] {
        self.get_variant_fields(range)
            .expect("validated variant field range must fit its side store")
    }

    pub fn get_variant_fields(&self, range: HirVariantFieldRange) -> Option<&[HirVariantField]> {
        slice_for_range(self.variant_field_slice(), range)
    }

    pub fn map_entries(&self, range: HirMapEntryRange) -> &[HirMapEntry] {
        self.get_map_entries(range)
            .expect("validated map entry range must fit its side store")
    }

    pub fn get_map_entries(&self, range: HirMapEntryRange) -> Option<&[HirMapEntry]> {
        slice_for_range(self.map_entry_slice(), range)
    }

    pub fn string_pieces(&self, range: HirStringPieceRange) -> &[ConstStringPiece] {
        self.get_string_pieces(range)
            .expect("validated string piece range must fit its side store")
    }

    pub fn get_string_pieces(&self, range: HirStringPieceRange) -> Option<&[ConstStringPiece]> {
        slice_for_range(self.string_piece_slice(), range)
    }

    pub fn projections(&self, range: HirProjectionRange) -> &[HirProjection] {
        self.get_projections(range)
            .expect("validated place projection range must fit its side store")
    }

    pub fn get_projections(&self, range: HirProjectionRange) -> Option<&[HirProjection]> {
        slice_for_range(self.projection_slice(), range)
    }

    pub(crate) fn append_expression(
        &mut self,
        row: HirExpression,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let row_id = self.prepare_expression_append(row.span)?;
        self.building_mut(row.span)?.rows.push(row);
        Ok(row_id)
    }

    fn prepare_expression_append(
        &mut self,
        span: Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.is_frozen() {
            return Err(Self::infrastructure_failure(
                "attempted to append to a frozen HIR expression store".to_owned(),
                span,
            ));
        }
        let row_id = HirValueId(
            u32::try_from(self.row_slice().len())
                .map_err(|_| Self::capacity_failure(HirCapacityResource::ExpressionRows, span))?,
        );
        #[cfg(test)]
        if self
            .test_limits
            .is_some_and(|limits| self.row_slice().len() >= limits.rows)
        {
            return Err(Self::capacity_failure(
                HirCapacityResource::ExpressionRows,
                span,
            ));
        }

        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.rows.capacity();
        building.rows.try_reserve(1).map_err(|error| {
            Self::infrastructure_failure(
                format!("could not reserve a HIR expression row: {error}"),
                span,
            )
        })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.rows.capacity() > old_capacity {
            building.accounting.growths[0] += 1;
        }
        Ok(row_id)
    }

    pub(crate) fn append_values(
        &mut self,
        values: &[HirValueId],
        span: Option<SourceSpan>,
    ) -> Result<HirValueRange, HirConstructionFailure> {
        let resource = HirCapacityResource::ValueEdges;
        let range = self.checked_side_range(
            self.value_slice().len(),
            values.len(),
            resource,
            span,
            self.test_limit(resource),
        )?;
        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.values.capacity();
        building.values.try_reserve(values.len()).map_err(|error| {
            Self::infrastructure_failure(
                format!("could not reserve HIR value edges: {error}"),
                span,
            )
        })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.values.capacity() > old_capacity {
            building.accounting.growths[1] += 1;
        }
        building.values.extend_from_slice(values);
        Ok(range)
    }

    pub(crate) fn append_struct_fields(
        &mut self,
        fields: &[(FieldId, HirValueId)],
        span: Option<SourceSpan>,
    ) -> Result<HirStructFieldRange, HirConstructionFailure> {
        let resource = HirCapacityResource::StructFields;
        let range = self.checked_side_range(
            self.struct_field_slice().len(),
            fields.len(),
            resource,
            span,
            self.test_limit(resource),
        )?;
        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.struct_fields.capacity();
        building
            .struct_fields
            .try_reserve(fields.len())
            .map_err(|error| {
                Self::infrastructure_failure(
                    format!("could not reserve HIR struct fields: {error}"),
                    span,
                )
            })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.struct_fields.capacity() > old_capacity {
            building.accounting.growths[2] += 1;
        }
        building.struct_fields.extend_from_slice(fields);
        Ok(range)
    }

    pub(crate) fn append_variant_fields(
        &mut self,
        fields: &[HirVariantField],
        span: Option<SourceSpan>,
    ) -> Result<HirVariantFieldRange, HirConstructionFailure> {
        let resource = HirCapacityResource::VariantFields;
        let range = self.checked_side_range(
            self.variant_field_slice().len(),
            fields.len(),
            resource,
            span,
            self.test_limit(resource),
        )?;
        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.variant_fields.capacity();
        building
            .variant_fields
            .try_reserve(fields.len())
            .map_err(|error| {
                Self::infrastructure_failure(
                    format!("could not reserve HIR variant fields: {error}"),
                    span,
                )
            })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.variant_fields.capacity() > old_capacity {
            building.accounting.growths[3] += 1;
        }
        building.variant_fields.extend_from_slice(fields);
        Ok(range)
    }

    pub(crate) fn append_map_entries(
        &mut self,
        entries: &[HirMapEntry],
        span: Option<SourceSpan>,
    ) -> Result<HirMapEntryRange, HirConstructionFailure> {
        let resource = HirCapacityResource::MapEntries;
        let range = self.checked_side_range(
            self.map_entry_slice().len(),
            entries.len(),
            resource,
            span,
            self.test_limit(resource),
        )?;
        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.map_entries.capacity();
        building
            .map_entries
            .try_reserve(entries.len())
            .map_err(|error| {
                Self::infrastructure_failure(
                    format!("could not reserve HIR map entries: {error}"),
                    span,
                )
            })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.map_entries.capacity() > old_capacity {
            building.accounting.growths[4] += 1;
        }
        building.map_entries.extend_from_slice(entries);
        Ok(range)
    }

    pub(crate) fn append_string_pieces(
        &mut self,
        pieces: &[ConstStringPiece],
        span: Option<SourceSpan>,
    ) -> Result<HirStringPieceRange, HirConstructionFailure> {
        let resource = HirCapacityResource::StringPieces;
        let range = self.checked_side_range(
            self.string_piece_slice().len(),
            pieces.len(),
            resource,
            span,
            self.test_limit(resource),
        )?;
        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.string_pieces.capacity();
        building
            .string_pieces
            .try_reserve(pieces.len())
            .map_err(|error| {
                Self::infrastructure_failure(
                    format!("could not reserve HIR string pieces: {error}"),
                    span,
                )
            })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.string_pieces.capacity() > old_capacity {
            building.accounting.growths[5] += 1;
        }
        building.string_pieces.extend_from_slice(pieces);
        Ok(range)
    }

    pub(crate) fn append_projections(
        &mut self,
        projections: &[HirProjection],
        span: Option<SourceSpan>,
    ) -> Result<HirProjectionRange, HirConstructionFailure> {
        let resource = HirCapacityResource::PlaceProjections;
        let range = self.checked_side_range(
            self.projection_slice().len(),
            projections.len(),
            resource,
            span,
            self.test_limit(resource),
        )?;
        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.projections.capacity();
        building
            .projections
            .try_reserve(projections.len())
            .map_err(|error| {
                Self::infrastructure_failure(
                    format!("could not reserve HIR place projections: {error}"),
                    span,
                )
            })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.projections.capacity() > old_capacity {
            building.accounting.growths[6] += 1;
        }
        building.projections.extend_from_slice(projections);
        Ok(range)
    }

    /// Copy a root-outward place path and append one checked projection.
    pub(crate) fn extend_projections(
        &mut self,
        prefix: HirProjectionRange,
        projection: HirProjection,
        span: Option<SourceSpan>,
    ) -> Result<HirProjectionRange, HirConstructionFailure> {
        let Some(prefix_end) = prefix.start.checked_add(prefix.length) else {
            return Err(Self::infrastructure_failure(
                "trusted HIR projection range overflowed".to_owned(),
                span,
            ));
        };
        if self.get_projections(prefix).is_none() {
            return Err(Self::infrastructure_failure(
                "trusted HIR projection range is outside its side store".to_owned(),
                span,
            ));
        }

        let Some(append_length) = prefix.len().checked_add(1) else {
            return Err(Self::capacity_failure(
                HirCapacityResource::PlaceProjections,
                span,
            ));
        };
        let resource = HirCapacityResource::PlaceProjections;
        let range = self.checked_side_range(
            self.projection_slice().len(),
            append_length,
            resource,
            span,
            self.test_limit(resource),
        )?;

        let building = self.building_mut(span)?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        let old_capacity = building.projections.capacity();
        building
            .projections
            .try_reserve(append_length)
            .map_err(|error| {
                Self::infrastructure_failure(
                    format!("could not reserve HIR place projections: {error}"),
                    span,
                )
            })?;
        #[cfg(any(test, feature = "benchmark_counters"))]
        if building.projections.capacity() > old_capacity {
            building.accounting.growths[6] += 1;
        }
        building
            .projections
            .extend_from_within(prefix.start as usize..prefix_end as usize);
        building.projections.push(projection);
        Ok(range)
    }

    /// Remap interned scalar identities in rows and side payloads without changing graph topology.
    pub(crate) fn remap_string_ids(
        &mut self,
        remap: &crate::compiler_frontend::symbols::string_interning::StringIdRemap,
    ) {
        match &mut self.storage {
            HirExpressionStoreStorage::Building(store) => {
                remap_side_payloads(&mut store.variant_fields, &mut store.string_pieces, remap);
            }
            HirExpressionStoreStorage::Frozen(store) => {
                remap_side_payloads(&mut store.variant_fields, &mut store.string_pieces, remap);
            }
        }
    }

    #[cfg(any(test, feature = "benchmark_counters"))]
    pub fn measurements(&self) -> HirExpressionStoreMeasurements {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => HirExpressionStoreMeasurements {
                rows: buffer_measurements(
                    store.rows.len(),
                    store.rows.capacity(),
                    store.accounting.initial[0],
                    store.accounting.growths[0],
                ),
                values: buffer_measurements(
                    store.values.len(),
                    store.values.capacity(),
                    store.accounting.initial[1],
                    store.accounting.growths[1],
                ),
                struct_fields: buffer_measurements(
                    store.struct_fields.len(),
                    store.struct_fields.capacity(),
                    store.accounting.initial[2],
                    store.accounting.growths[2],
                ),
                variant_fields: buffer_measurements(
                    store.variant_fields.len(),
                    store.variant_fields.capacity(),
                    store.accounting.initial[3],
                    store.accounting.growths[3],
                ),
                map_entries: buffer_measurements(
                    store.map_entries.len(),
                    store.map_entries.capacity(),
                    store.accounting.initial[4],
                    store.accounting.growths[4],
                ),
                string_pieces: buffer_measurements(
                    store.string_pieces.len(),
                    store.string_pieces.capacity(),
                    store.accounting.initial[5],
                    store.accounting.growths[5],
                ),
                projections: buffer_measurements(
                    store.projections.len(),
                    store.projections.capacity(),
                    store.accounting.initial[6],
                    store.accounting.growths[6],
                ),
            },
            HirExpressionStoreStorage::Frozen(store) => HirExpressionStoreMeasurements {
                rows: buffer_measurements(
                    store.rows.len(),
                    store.rows.len(),
                    store.accounting.initial[0],
                    store.accounting.growths[0],
                ),
                values: buffer_measurements(
                    store.values.len(),
                    store.values.len(),
                    store.accounting.initial[1],
                    store.accounting.growths[1],
                ),
                struct_fields: buffer_measurements(
                    store.struct_fields.len(),
                    store.struct_fields.len(),
                    store.accounting.initial[2],
                    store.accounting.growths[2],
                ),
                variant_fields: buffer_measurements(
                    store.variant_fields.len(),
                    store.variant_fields.len(),
                    store.accounting.initial[3],
                    store.accounting.growths[3],
                ),
                map_entries: buffer_measurements(
                    store.map_entries.len(),
                    store.map_entries.len(),
                    store.accounting.initial[4],
                    store.accounting.growths[4],
                ),
                string_pieces: buffer_measurements(
                    store.string_pieces.len(),
                    store.string_pieces.len(),
                    store.accounting.initial[5],
                    store.accounting.growths[5],
                ),
                projections: buffer_measurements(
                    store.projections.len(),
                    store.projections.len(),
                    store.accounting.initial[6],
                    store.accounting.growths[6],
                ),
            },
        }
    }

    #[cfg(feature = "benchmark_counters")]
    fn record_freeze_measurements(&self, construction: HirExpressionStoreMeasurements) {
        let frozen = self.measurements();
        add_frontend_counter(FrontendCounter::HirExpressionCount, frozen.rows.len);
        add_frontend_counter(
            FrontendCounter::HirExpressionInitialCapacity,
            frozen.rows.initial_capacity,
        );
        add_frontend_counter(
            FrontendCounter::HirExpressionGrowthEvents,
            frozen.rows.growth_events,
        );
        add_frontend_counter(
            FrontendCounter::HirExpressionConstructionCapacity,
            construction.rows.retained_capacity,
        );
        add_frontend_counter(
            FrontendCounter::HirExpressionFrozenCapacity,
            frozen.rows.retained_capacity,
        );
        add_frontend_counter(FrontendCounter::HirValueItemCount, frozen.values.len);
        add_frontend_counter(
            FrontendCounter::HirValueItemInitialCapacity,
            frozen.values.initial_capacity,
        );
        add_frontend_counter(
            FrontendCounter::HirValueItemGrowthEvents,
            frozen.values.growth_events,
        );
        add_frontend_counter(
            FrontendCounter::HirValueItemConstructionCapacity,
            construction.values.retained_capacity,
        );
        add_frontend_counter(
            FrontendCounter::HirValueItemFrozenCapacity,
            frozen.values.retained_capacity,
        );

        let side_growths = [
            frozen.struct_fields.growth_events,
            frozen.variant_fields.growth_events,
            frozen.map_entries.growth_events,
            frozen.string_pieces.growth_events,
            frozen.projections.growth_events,
        ]
        .into_iter()
        .sum();
        add_frontend_counter(FrontendCounter::HirSideStoreGrowthEvents, side_growths);
        add_frontend_counter(
            FrontendCounter::HirStoreConstructionBytes,
            store_bytes(construction),
        );
        add_frontend_counter(FrontendCounter::HirStoreFrozenBytes, store_bytes(frozen));
    }

    fn checked_side_range<R>(
        &self,
        current_len: usize,
        append_len: usize,
        resource: HirCapacityResource,
        span: Option<SourceSpan>,
        test_limit: Option<usize>,
    ) -> Result<R, HirConstructionFailure>
    where
        R: FromCheckedRange,
    {
        if self.is_frozen() {
            return Err(Self::infrastructure_failure(
                "attempted to append to a frozen HIR expression store".to_owned(),
                span,
            ));
        }
        let Some(end) = current_len.checked_add(append_len) else {
            return Err(Self::capacity_failure(resource, span));
        };
        let Some(start) = u32::try_from(current_len).ok() else {
            return Err(Self::capacity_failure(resource, span));
        };
        let Some(length) = u32::try_from(append_len).ok() else {
            return Err(Self::capacity_failure(resource, span));
        };
        let Some(checked_end) = start.checked_add(length) else {
            return Err(Self::capacity_failure(resource, span));
        };
        if checked_end as usize != end || test_limit.is_some_and(|maximum| end > maximum) {
            return Err(Self::capacity_failure(resource, span));
        }

        Ok(R::from_checked_parts(start, length))
    }

    fn building_mut(
        &mut self,
        span: Option<SourceSpan>,
    ) -> Result<&mut BuildingHirExpressionStore, HirConstructionFailure> {
        match &mut self.storage {
            HirExpressionStoreStorage::Building(store) => Ok(store),
            HirExpressionStoreStorage::Frozen(_) => Err(Self::infrastructure_failure(
                "attempted to append to a frozen HIR expression store".to_owned(),
                span,
            )),
        }
    }

    fn capacity_failure(
        resource: HirCapacityResource,
        span: Option<SourceSpan>,
    ) -> HirConstructionFailure {
        CompilerDiagnostic::compiler_capacity_exceeded(resource, span).into()
    }

    fn infrastructure_failure(message: String, span: Option<SourceSpan>) -> HirConstructionFailure {
        CompilerError::new(message, span, ErrorType::Compiler).into()
    }

    fn row_slice(&self) -> &[HirExpression] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.rows,
            HirExpressionStoreStorage::Frozen(store) => &store.rows,
        }
    }

    fn value_slice(&self) -> &[HirValueId] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.values,
            HirExpressionStoreStorage::Frozen(store) => &store.values,
        }
    }

    fn struct_field_slice(&self) -> &[(FieldId, HirValueId)] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.struct_fields,
            HirExpressionStoreStorage::Frozen(store) => &store.struct_fields,
        }
    }

    fn variant_field_slice(&self) -> &[HirVariantField] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.variant_fields,
            HirExpressionStoreStorage::Frozen(store) => &store.variant_fields,
        }
    }

    fn map_entry_slice(&self) -> &[HirMapEntry] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.map_entries,
            HirExpressionStoreStorage::Frozen(store) => &store.map_entries,
        }
    }

    fn string_piece_slice(&self) -> &[ConstStringPiece] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.string_pieces,
            HirExpressionStoreStorage::Frozen(store) => &store.string_pieces,
        }
    }

    fn projection_slice(&self) -> &[HirProjection] {
        match &self.storage {
            HirExpressionStoreStorage::Building(store) => &store.projections,
            HirExpressionStoreStorage::Frozen(store) => &store.projections,
        }
    }

    fn test_limit(&self, resource: HirCapacityResource) -> Option<usize> {
        #[cfg(test)]
        {
            self.test_limits.map(|limits| match resource {
                HirCapacityResource::ExpressionRows => limits.rows,
                HirCapacityResource::ValueEdges => limits.values,
                HirCapacityResource::StructFields => limits.struct_fields,
                HirCapacityResource::VariantFields => limits.variant_fields,
                HirCapacityResource::MapEntries => limits.map_entries,
                HirCapacityResource::StringPieces => limits.string_pieces,
                HirCapacityResource::PlaceProjections => limits.projections,
            })
        }
        #[cfg(not(test))]
        {
            let _ = resource;
            None
        }
    }
}

trait FromCheckedRange: Sized {
    fn from_checked_parts(start: u32, length: u32) -> Self;
}

macro_rules! impl_from_checked_range {
    ($($range:ty),+ $(,)?) => {
        $(
            impl FromCheckedRange for $range {
                fn from_checked_parts(start: u32, length: u32) -> Self {
                    Self::from_checked_parts(start, length)
                }
            }
        )+
    };
}

impl_from_checked_range!(
    HirValueRange,
    HirStructFieldRange,
    HirVariantFieldRange,
    HirMapEntryRange,
    HirStringPieceRange,
    HirProjectionRange,
);

fn slice_for_range<T>(slice: &[T], range: impl RangeBounds) -> Option<&[T]> {
    let start = range.start();
    let end = start.checked_add(range.length())?;
    slice.get(start as usize..end as usize)
}

trait RangeBounds {
    fn start(&self) -> u32;
    fn length(&self) -> u32;
}

macro_rules! impl_range_bounds {
    ($($range:ty),+ $(,)?) => {
        $(
            impl RangeBounds for $range {
                fn start(&self) -> u32 { self.start }
                fn length(&self) -> u32 { self.length }
            }
        )+
    };
}

impl_range_bounds!(
    HirValueRange,
    HirStructFieldRange,
    HirVariantFieldRange,
    HirMapEntryRange,
    HirStringPieceRange,
    HirProjectionRange,
);

#[cfg(any(test, feature = "benchmark_counters"))]
fn buffer_measurements(
    len: usize,
    retained_capacity: usize,
    initial_capacity: usize,
    growth_events: usize,
) -> HirExpressionBufferMeasurements {
    HirExpressionBufferMeasurements {
        len,
        retained_capacity,
        initial_capacity,
        growth_events,
    }
}

#[cfg(feature = "benchmark_counters")]
fn store_bytes(measurements: HirExpressionStoreMeasurements) -> usize {
    // These are inline row and side-store bytes. Nested String and NumberValue heaps are excluded.
    [
        measurements
            .rows
            .retained_capacity
            .saturating_mul(std::mem::size_of::<HirExpression>()),
        measurements
            .values
            .retained_capacity
            .saturating_mul(std::mem::size_of::<HirValueId>()),
        measurements
            .struct_fields
            .retained_capacity
            .saturating_mul(std::mem::size_of::<(FieldId, HirValueId)>()),
        measurements
            .variant_fields
            .retained_capacity
            .saturating_mul(std::mem::size_of::<HirVariantField>()),
        measurements
            .map_entries
            .retained_capacity
            .saturating_mul(std::mem::size_of::<HirMapEntry>()),
        measurements
            .string_pieces
            .retained_capacity
            .saturating_mul(std::mem::size_of::<ConstStringPiece>()),
        measurements
            .projections
            .retained_capacity
            .saturating_mul(std::mem::size_of::<HirProjection>()),
    ]
    .into_iter()
    .fold(0usize, usize::saturating_add)
}

fn remap_side_payloads(
    variant_fields: &mut [HirVariantField],
    string_pieces: &mut [ConstStringPiece],
    remap: &crate::compiler_frontend::symbols::string_interning::StringIdRemap,
) {
    for field in variant_fields {
        if let Some(name) = &mut field.name {
            *name = remap.get(*name);
        }
    }
    for piece in string_pieces {
        if let ConstStringPiece::Text(string_id) = piece {
            *string_id = remap.get(*string_id);
        }
    }
}

#[cfg(test)]
#[path = "tests/hir_expression_store_tests.rs"]
mod tests;
