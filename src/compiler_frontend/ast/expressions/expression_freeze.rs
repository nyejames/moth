//! Consuming last-reader remapping from surviving completed roots into exact typed owners.
//! Typed side payloads are moved once, after their references and all retained roots are validated.
use super::*;

/// The finalizer supplies every retained executable root after releasing TIR and validation bodies.
pub(crate) trait ExpressionFreezeRoots {
    fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId));
    fn visit_expression_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut ExprId));
    fn visit_place_ids(&self, visitor: &mut dyn FnMut(PlaceId));
    fn visit_place_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut PlaceId));
}

/// A selected entry's dense index is assigned only after selection ends.
struct EntryRemap {
    indices: Vec<Option<u32>>,
}

impl EntryRemap {
    fn new(len: usize) -> Self {
        Self {
            indices: vec![None; len],
        }
    }

    fn select(&mut self, index: usize) -> Result<(), CompilerError> {
        let entry = self
            .indices
            .get_mut(index)
            .ok_or_else(|| invariant("freeze side ID outside owner"))?;
        *entry = Some(0);
        Ok(())
    }

    fn select_range(&mut self, start: u32, len: u32) -> Result<(), CompilerError> {
        let end = start
            .checked_add(len)
            .ok_or_else(|| invariant("freeze range overflow"))?;
        let entries = self
            .indices
            .get_mut(start as usize..end as usize)
            .ok_or_else(|| invariant("freeze range outside owner"))?;
        entries.fill(Some(0));
        Ok(())
    }

    fn assign(&mut self) {
        let mut next = 0;
        for entry in &mut self.indices {
            if entry.is_some() {
                *entry = Some(next);
                next += 1;
            }
        }
    }

    fn index(&self, index: usize) -> Result<u32, CompilerError> {
        self.indices
            .get(index)
            .copied()
            .flatten()
            .ok_or_else(|| invariant("freeze reference to omitted entry"))
    }

    fn range_start(&self, start: u32, len: u32) -> Result<u32, CompilerError> {
        if len == 0 {
            return Ok(0);
        }
        let first = self.index(start as usize)?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| invariant("freeze range overflow"))?;
        for index in start..end {
            if self.index(index as usize)? != first + index - start {
                return Err(invariant("freeze selected range is not contiguous"));
            }
        }
        Ok(first)
    }

    fn retain<T>(&self, mut values: Vec<T>) -> Vec<T> {
        let mut index = 0;
        values.retain(|_| {
            let selected = self.indices[index].is_some();
            index += 1;
            selected
        });
        values
    }
}

macro_rules! side_remaps {
    ($($field:ident),* $(,)?) => {
        struct FreezeRemap { rows: EntryRemap, $($field: EntryRemap,)* }
        impl FreezeRemap {
            fn new(storage: &ExpressionStore) -> Self {
                Self { rows: EntryRemap::new(storage.rows.len()), $($field: EntryRemap::new(storage.$field.len()),)* }
            }
            fn assign(&mut self) { self.rows.assign(); $(self.$field.assign();)* }
            fn retain(self, storage: ExpressionStore) -> ExpressionStore {
                ExpressionStore { owner: storage.owner, rows: self.rows.retain(storage.rows), $($field: self.$field.retain(storage.$field),)* }
            }
        }
    };
}
side_remaps!(
    children,
    result_types,
    arguments,
    constructor_fields,
    map_entries,
    places,
    projections,
    calls,
    collection_calls,
    map_calls,
    numeric,
    comparisons,
    ranges,
    casts,
    handling,
    value_blocks,
    runtime_templates,
    runtime_slots,
    templates,
    failures,
    occurrences
);

impl FreezeRemap {
    fn expression(&self, id: ExprId) -> Result<ExprId, CompilerError> {
        Ok(ExprId::at(self.rows.index(id.index())? as usize))
    }

    fn place(&self, id: PlaceId) -> Result<PlaceId, CompilerError> {
        Ok(PlaceId::at(self.places.index(id.index())? as usize))
    }

