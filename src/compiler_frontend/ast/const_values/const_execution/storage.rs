//! Attempt-local cells and aggregate allocation identities for const execution.
//!
//! A frame names cells or aliases of places. Edges name stored values, not the
//! cell from which a value came. The immutable store still owns payload shape
//! and scalar content; the live arena owns aggregate membership and permissions.
//! Generations track committed writes. Target existence and permission remain
//! separate from whether an older shared access can still observe that target.

use super::budget::ExecutionBudget;
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::{CompilerStage, InvalidConstAccessReason};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::num::NonZeroU32;

#[cfg(test)]
#[path = "tests/storage.rs"]
mod tests;

#[derive(Default)]
pub(super) struct LiveStorage {
    cells: Vec<Cell>,
    allocations: Vec<Allocation>,
    projections: Vec<Projection>,
    shared: Vec<SharedAccess>,
    sites: Vec<AccessSiteRecord>,
    site_ids: FxHashMap<AccessSite, SiteId>,
    epoch: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CellId(NonZeroU32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct AllocationId(NonZeroU32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ProjectionId(NonZeroU32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SharedAccessId(NonZeroU32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SiteId(NonZeroU32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct AccessSite {
    pub(super) span: Option<SourceSpan>,
    pub(super) body: Option<PathId>,
}

struct AccessSiteRecord {
    source: AccessSite,
    last_acquisition: Option<SharedAccessId>,
}
#[derive(Clone, Copy, Default)]
struct Stamp {
    epoch: u32,
    site: Option<SiteId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct AccessTarget {
    allocation: Option<AllocationId>,
    scalar: Option<CellId>,
    projection: Option<ProjectionId>,
}

struct SharedAccess {
    stamp: Stamp,
    target: AccessTarget,
}

pub(super) struct InvalidAccess {
    pub(super) reason: InvalidConstAccessReason,
    pub(super) acquired: Option<AccessSite>,
    pub(super) invalidated: Option<AccessSite>,
}

pub(super) enum StorageError {
    InvalidAccess(InvalidAccess),
    Execution(ConstantFoldError),
}

impl From<ConstantFoldError> for StorageError {
    fn from(error: ConstantFoldError) -> Self {
        Self::Execution(error)
    }
}

impl From<InfrastructureFailure> for StorageError {
    fn from(error: InfrastructureFailure) -> Self {
        Self::Execution(error.into())
    }
}

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
    scalar: Option<CellId>,
    shared: Option<SharedAccessId>,
}

impl LiveValue {
    pub(super) fn leaf(shape: ConstValueId, permission: Permission) -> Self {
        Self {
            shape,
            allocation: None,
            permission,
            projection: None,
            scalar: None,
            shared: None,
        }
    }

    fn access_target(self) -> AccessTarget {
        AccessTarget {
            allocation: self.allocation,
            scalar: self.scalar,
            projection: self.projection,
        }
    }

    pub(super) fn release_operand(&mut self) {
        if self.permission == Permission::Writable {
            self.shared = None;
        }
    }

    /// A writable value stored by value names the allocation, not the place
    /// it was read from. A read-only reference keeps its access dependencies.
    fn into_slot(mut self) -> Self {
        if self.permission == Permission::Writable {
            self.projection = None;
            self.scalar = None;
            self.shared = None;
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
    acquisition: Option<SiteId>,
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

struct Cell {
    target: CellTarget,
    write: Stamp,
}

enum CellTarget {
    Value(LiveValue),
    Alias(LivePlace),
}

struct Allocation {
    edges: Vec<Edge>,
    structure: u32,
    /// Reverse containment, one entry per retained child edge. This records
    /// allocation overlap, not the liveness or number of references.
    parents: Vec<AllocationId>,
    write: Stamp,
    structural_write: Stamp,
    /// A retained read-only edge can still observe a scalar cell or an
    /// originating projection outside the allocation graph.
    may_retain_origins: bool,
}

struct Edge {
    value: LiveValue,
    revision: u32,
    current_projection: Option<ProjectionId>,
    write: Stamp,
}

impl Edge {
    fn new(value: LiveValue) -> Self {
        Self {
            value: value.into_slot(),
            revision: 0,
            current_projection: None,
            write: Stamp::default(),
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

#[derive(Default)]
struct ProjectionWalk {
    since: Option<u32>,
    pending: Vec<ProjectionId>,
    visited: Option<FxHashSet<ProjectionId>>,
}

#[derive(Default)]
struct ContainmentChange {
    removed: Option<(AllocationId, usize)>,
    added: Option<AllocationId>,
}

impl LiveStorage {
    pub(super) fn bind_slot(
        &mut self,
        value: LiveValue,
        permission: Permission,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveBinding, ConstantFoldError> {
        let value = if permission == Permission::ReadOnly {
            self.share(value, site, budget)?
        } else {
            value.into_slot()
        };
        retain::<Cell>(1, budget)?;
        let cell = CellId(nonzero_index(self.cells.len())?);
        self.cells.push(Cell {
            target: CellTarget::Value(value),
            write: Stamp::default(),
        });
        Ok(LiveBinding::Slot(LivePlace {
            location: Location::Cell(cell),
            permission,
            ancestors: None,
            acquisition: None,
        }))
    }

    pub(super) fn acquire_alias(
        &mut self,
        mut place: LivePlace,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveBinding, ConstantFoldError> {
        place.acquisition = self.site_id(site, budget)?;
        Ok(LiveBinding::Alias(place))
    }

    /// Change a slot binding's target in place so aliases of the cell keep observing it.
    pub(super) fn rebind_to_place(
        &mut self,
        target: LivePlace,
        mut source: LivePlace,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        let Location::Cell(cell) = target.location else {
            return Err(storage_invariant());
        };
        let changed = self.resolve(source, budget)?.location != target.location;
        let stamp = self.prepare_stamp(site, budget)?;
        source.acquisition = stamp.site;
        self.epoch = stamp.epoch;
        let cell = &mut self.cells[cell.0.get() as usize - 1];
        cell.write = stamp;
        if changed {
            cell.target = CellTarget::Alias(source);
        }
        Ok(changed)
    }

    fn resolve(
        &self,
        mut place: LivePlace,
        budget: &mut ExecutionBudget,
    ) -> Result<LivePlace, ConstantFoldError> {
        while let Location::Cell(cell) = place.location {
            let CellTarget::Alias(next) = self.cell(cell).target else {
                break;
            };
            budget.step()?;
            place = LivePlace {
                permission: place.permission.intersect(next.permission),
                acquisition: place.acquisition.or(next.acquisition),
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
        let mut edges: Vec<Edge> = Vec::with_capacity(children.len());
        let mut may_retain_origins = false;
        for value in children {
            let link = budget.step().and_then(|()| {
                if let Some(child) = value.allocation {
                    self.add_parent(child, allocation, budget)?;
                }
                Ok(())
            });
            if let Err(error) = link {
                // This unpublished parent is the last entry for each edge
                // already linked, including repeated children.
                for edge in edges {
                    if let Some(child) = edge.value.allocation {
                        self.allocation_mut(child).parents.pop();
                    }
                }
                return Err(error);
            }
            let edge = Edge::new(value);
            may_retain_origins |= self.retains_origins(edge.value);
            edges.push(edge);
        }
        self.allocations.push(Allocation {
            edges,
            structure: 0,
            parents: Vec::new(),
            write: Stamp::default(),
            structural_write: Stamp::default(),
            may_retain_origins,
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
            acquisition: None,
        }))
    }

    /// The slot's permission controls replacement. Mutating its child additionally
    /// requires the capability retained from the value stored in that slot.
    pub(super) fn read(
        &mut self,
        place: LivePlace,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveValue, StorageError> {
        let place = self.check_place(place, budget)?;
        let mut value = match place.location {
            Location::Cell(cell) => {
                let CellTarget::Value(mut value) = self.cell(cell).target else {
                    return Err(storage_invariant().into());
                };
                self.validate_value(value, budget)?;
                if value.allocation.is_none()
                    && value.shared.is_none()
                    && place.permission == Permission::Writable
                {
                    value.scalar = Some(cell);
                }
                value
            }
            Location::Edge(location) => {
                let edge = self.edge(location).ok_or_else(storage_invariant)?;
                let mut value = edge.value;
                self.validate_value(value, budget)?;
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
        if value.permission == Permission::ReadOnly {
            value = self.share(value, site, budget)?;
        }
        Ok(value)
    }

    /// Validate target identity without requiring permission to mutate the stored value.
    pub(super) fn check_place(
        &self,
        place: LivePlace,
        budget: &mut ExecutionBudget,
    ) -> Result<LivePlace, StorageError> {
        let place = self.resolve(place, budget)?;
        if let Some(write) = self.expired_projection(place.ancestors, budget)? {
            return Err(self.invalid_access(
                InvalidConstAccessReason::ExpiredProjection,
                place.acquisition,
                write,
            ));
        }
        if let Location::Edge(location) = place.location
            && self.edge(location).is_none()
        {
            let write = self.allocation(location.owner).structural_write;
            return Err(self.invalid_access(
                InvalidConstAccessReason::ExpiredProjection,
                place.acquisition,
                write,
            ));
        }
        Ok(place)
    }

    pub(super) fn validate_value(
        &self,
        value: LiveValue,
        budget: &mut ExecutionBudget,
    ) -> Result<(), StorageError> {
        let Some(id) = value.shared else {
            return match self.expired_projection(value.projection, budget)? {
                Some(write) => Err(self.invalid_access(
                    InvalidConstAccessReason::ExpiredProjection,
                    None,
                    write,
                )),
                None => Ok(()),
            };
        };
        let access = self.shared_access(id);
        let after = access.stamp.epoch;
        if after == self.epoch {
            return Ok(());
        }
        budget.step()?;
        if let Some(write) = self.access_write(access.target, after, budget)? {
            return Err(self.invalid_access(
                InvalidConstAccessReason::InvalidatedReference,
                access.stamp.site,
                write,
            ));
        }
        Ok(())
    }

    /// Acquire access to an already validated target. Forwarding an identical
    /// reference retains its acquisition; deriving a new place records its own.
    pub(super) fn share(
        &mut self,
        mut value: LiveValue,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveValue, ConstantFoldError> {
        value.permission = Permission::ReadOnly;
        let target = value.access_target();
        if target.allocation.is_none() && target.scalar.is_none() && target.projection.is_none() {
            return Ok(value);
        }
        if value
            .shared
            .is_some_and(|id| self.shared_access(id).target == target)
        {
            return Ok(value);
        }
        let site = self.site_id(site, budget)?;
        let recent = site.and_then(|id| self.site(id).last_acquisition);
        if let Some(id) = recent {
            let access = self.shared_access(id);
            if access.target == target
                && (access.stamp.epoch == self.epoch
                    || self
                        .access_write(target, access.stamp.epoch, budget)?
                        .is_none())
            {
                value.shared = Some(id);
                return Ok(value);
            }
        }
        retain::<SharedAccess>(1, budget)?;
        let id = SharedAccessId(nonzero_index(self.shared.len())?);
        self.shared.push(SharedAccess {
            stamp: Stamp {
                epoch: self.epoch,
                site,
            },
            target,
        });
        if let Some(site) = site {
            self.sites[site.0.get() as usize - 1].last_acquisition = Some(id);
        }
        value.shared = Some(id);
        Ok(value)
    }

    /// A result operand remains observed until its whole producer finishes.
    /// Keep the transferred capability for the receiver, not for this read.
    pub(super) fn hold_operand(
        &mut self,
        value: LiveValue,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<LiveValue, ConstantFoldError> {
        if value.permission == Permission::ReadOnly {
            return Ok(value);
        }
        let mut held = self.share(value, site, budget)?;
        held.permission = value.permission;
        Ok(held)
    }

    fn invalid_access(
        &self,
        reason: InvalidConstAccessReason,
        acquired: Option<SiteId>,
        write: Stamp,
    ) -> StorageError {
        StorageError::InvalidAccess(InvalidAccess {
            reason,
            acquired: acquired.map(|site| self.site(site).source),
            invalidated: write.site.map(|site| self.site(site).source),
        })
    }

    pub(super) fn replace(
        &mut self,
        place: LivePlace,
        value: LiveValue,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, StorageError> {
        let place = self.check_place(place, budget)?;
        if place.permission == Permission::ReadOnly {
            return Ok(false);
        }
        let value = value.into_slot();
        match place.location {
            Location::Cell(cell) => {
                let stamp = self.prepare_stamp(site, budget)?;
                self.epoch = stamp.epoch;
                self.cells[cell.0.get() as usize - 1] = Cell {
                    target: CellTarget::Value(value),
                    write: stamp,
                };
            }
            Location::Edge(location) => {
                let edge = self.edge(location).ok_or_else(storage_invariant)?;
                let current = edge.value;
                let same_allocation =
                    value.allocation.is_some() && value.allocation == current.allocation;
                if !same_allocation && !self.can_store(location.owner, value, budget)? {
                    return Ok(false);
                }
                // Mutable descendants keep their place if the same allocation remains.
                let revision = if same_allocation {
                    edge.revision
                } else {
                    next_revision(edge.revision)?
                };
                let change = self.prepare_containment(
                    location.owner,
                    current.allocation,
                    value.allocation,
                    budget,
                )?;
                self.mutate(location.owner, site, budget, |storage, stamp| {
                    let retains_origins = storage.retains_origins(value);
                    storage.commit_containment(location.owner, change);
                    let allocation = storage.allocation_mut(location.owner);
                    allocation.may_retain_origins |= retains_origins;
                    let edge = &mut allocation.edges[location.index as usize];
                    edge.revision = revision;
                    edge.write = stamp;
                    edge.value = value;
                })?;
            }
        }
        Ok(true)
    }

    pub(super) fn push(
        &mut self,
        allocation: AllocationId,
        value: LiveValue,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<bool, ConstantFoldError> {
        let edge = Edge::new(value);
        if !self.can_store(allocation, value, budget)? {
            return Ok(false);
        }
        let edges = &mut self.allocation_mut(allocation).edges;
        if edges.len() == edges.capacity() {
            let target = (edges.len() + 1).next_power_of_two();
            retain::<Edge>(target - edges.capacity(), budget)?;
            edges.reserve_exact(target - edges.len());
        }
        let structure = next_revision(self.allocation(allocation).structure)?;
        let change = self.prepare_containment(allocation, None, value.allocation, budget)?;
        self.mutate(allocation, site, budget, |storage, stamp| {
            let retains_origins = storage.retains_origins(edge.value);
            storage.commit_containment(allocation, change);
            let collection = storage.allocation_mut(allocation);
            collection.may_retain_origins |= retains_origins;
            collection.structure = structure;
            collection.structural_write = stamp;
            collection.edges.push(edge);
        })?;
        Ok(true)
    }

    /// The removed value keeps its existing allocation, but not a projection
    /// through the removed edge. Any independent retained edge remains valid.
    pub(super) fn remove(
        &mut self,
        allocation: AllocationId,
        index: usize,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        let collection = self.allocation(allocation);
        let Some(edge) = collection.edges.get(index) else {
            return Ok(None);
        };
        let structure = next_revision(collection.structure)?;
        for _ in index + 1..collection.edges.len() {
            budget.step()?;
        }
        let change = self.prepare_containment(allocation, edge.value.allocation, None, budget)?;
        let removed = self.mutate(allocation, site, budget, |storage, stamp| {
            storage.commit_containment(allocation, change);
            let collection = storage.allocation_mut(allocation);
            collection.structure = structure;
            collection.structural_write = stamp;
            collection.edges.remove(index).value
        })?;
        Ok(Some(removed))
    }

    fn access_write(
        &self,
        target: AccessTarget,
        after: u32,
        budget: &mut ExecutionBudget,
    ) -> Result<Option<Stamp>, ConstantFoldError> {
        if let Some(allocation) = target.allocation {
            let write = self.allocation(allocation).write;
            if write.epoch > after {
                return Ok(Some(write));
            }
        }
        let retained = target
            .allocation
            .filter(|&allocation| self.allocation(allocation).may_retain_origins);
        budget.with_scratch(|budget, scratch| {
            let mut projections = ProjectionWalk {
                since: Some(after),
                visited: retained.map(|_| FxHashSet::default()),
                ..ProjectionWalk::default()
            };
            if let Some(write) =
                self.origin_write(target, after, &mut projections, budget, scratch)?
            {
                return Ok(Some(write));
            }
            match retained {
                Some(allocation) => {
                    self.retained_origin_write(allocation, after, &mut projections, budget, scratch)
                }
                None => Ok(None),
            }
        })
    }

    fn origin_write(
        &self,
        target: AccessTarget,
        after: u32,
        projections: &mut ProjectionWalk,
        budget: &mut ExecutionBudget,
        scratch: &mut usize,
    ) -> Result<Option<Stamp>, ConstantFoldError> {
        if let Some(cell) = target.scalar {
            let write = self.cell(cell).write;
            if write.epoch > after {
                return Ok(Some(write));
            }
        }
        self.walk_projection(target.projection, projections, budget, scratch)
    }

    fn retains_origins(&self, value: LiveValue) -> bool {
        value.shared.is_some() && (value.scalar.is_some() || value.projection.is_some())
            || value
                .allocation
                .is_some_and(|child| self.allocation(child).may_retain_origins)
    }

    /// Allocation stamps cover actual child mutations. Read-only stored
    /// references can also observe writes outside those containment edges.
    /// Consult their existing witnesses rather than tracking alias holders.
    fn retained_origin_write(
        &self,
        allocation: AllocationId,
        after: u32,
        projections: &mut ProjectionWalk,
        budget: &mut ExecutionBudget,
        scratch: &mut usize,
    ) -> Result<Option<Stamp>, ConstantFoldError> {
        let mut pending = Vec::new();
        let mut visited = FxHashSet::default();
        let mut current = Some(allocation);
        while let Some(allocation) = current {
            for value in self.children(allocation) {
                budget.step()?;
                if value.shared.is_some()
                    && let Some(write) = self.origin_write(
                        value.access_target(),
                        after,
                        projections,
                        budget,
                        scratch,
                    )?
                {
                    return Ok(Some(write));
                }
                if let Some(child) = value.allocation
                    && self.allocation(child).may_retain_origins
                    && !visited.contains(&child)
                {
                    budget.charge_scratch(scratch, 4 * std::mem::size_of::<AllocationId>())?;
                    visited.insert(child);
                    budget.reserve_scratch(&mut pending, 1, scratch)?;
                    pending.push(child);
                }
            }
            current = pending.pop();
        }
        Ok(None)
    }

    fn expired_projection(
        &self,
        projection: Option<ProjectionId>,
        budget: &mut ExecutionBudget,
    ) -> Result<Option<Stamp>, ConstantFoldError> {
        budget.with_scratch(|budget, scratch| {
            let mut walk = ProjectionWalk::default();
            self.walk_projection(projection, &mut walk, budget, scratch)
        })
    }

    fn walk_projection(
        &self,
        mut projection: Option<ProjectionId>,
        walk: &mut ProjectionWalk,
        budget: &mut ExecutionBudget,
        scratch: &mut usize,
    ) -> Result<Option<Stamp>, ConstantFoldError> {
        while let Some(id) = projection.take().or_else(|| walk.pending.pop()) {
            budget.step()?;
            if let Some(visited) = &mut walk.visited {
                if visited.contains(&id) {
                    continue;
                }
                budget.charge_scratch(scratch, 4 * std::mem::size_of::<ProjectionId>())?;
                visited.insert(id);
            }
            let row = &self.projections[id.0.get() as usize - 1];
            let write = if let Some(edge) = self.edge(row.edge) {
                match walk.since {
                    Some(after) => (edge.write.epoch > after).then_some(edge.write),
                    None => (edge.revision != row.revision).then_some(edge.write),
                }
            } else {
                let write = self.allocation(row.edge.owner).structural_write;
                walk.since
                    .is_none_or(|after| write.epoch > after)
                    .then_some(write)
            };
            if write.is_some() {
                return Ok(write);
            }
            projection = row.ancestors.or(row.retained);
            if row.ancestors.is_some()
                && let Some(origin) = row.retained.filter(|origin| Some(*origin) != row.ancestors)
            {
                // Projection rows point only backwards. The first fork cannot
                // revisit the preceding linear path.
                walk.visited.get_or_insert_with(FxHashSet::default);
                budget.reserve_scratch(&mut walk.pending, 1, scratch)?;
                walk.pending.push(origin);
            }
        }
        Ok(None)
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

    fn cell(&self, id: CellId) -> &Cell {
        &self.cells[id.0.get() as usize - 1]
    }

    fn shared_access(&self, id: SharedAccessId) -> &SharedAccess {
        &self.shared[id.0.get() as usize - 1]
    }

    fn site(&self, id: SiteId) -> &AccessSiteRecord {
        &self.sites[id.0.get() as usize - 1]
    }

    fn site_id(
        &mut self,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<Option<SiteId>, ConstantFoldError> {
        if site.span.is_none() {
            return Ok(None);
        }
        if let Some(&id) = self.site_ids.get(&site) {
            return Ok(Some(id));
        }
        retain::<AccessSiteRecord>(1, budget)?;
        retain::<(AccessSite, SiteId)>(1, budget)?;
        let id = SiteId(nonzero_index(self.sites.len())?);
        self.sites.push(AccessSiteRecord {
            source: site,
            last_acquisition: None,
        });
        self.site_ids.insert(site, id);
        Ok(Some(id))
    }

    fn prepare_stamp(
        &mut self,
        site: AccessSite,
        budget: &mut ExecutionBudget,
    ) -> Result<Stamp, ConstantFoldError> {
        Ok(Stamp {
            epoch: next_revision(self.epoch)?,
            site: self.site_id(site, budget)?,
        })
    }

    fn add_parent(
        &mut self,
        child: AllocationId,
        parent: AllocationId,
        budget: &mut ExecutionBudget,
    ) -> Result<(), ConstantFoldError> {
        self.reserve_parent(child, budget)?;
        self.allocation_mut(child).parents.push(parent);
        Ok(())
    }

    fn reserve_parent(
        &mut self,
        child: AllocationId,
        budget: &mut ExecutionBudget,
    ) -> Result<(), ConstantFoldError> {
        let parents = &mut self.allocation_mut(child).parents;
        if parents.len() == parents.capacity() {
            let target = (parents.len() + 1).next_power_of_two();
            retain::<AllocationId>(target - parents.capacity(), budget)?;
            parents.reserve_exact(target - parents.len());
        }
        Ok(())
    }

    fn prepare_containment(
        &mut self,
        parent: AllocationId,
        old: Option<AllocationId>,
        new: Option<AllocationId>,
        budget: &mut ExecutionBudget,
    ) -> Result<ContainmentChange, ConstantFoldError> {
        if old == new {
            return Ok(ContainmentChange::default());
        }
        let mut change = ContainmentChange {
            removed: None,
            added: new,
        };
        if let Some(child) = old {
            for (index, &owner) in self.allocation(child).parents.iter().enumerate() {
                budget.step()?;
                if owner == parent {
                    change.removed = Some((child, index));
                    break;
                }
            }
            if change.removed.is_none() {
                return Err(storage_invariant());
            }
        }
        if let Some(child) = new {
            self.reserve_parent(child, budget)?;
        }
        Ok(change)
    }

    fn commit_containment(&mut self, parent: AllocationId, change: ContainmentChange) {
        if let Some((child, index)) = change.removed {
            self.allocation_mut(child).parents.swap_remove(index);
        }
        if let Some(child) = change.added {
            self.allocation_mut(child).parents.push(parent);
        }
    }

    /// Check work and capacity before changing live contents or generations.
    fn mutate<T>(
        &mut self,
        owner: AllocationId,
        site: AccessSite,
        budget: &mut ExecutionBudget,
        commit: impl FnOnce(&mut Self, Stamp) -> T,
    ) -> Result<T, ConstantFoldError> {
        budget.with_scratch(|budget, scratch| {
            let mut ancestors = Vec::new();
            let mut visited = FxHashSet::default();
            let mut current = owner;
            let mut cursor = 0;
            loop {
                for &parent in &self.allocation(current).parents {
                    budget.step()?;
                    if !visited.contains(&parent) {
                        budget.charge_scratch(scratch, 4 * std::mem::size_of::<AllocationId>())?;
                        visited.insert(parent);
                        budget.reserve_scratch(&mut ancestors, 1, scratch)?;
                        ancestors.push(parent);
                    }
                }
                let Some(&ancestor) = ancestors.get(cursor) else {
                    break;
                };
                current = ancestor;
                cursor += 1;
            }
            let stamp = self.prepare_stamp(site, budget)?;
            let value = commit(self, stamp);
            self.epoch = stamp.epoch;
            let allocation = self.allocation_mut(owner);
            allocation.write = stamp;
            let retains_origins = allocation.may_retain_origins;
            for ancestor in ancestors {
                let allocation = self.allocation_mut(ancestor);
                allocation.write = stamp;
                allocation.may_retain_origins |= retains_origins;
            }
            Ok(value)
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
