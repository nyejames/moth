//! Attempt-local cells and aggregate allocation identities for const execution.
//!
//! A frame names cells or aliases of places. Edges name stored values, not the
//! cell from which a value came. The immutable store still owns payload shape
//! and scalar content; the live arena owns aggregate membership and permissions.

use super::budget::ExecutionBudget;
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashSet;
use std::num::NonZeroU32;

#[derive(Default)]
pub(super) struct LiveStorage {
    cells: Vec<CellTarget>,
    allocations: Vec<Allocation>,
    projections: Vec<Projection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CellId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct AllocationId(NonZeroU32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProjectionId(NonZeroU32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Permission {
    ReadOnly,
    Writable,
}

impl Permission {
    fn intersect(self, other: Self) -> Self {
        if self == Self::Writable { other } else { self }
    }
}

impl From<ValueMode> for Permission {
    fn from(mode: ValueMode) -> Self {
        if mode.is_mutable() {
            Self::Writable
        } else {
            Self::ReadOnly
        }
    }
}

/// The shape ID carries the exact type and occurrence provenance. Two different
/// shape IDs can name one allocation, and equal shapes need not alias.
#[derive(Clone, Copy, Debug)]
pub(super) struct LiveValue {
    pub(super) shape: ConstValueId,
    pub(super) allocation: Option<AllocationId>,
    pub(super) permission: Permission,
    projection: Option<ProjectionId>,
}

impl LiveValue {
    pub(super) fn leaf(shape: ConstValueId, permission: Permission) -> Self {
        Self {
            shape,
            allocation: None,
            permission,
            projection: None,
        }
    }

    pub(super) fn read_only(mut self) -> Self {
        self.permission = Permission::ReadOnly;
        self
    }

    /// A writable value stored by value names the allocation, not the place
    /// it was read from. A read-only reference keeps its access dependencies.
    fn into_slot(mut self, permission: Permission) -> Self {
        if self.permission == Permission::Writable && permission == Permission::Writable {
            self.projection = None;
        }
        self
    }
}

/// A binding created from a produced value can rebind its own slot. A mutable
/// alias instead writes through to its designated place.
#[derive(Clone, Copy)]
pub(super) enum LiveBinding {
    Slot(LivePlace),
    Alias(LivePlace),
}

impl LiveBinding {
    pub(super) fn place(self) -> LivePlace {
        match self {
            Self::Slot(place) | Self::Alias(place) => place,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct LivePlace {
    location: Location,
    pub(super) permission: Permission,
    ancestors: Option<ProjectionId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Location {
    Cell(CellId),
    Edge(EdgeLocation),
}

/// Struct indices retain their identity for an allocation's lifetime. Collection
/// structural edits conservatively expire all of that collection's element places.
#[derive(Clone, Copy, PartialEq, Eq)]
struct EdgeLocation {
    owner: AllocationId,
    index: u32,
    structure: u32,
}

enum CellTarget {
    Value(LiveValue),
    Alias(LivePlace),
}

struct Allocation {
    edges: Vec<Edge>,
    structure: u32,
}

struct Edge {
    value: LiveValue,
    revision: u32,
    current_projection: Option<ProjectionId>,
}

impl Edge {
    fn new(value: LiveValue) -> Self {
        Self {
            value: value.into_slot(Permission::Writable),
            revision: 0,
            current_projection: None,
        }
    }
}

#[derive(PartialEq, Eq)]
struct Projection {
    edge: EdgeLocation,
    revision: u32,
    ancestors: Option<ProjectionId>,
    retained: Option<ProjectionId>,
}

impl LiveStorage {
    pub(super) fn bind_slot(
        &mut self,
        value: LiveValue,
        permission: Permission,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveBinding, ConstantFoldError> {
        retain::<CellTarget>(1, budget)?;
        let cell = CellId(index(self.cells.len())?);
        self.cells
            .push(CellTarget::Value(value.into_slot(permission)));
        Ok(LiveBinding::Slot(LivePlace {
            location: Location::Cell(cell),
            permission,
            ancestors: None,
        }))
    }

    /// Change a slot binding's target in place so aliases of the cell keep observing it.
    pub(super) fn rebind_to_place(
        &mut self,
        target: LivePlace,
        source: LivePlace,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        let Location::Cell(cell) = target.location else {
            return Err(storage_invariant());
        };
        if self.resolve(source, budget)?.location == target.location {
            return Ok(false);
        }
        self.cells[cell.0 as usize] = CellTarget::Alias(source);
        Ok(true)
    }

    fn resolve(
        &self,
        mut place: LivePlace,
        budget: &mut ExecutionBudget,
    ) -> Result<LivePlace, ConstantFoldError> {
        while let Location::Cell(cell) = place.location {
            let CellTarget::Alias(next) = self.cells[cell.0 as usize] else {
                break;
            };
            budget.step()?;
            place = LivePlace {
                permission: place.permission.intersect(next.permission),
                ..next
            };
        }
        Ok(place)
    }

    pub(super) fn allocate(
        &mut self,
        shape: ConstValueId,
        children: impl ExactSizeIterator<Item = LiveValue>,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveValue, ConstantFoldError> {
        retain::<Allocation>(1, budget)?;
        retain::<Edge>(children.len(), budget)?;
        let allocation = AllocationId(nonzero_index(self.allocations.len())?);
        let edges = children.map(Edge::new).collect();
        self.allocations.push(Allocation {
            edges,
            structure: 0,
        });
        Ok(LiveValue {
            allocation: Some(allocation),
            ..LiveValue::leaf(shape, Permission::Writable)
        })
    }

    pub(super) fn children(
        &self,
        allocation: AllocationId,
    ) -> impl ExactSizeIterator<Item = LiveValue> + '_ {
        self.allocation(allocation)
            .edges
            .iter()
            .map(|edge| edge.value)
    }

    pub(super) fn project(
        &self,
        base: LiveValue,
        child: usize,
    ) -> Result<Option<LivePlace>, ConstantFoldError> {
        let Some(owner) = base.allocation else {
            return Ok(None);
        };
        let allocation = self.allocation(owner);
        if child >= allocation.edges.len() {
            return Ok(None);
        }
        let location = EdgeLocation {
            owner,
            index: index(child)?,
            structure: allocation.structure,
        };
        Ok(Some(LivePlace {
            location: Location::Edge(location),
            permission: base.permission,
            ancestors: base.projection,
        }))
    }

    /// The slot's permission controls replacement. Mutating its child additionally
    /// requires the capability retained from the value stored in that slot.
    pub(super) fn read(
        &mut self,
        place: LivePlace,
        budget: &mut ExecutionBudget,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        let place = self.resolve(place, budget)?;
        if !self.projection_is_live(place.ancestors, budget)? {
            return Ok(None);
        }
        let mut value = match place.location {
            Location::Cell(cell) => {
                let CellTarget::Value(value) = self.cells[cell.0 as usize] else {
                    return Err(storage_invariant());
                };
                if !self.validate_value(value, budget)? {
                    return Ok(None);
                }
                value
            }
            Location::Edge(location) => {
                let Some(edge) = self.edge(location) else {
                    return Ok(None);
                };
                let mut value = edge.value;
                if !self.validate_value(value, budget)? {
                    return Ok(None);
                }
                let projection = Projection {
                    edge: location,
                    revision: edge.revision,
                    ancestors: place.ancestors,
                    retained: value.projection,
                };
                // Repeated reads of the same place and ancestry share a
                // structural witness. Nothing about the value is cached.
                let id = if let Some(id) = edge.current_projection
                    && self.projections[id.0.get() as usize - 1] == projection
                {
                    id
                } else {
                    retain::<Projection>(1, budget)?;
                    let id = ProjectionId(nonzero_index(self.projections.len())?);
                    self.projections.push(projection);
                    self.allocation_mut(location.owner).edges[location.index as usize]
                        .current_projection = Some(id);
                    id
                };
                value.projection = Some(id);
                value
            }
        };
        value.permission = place.permission.intersect(value.permission);
        Ok(Some(value))
    }

    pub(super) fn place_is_live(
        &self,
        place: LivePlace,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        let place = self.resolve(place, budget)?;
        if !self.projection_is_live(place.ancestors, budget)? {
            return Ok(false);
        }
        Ok(match place.location {
            Location::Cell(_) => true,
            Location::Edge(location) => self.edge(location).is_some(),
        })
    }

    pub(super) fn validate_value(
        &self,
        value: LiveValue,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        self.projection_is_live(value.projection, budget)
    }

    pub(super) fn replace(
        &mut self,
        place: LivePlace,
        value: LiveValue,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        let place = self.resolve(place, budget)?;
        if place.permission == Permission::ReadOnly
            || !self.projection_is_live(place.ancestors, budget)?
        {
            return Ok(false);
        }
        let mut value = value.into_slot(Permission::Writable);
        match place.location {
            Location::Cell(cell) => self.cells[cell.0 as usize] = CellTarget::Value(value),
            Location::Edge(location) => {
                let Some(edge) = self.edge(location) else {
                    return Ok(false);
                };
                let same_allocation =
                    value.allocation.is_some() && value.allocation == edge.value.allocation;
                if !same_allocation && !self.can_store(location.owner, value, budget)? {
                    return Ok(false);
                }
                if let Some(origin) = value.projection {
                    let origin = &self.projections[origin.0.get() as usize - 1];
                    // A value read from this very edge retains its prior origin,
                    // not a dependency on the edge being installed now.
                    if origin.edge == location {
                        value.projection = origin.retained;
                    }
                }
                let edge = &mut self.allocation_mut(location.owner).edges[location.index as usize];
                // A mutable descendant still names the same place when the
                // containing edge retains the very same allocation.
                if !same_allocation {
                    edge.revision = next_revision(edge.revision)?;
                }
                edge.value = value;
            }
        }
        Ok(true)
    }

    pub(super) fn push(
        &mut self,
        allocation: AllocationId,
        value: LiveValue,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        if !self.can_store(allocation, value, budget)? {
            return Ok(false);
        }
        retain::<Edge>(1, budget)?;
        let collection = self.allocation_mut(allocation);
        collection.structure = next_revision(collection.structure)?;
        collection.edges.push(Edge::new(value));
        Ok(true)
    }

    /// The removed value keeps its existing allocation, but not a projection
    /// through the removed edge. Any independent retained edge remains valid.
    pub(super) fn remove(
        &mut self,
        allocation: AllocationId,
        index: usize,
        budget: &mut ExecutionBudget,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        let collection = self.allocation_mut(allocation);
        if index >= collection.edges.len() {
            return Ok(None);
        }
        let structure = next_revision(collection.structure)?;
        for _ in index + 1..collection.edges.len() {
            budget.step()?;
        }
        let removed = collection.edges.remove(index);
        collection.structure = structure;
        Ok(Some(removed.value))
    }

    fn projection_is_live(
        &self,
        projection: Option<ProjectionId>,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        budget.with_scratch(|budget, scratch| {
            let mut projection = projection;
            let mut retained = Vec::new();
            while let Some(id) = projection.or_else(|| retained.pop()) {
                budget.step()?;
                let row = &self.projections[id.0.get() as usize - 1];
                if self.edge(row.edge).map(|edge| edge.revision) != Some(row.revision) {
                    return Ok(false);
                }
                projection = row.ancestors.or(row.retained);
                if row.ancestors.is_some()
                    && let Some(origin) = row.retained
                {
                    budget.reserve_scratch(&mut retained, 1, scratch)?;
                    retained.push(origin);
                }
            }
            Ok(true)
        })
    }

    fn can_store(
        &self,
        owner: AllocationId,
        value: LiveValue,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        let Some(child) = value.allocation else {
            return Ok(true);
        };
        // The ordinary type surface already rejects recursive nominal structs.
        // Keep this guard at the edge commit so no future admitted type can
        // introduce a cyclic const graph or an unbounded traversal.
        budget.with_scratch(|budget, scratch| {
            let mut pending = Vec::new();
            let mut visited = FxHashSet::default();
            let mut next = Some(child);
            while let Some(allocation) = next {
                budget.step()?;
                if allocation == owner {
                    return Ok(false);
                }
                if !visited.contains(&allocation) {
                    // Allow for the hash table's control bytes and spare buckets.
                    budget.charge_scratch(scratch, 4 * std::mem::size_of::<AllocationId>())?;
                    visited.insert(allocation);
                    for value in self.children(allocation) {
                        budget.step()?;
                        if let Some(child) = value.allocation {
                            budget.reserve_scratch(&mut pending, 1, scratch)?;
                            pending.push(child);
                        }
                    }
                }
                next = pending.pop();
            }
            Ok(true)
        })
    }

    fn allocation(&self, id: AllocationId) -> &Allocation {
        &self.allocations[id.0.get() as usize - 1]
    }

    fn allocation_mut(&mut self, id: AllocationId) -> &mut Allocation {
        &mut self.allocations[id.0.get() as usize - 1]
    }

    fn edge(&self, location: EdgeLocation) -> Option<&Edge> {
        let allocation = self.allocation(location.owner);
        if allocation.structure != location.structure {
            return None;
        }
        allocation.edges.get(location.index as usize)
    }
}

fn retain<T>(count: usize, budget: &mut ExecutionBudget) -> Result<(), ConstantFoldError> {
    budget.retain_values(count, budget.span)?;
    budget.retain_memory_bytes(count.saturating_mul(std::mem::size_of::<T>()), budget.span)
}

fn index(len: usize) -> Result<u32, InfrastructureFailure> {
    // MAX_RETAINED_VALUES bounds cells, edges and paths well below this capacity.
    u32::try_from(len).map_err(|_| capacity_invariant())
}

fn nonzero_index(len: usize) -> Result<NonZeroU32, InfrastructureFailure> {
    NonZeroU32::new(next_revision(index(len)?)?).ok_or_else(capacity_invariant)
}

fn next_revision(revision: u32) -> Result<u32, InfrastructureFailure> {
    revision.checked_add(1).ok_or_else(capacity_invariant)
}

fn capacity_invariant() -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "const storage exceeded its bounded identity range",
    )
}

fn storage_invariant() -> ConstantFoldError {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "const binding cell did not resolve to the expected storage",
    )
    .into()
}