    fn select_place(
        &mut self,
        storage: &ExpressionStore,
        id: PlaceId,
    ) -> Result<(), CompilerError> {
        self.places.select(id.index())?;
        let place = storage
            .places
            .get(id.index())
            .ok_or_else(|| invariant("freeze place root outside owner"))?;
        self.projections
            .select_range(place.projections.start, place.projections.len)
    }

    fn select_call(&mut self, storage: &ExpressionStore, id: CallId) -> Result<(), CompilerError> {
        self.calls.select(id.index())?;
        let call = storage
            .calls
            .get(id.index())
            .ok_or_else(|| invariant("freeze call side ID outside owner"))?;
        self.arguments
            .select_range(call.arguments.start, call.arguments.len)?;
        if let Some(handling) = call.handling {
            self.handling.select(handling.index())?;
        }
        Ok(())
    }

    fn select_row_sides(
        &mut self,
        storage: &ExpressionStore,
        row: &ExpressionRow,
    ) -> Result<(), CompilerError> {
        if let SuccessShape::Multiple(range) = row.success {
            self.result_types.select_range(range.start, range.len)?;
        }
        if let Some(id) = row.failure {
            self.failures.select(id.index())?;
        }
        if let Some(id) = row.occurrence {
            self.occurrences.select(id.index())?;
        }
        match row.operation {
            ExpressionOp::Constant(_)
            | ExpressionOp::Reference(_)
            | ExpressionOp::FieldAccess { .. }
            | ExpressionOp::LogicalNot { .. }
            | ExpressionOp::LogicalBinary { .. }
            | ExpressionOp::PropagateOption { .. }
            | ExpressionOp::Coerce { .. } => {}
            ExpressionOp::Copy(id) => self.select_place(storage, id)?,
            ExpressionOp::NumericUnary { facts, .. }
            | ExpressionOp::NumericBinary { facts, .. } => self.numeric.select(facts.index())?,
            ExpressionOp::Compare { facts, .. } => self.comparisons.select(facts.index())?,
            ExpressionOp::Range { facts, .. } => self.ranges.select(facts.index())?,
            ExpressionOp::Call(id) | ExpressionOp::MethodCall { call: id, .. } => {
                self.select_call(storage, id)?
            }
            ExpressionOp::CollectionBuiltin { call, .. } => {
                let range = storage
                    .collection_calls
                    .get(call.index())
                    .ok_or_else(|| invariant("freeze collection side ID outside owner"))?
                    .arguments;
                self.arguments.select_range(range.start, range.len)?;
                self.collection_calls.select(call.index())?;
            }
            ExpressionOp::MapBuiltin { call, .. } => {
                let range = storage
                    .map_calls
                    .get(call.index())
                    .ok_or_else(|| invariant("freeze map side ID outside owner"))?
                    .arguments;
                self.arguments.select_range(range.start, range.len)?;
                self.map_calls.select(call.index())?;
            }
            ExpressionOp::Cast { facts, .. } => self.casts.select(facts.index())?,
            ExpressionOp::HandleFallible { handling, .. } => {
                self.handling.select(handling.index())?
            }
            ExpressionOp::Collection(range) => {
                self.children.select_range(range.start, range.len)?
            }
            ExpressionOp::Map(range) => self.map_entries.select_range(range.start, range.len)?,
            ExpressionOp::Struct { fields } | ExpressionOp::Choice { fields, .. } => self
                .constructor_fields
                .select_range(fields.start, fields.len)?,
            ExpressionOp::ValueBlock(id) => {
                storage
                    .value_blocks
                    .get(id.index())
                    .ok_or_else(|| invariant("freeze value-block side ID outside owner"))?;
                self.value_blocks.select(id.index())?;
                let mut result = Ok(());
                storage.value_blocks[id.index()].visit_place_ids(&mut |place| {
                    if result.is_ok() {
                        result = self.select_place(storage, place);
                    }
                });
                result?;
            }
            ExpressionOp::RuntimeTemplate(id) => self.runtime_templates.select(id.index())?,
            ExpressionOp::RuntimeSlotApplication(id) => self.runtime_slots.select(id.index())?,
            ExpressionOp::Template(_) => {
                return Err(invariant("build-only Template selected for freeze"));
            }
        }
        Ok(())
    }
    fn rewrite_row(&self, row: &mut ExpressionRow) -> Result<(), CompilerError> {
        macro_rules! side {
            ($field:ident, $id:expr, $ty:ident) => {
                *$id = $ty::at(self.$field.index($id.index())? as usize)
            };
        }
        macro_rules! range {
            ($field:ident, $range:expr) => {
                $range.start = self.$field.range_start($range.start, $range.len)?
            };
        }
        if let SuccessShape::Multiple(range) = &mut row.success {
            range!(result_types, range);
        }
        if let Some(id) = &mut row.failure {
            side!(failures, id, FailureFactId);
        }
        if let Some(id) = &mut row.occurrence {
            side!(occurrences, id, OccurrenceFactId);
        }
        match &mut row.operation {
            ExpressionOp::Constant(_) | ExpressionOp::Reference(_) => {}
            ExpressionOp::Copy(id) => side!(places, id, PlaceId),
            ExpressionOp::FieldAccess { base, .. } => *base = self.expression(*base)?,
            ExpressionOp::NumericUnary { operand, facts } => {
                *operand = self.expression(*operand)?;
                side!(numeric, facts, NumericOperationId);
            }
            ExpressionOp::NumericBinary { left, right, facts } => {
                *left = self.expression(*left)?;
                *right = self.expression(*right)?;
                side!(numeric, facts, NumericOperationId);
            }
            ExpressionOp::LogicalNot { operand } => *operand = self.expression(*operand)?,
            ExpressionOp::LogicalBinary { left, right, .. } => {
                *left = self.expression(*left)?;
                *right = self.expression(*right)?;
            }
            ExpressionOp::Compare { left, right, facts } => {
                *left = self.expression(*left)?;
                *right = self.expression(*right)?;
                side!(comparisons, facts, ComparisonOperationId);
            }
            ExpressionOp::Range { start, end, facts } => {
                *start = self.expression(*start)?;
                *end = self.expression(*end)?;
                side!(ranges, facts, RangeOperationId);
            }
            ExpressionOp::Call(id) => side!(calls, id, CallId),
            ExpressionOp::MethodCall { receiver, call } => {
                *receiver = self.expression(*receiver)?;
                side!(calls, call, CallId);
            }
            ExpressionOp::CollectionBuiltin { receiver, call } => {
                *receiver = self.expression(*receiver)?;
                side!(collection_calls, call, CollectionCallId);
            }
            ExpressionOp::MapBuiltin { receiver, call } => {
                *receiver = self.expression(*receiver)?;
                side!(map_calls, call, MapCallId);
            }
            ExpressionOp::Cast { source, facts } => {
                *source = self.expression(*source)?;
                side!(casts, facts, CastFactId);
            }
            ExpressionOp::HandleFallible { value, handling } => {
                *value = self.expression(*value)?;
                side!(handling, handling, HandlingId);
            }
            ExpressionOp::PropagateOption { value } | ExpressionOp::Coerce { value, .. } => {
                *value = self.expression(*value)?
            }
            ExpressionOp::Collection(values) => range!(children, values),
            ExpressionOp::Map(values) => range!(map_entries, values),
            ExpressionOp::Struct { fields } | ExpressionOp::Choice { fields, .. } => {
                range!(constructor_fields, fields)
            }
            ExpressionOp::ValueBlock(id) => side!(value_blocks, id, ValueBlockId),
            ExpressionOp::RuntimeTemplate(id) => side!(runtime_templates, id, RuntimeTemplateId),
            ExpressionOp::RuntimeSlotApplication(id) => {
                side!(runtime_slots, id, RuntimeSlotApplicationId)
            }
            ExpressionOp::Template(_) => {
                return Err(invariant("build-only Template survives expression freeze"));
            }
        }
        Ok(())
    }

    fn rewrite_sides(&self, storage: &mut ExpressionStore) -> Result<(), CompilerError> {
        macro_rules! selected { ($field:ident, $value:ident, $body:block) => {
            for (index, $value) in storage.$field.iter_mut().enumerate() {
                if self.$field.indices[index].is_some() $body
            }
        }; }
        selected!(children, value, {
            *value = self.expression(*value)?;
        });
        selected!(arguments, value, {
            value.value = self.expression(value.value)?;
        });
        selected!(constructor_fields, value, {
            value.value = self.expression(value.value)?;
        });
        selected!(map_entries, value, {
            value.key = self.expression(value.key)?;
            value.value = self.expression(value.value)?;
        });
        selected!(projections, value, {
            if let PlaceProjection::Index { index, .. } = value {
                *index = self.expression(*index)?;
            }
        });
        selected!(value_blocks, value, {
            rewrite_expression_leaves(self, |visitor| value.visit_expression_ids_mut(visitor))?;
            rewrite_place_leaves(self, |visitor| value.visit_place_ids_mut(visitor))?;
        });
        selected!(runtime_templates, value, {
            rewrite_expression_leaves(self, |visitor| value.visit_expression_ids_mut(visitor))?;
        });
        selected!(runtime_slots, value, {
            rewrite_expression_leaves(self, |visitor| value.visit_expression_ids_mut(visitor))?;
        });
        selected!(places, value, {
            value.projections.start = self
                .projections
                .range_start(value.projections.start, value.projections.len)?;
        });
        selected!(calls, value, {
            value.arguments.start = self
                .arguments
                .range_start(value.arguments.start, value.arguments.len)?;
            if let Some(id) = &mut value.handling {
                *id = HandlingId::at(self.handling.index(id.index())? as usize);
            }
        });
        selected!(collection_calls, value, {
            value.arguments.start = self
                .arguments
                .range_start(value.arguments.start, value.arguments.len)?;
        });
        selected!(map_calls, value, {
            value.arguments.start = self
                .arguments
                .range_start(value.arguments.start, value.arguments.len)?;
        });
        Ok(())
    }
}

fn rewrite_expression_leaves(
    remap: &FreezeRemap,
    visit: impl FnOnce(&mut dyn FnMut(&mut ExprId)),
) -> Result<(), CompilerError> {
    let mut result = Ok(());
    visit(&mut |id| {
        if result.is_ok() {
            match remap.expression(*id) {
                Ok(mapped) => *id = mapped,
                Err(error) => result = Err(error),
            }
        }
    });
    result
}

fn rewrite_place_leaves(
    remap: &FreezeRemap,
    visit: impl FnOnce(&mut dyn FnMut(&mut PlaceId)),
) -> Result<(), CompilerError> {
    let mut result = Ok(());
    visit(&mut |id| {
        if result.is_ok() {
            match remap.place(*id) {
                Ok(mapped) => *id = mapped,
                Err(error) => result = Err(error),
            }
        }
    });
    result
}

impl AstExpressionBuilder {
    /// Consume the builder after TIR's final reader, keeping exactly reachable entries.
    ///
    /// This is the single consuming remapper. Roots and all range/side edges are remapped once.
    /// Any build-only template reachable from a root is a compiler invariant failure.
    pub(crate) fn freeze(
        self,
        roots: &mut impl ExpressionFreezeRoots,
    ) -> Result<ExpressionStore, CompilerError> {
        let mut storage = self.storage;
        let mut remap = FreezeRemap::new(&storage);
        let mut pending = Vec::new();
        roots.visit_expression_ids(&mut |id| pending.push(id));
        // Assignment places are roots even when no Copy operation owns their projections.
        let mut result = Ok(());
        roots.visit_place_ids(&mut |id| {
            if result.is_ok() {
                result = remap.select_place(&storage, id);
                if result.is_ok() {
                    match storage.places.get(id.index()) {
                        Some(place) => match place.projections.bounds(storage.projections.len()) {
                            Ok(bounds) => {
                                for projection in &storage.projections[bounds] {
                                    if let PlaceProjection::Index { index, .. } = projection {
                                        pending.push(*index);
                                    }
                                }
                            }
                            Err(error) => result = Err(error),
                        },
                        None => result = Err(invariant("freeze place root outside owner")),
                    }
                }
            }
        });
        result?;

        // Iterative reachability keeps deeply nested expression graphs off the Rust call stack.
        while let Some(id) = pending.pop() {
            if storage.rows.get(id.index()).is_none() {
                return Err(invariant("freeze expression root outside owner"));
            }
            if remap.rows.indices[id.index()].is_some() {
                continue;
            }
            if matches!(
                storage.rows[id.index()].operation,
                ExpressionOp::Template(_)
            ) {
                return Err(invariant(
                    "build-only Template reachable from completed root",
                ));
            }
            remap.rows.select(id.index())?;
            // Validate row and side ownership before indexing typed side payloads or children.
            storage.view(id)?;
            remap.select_row_sides(&storage, &storage.rows[id.index()])?;
            storage.visit_operation_edges(storage.rows[id.index()].operation, &mut |child| {
                pending.push(child)
            })?;
        }
        remap.assign();
        #[cfg(debug_assertions)]
        let parity = FreezeParity::capture(&storage, &remap, roots)?;

        // Range starts are rewritten after all dense indices are assigned, before compaction.
        for (index, row) in storage.rows.iter_mut().enumerate() {
            if remap.rows.indices[index].is_some() {
                remap.rewrite_row(row)?;
            }
        }
        remap.rewrite_sides(&mut storage)?;
        rewrite_expression_leaves(&remap, |visitor| roots.visit_expression_ids_mut(visitor))?;
        rewrite_place_leaves(&remap, |visitor| roots.visit_place_ids_mut(visitor))?;
        let frozen = remap.retain(storage).freeze()?;
        #[cfg(debug_assertions)]
        parity.check(&frozen, roots)?;
        Ok(frozen)
    }
}

/// Debug/test coverage detects missed mutable root visits and repeated side remapping.
#[cfg(debug_assertions)]
struct FreezeParity {
    roots: Vec<ExprId>,
    places: Vec<PlaceId>,
    edges: Vec<Vec<ExprId>>,
    success_types: Vec<Vec<TypeId>>,
}

#[cfg(debug_assertions)]
impl FreezeParity {
    fn capture(
        storage: &ExpressionStore,
        remap: &FreezeRemap,
        roots: &impl ExpressionFreezeRoots,
    ) -> Result<Self, CompilerError> {
        let mut snapshot = Self {
            roots: Vec::new(),
            places: Vec::new(),
            edges: Vec::new(),
            success_types: Vec::new(),
        };
        let mut result = Ok(());
        roots.visit_expression_ids(&mut |id| match remap.expression(id) {
            Ok(id) => snapshot.roots.push(id),
            Err(error) => result = Err(error),
        });
        roots.visit_place_ids(&mut |id| match remap.place(id) {
            Ok(id) => snapshot.places.push(id),
            Err(error) => result = Err(error),
        });
        result?;
        for (index, row) in storage.rows.iter().enumerate() {
            if remap.rows.indices[index].is_none() {
                continue;
            }
            let mut edges = Vec::new();
            let mut result = Ok(());
            storage.visit_operation_edges(row.operation, &mut |id| match remap.expression(id) {
                Ok(id) => edges.push(id),
                Err(error) => result = Err(error),
            })?;
            result?;
            snapshot.edges.push(edges);
            snapshot
                .success_types
                .push(storage.view(ExprId::at(index))?.success_types().to_vec());
        }
        Ok(snapshot)
    }

    fn check(
        self,
        store: &ExpressionStore,
        roots: &impl ExpressionFreezeRoots,
    ) -> Result<(), CompilerError> {
        let mut expressions = Vec::new();
        let mut places = Vec::new();
        roots.visit_expression_ids(&mut |id| expressions.push(id));
        roots.visit_place_ids(&mut |id| places.push(id));
        if expressions != self.roots || places != self.places || store.len() != self.edges.len() {
            return Err(invariant("freeze root visitor/remapping parity failure"));
        }
        for (index, expected) in self.edges.into_iter().enumerate() {
            let mut actual = Vec::new();
            store.visit_operation_edges(store.rows[index].operation, &mut |id| actual.push(id))?;
            if actual != expected
                || store.view(ExprId::at(index))?.success_types() != self.success_types[index]
            {
                return Err(invariant("freeze expression graph parity failure"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/expression_freeze_tests.rs"]
mod tests;
