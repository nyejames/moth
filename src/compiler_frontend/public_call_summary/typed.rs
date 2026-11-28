//! Finite call-boundary identities, guarded products and their widening order.
//!
//! A guard only partitions a relation already certified to factor by the fixed
//! supports under that guard. A joint relation must not be projected and then
//! restricted: projection has already forgotten the correlations.

use super::{
    CallEndpoint, CallField, CallParameterPath, CallResultPath, PublicCallParameterAccess,
    allocation_error, sorted_subset, strictly_sorted,
};
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PortId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SupportId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum EntryPort {
    Wrapper(usize),
    Immediate(usize),
    Terminal(usize),
    Value {
        parameter: usize,
        path: Box<[CallField]>,
    },
    Edge(PortId, CallField),
}

impl PortId {
    pub(crate) fn parameter(mut self, ports: &[EntryPort]) -> usize {
        loop {
            match ports[self.0 as usize] {
                EntryPort::Wrapper(parameter)
                | EntryPort::Immediate(parameter)
                | EntryPort::Terminal(parameter)
                | EntryPort::Value { parameter, .. } => return parameter,
                EntryPort::Edge(holder, _) => self = holder,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum IdentityKind {
    Wrapper,
    Cell,
    Allocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Role {
    Slot,
    WholeAlias,
    ProjectedAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Permission {
    ReadOnly,
    Writable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ReachKind {
    Cell,
    Referent,
    EntryStructure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Predicate {
    Role(PortId, Role),
    Equal(IdentityKind, PortId, PortId),
    Writable(PortId),
    DerivedFrom(PortId, PortId),
}

impl Predicate {
    fn canonical(self) -> Self {
        match self {
            Self::Equal(kind, left, right) if right < left => Self::Equal(kind, right, left),
            _ => self,
        }
    }
}

pub(crate) type Literal = (Predicate, bool);

/// Equality is closed separately for each kind of object. A different parameter
/// spelling is never, on its own, a disequality.
#[derive(Default)]
struct GuardFacts {
    parents: BTreeMap<(IdentityKind, PortId), PortId>,
    unequal: Vec<(IdentityKind, PortId, PortId)>,
    roles: BTreeMap<PortId, u8>,
    access: BTreeMap<Predicate, bool>,
}

impl GuardFacts {
    fn under(path: &[Literal]) -> Self {
        let mut facts = Self::default();
        for &literal in path {
            facts.assume(literal);
        }
        facts
    }

    fn representative(&self, kind: IdentityKind, mut port: PortId) -> PortId {
        while let Some(&parent) = self.parents.get(&(kind, port)) {
            port = parent;
        }
        port
    }

    fn role_bit(role: Role) -> u8 {
        1 << role as u8
    }

    fn meaning(&self, predicate: Predicate) -> Option<bool> {
        match predicate {
            Predicate::Equal(kind, left, right) => {
                let left = self.representative(kind, left);
                let right = self.representative(kind, right);
                if left == right {
                    Some(true)
                } else if self.unequal.iter().any(|&(sort, a, b)| {
                    if kind != sort {
                        return false;
                    }
                    let a = self.representative(kind, a);
                    let b = self.representative(kind, b);
                    (left == a && right == b) || (left == b && right == a)
                }) {
                    Some(false)
                } else {
                    None
                }
            }
            Predicate::Role(port, role) => {
                let possible = self.roles.get(&port).copied().unwrap_or(0b111);
                let bit = Self::role_bit(role);
                if possible & bit == 0 {
                    Some(false)
                } else {
                    (possible == bit).then_some(true)
                }
            }
            _ => self.access.get(&predicate).copied(),
        }
    }

    fn assume(&mut self, (predicate, holds): Literal) -> bool {
        if let Some(known) = self.meaning(predicate) {
            return known == holds;
        }
        match predicate {
            Predicate::Equal(kind, left, right) if holds => {
                let left = self.representative(kind, left);
                let right = self.representative(kind, right);
                self.parents
                    .insert((kind, left.max(right)), left.min(right));
            }
            Predicate::Equal(kind, left, right) => self.unequal.push((kind, left, right)),
            Predicate::Role(port, role) => {
                let possible = self.roles.entry(port).or_insert(0b111);
                let bit = Self::role_bit(role);
                *possible &= if holds { bit } else { !bit };
            }
            _ => {
                self.access.insert(predicate, holds);
            }
        }
        true
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Guard(Vec<Literal>);

impl Guard {
    pub(crate) fn implied_by(literals: &[Literal], predicate: Predicate) -> Option<bool> {
        GuardFacts::under(literals).meaning(predicate)
    }

    pub(crate) fn new(literals: impl IntoIterator<Item = Literal>) -> Option<Self> {
        let mut facts = GuardFacts::default();
        for (test, holds) in literals {
            if !facts.assume((test.canonical(), holds)) {
                return None;
            }
        }
        let mut normalised = Vec::new();
        for &(kind, port) in facts.parents.keys() {
            normalised.push((
                Predicate::Equal(kind, facts.representative(kind, port), port),
                true,
            ));
        }
        for &(kind, left, right) in &facts.unequal {
            let left = facts.representative(kind, left);
            let right = facts.representative(kind, right);
            normalised.push((Predicate::Equal(kind, left, right).canonical(), false));
        }
        for (&port, &possible) in &facts.roles {
            for role in [Role::Slot, Role::WholeAlias, Role::ProjectedAlias] {
                let bit = GuardFacts::role_bit(role);
                if possible == bit {
                    normalised.push((Predicate::Role(port, role), true));
                } else if possible.count_ones() > 1 && possible & bit == 0 {
                    normalised.push((Predicate::Role(port, role), false));
                }
            }
        }
        normalised.extend(facts.access);
        normalised.sort_unstable();
        normalised.dedup();
        Some(Self(normalised))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ExportPort {
    Result(CallResultPath),
    Binding(usize),
    PrivateTerminal(usize),
    Cell(CallParameterPath),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum StorageKind {
    Wrapper,
    Allocation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ObjectRef {
    Entry(PortId),
    Exact(ExportPort),
    Family(StorageKind, CanonicalTypeIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CellPort {
    Slot(ObjectRef),
    Terminal(PortId),
    Edge(ObjectRef, CallField),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FamilyPort {
    pub receiver: ObjectRef,
    pub selector: CallField,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OutputPort {
    Result(usize),
    Binding(ObjectRef),
    Cell(CellPort),
    Family(FamilyPort),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ReachBound {
    pub roots: Vec<PortId>,
    pub region: ReachKind,
    pub types: Vec<CanonicalTypeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Footprint {
    Locations(Vec<OutputPort>),
    WritableReach(ReachBound),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScalarSourceBound {
    Values(Vec<PortId>),
    Reach(ReachBound),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScalarSource {
    EntryValue(PortId),
    Independent,
    Unresolved(ScalarSourceBound),
}

/// A retained access restriction is independent of the value's source. `None`
/// in a raw value transfer leaves the actual snapshot's permission unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PathAccess {
    pub restriction: Option<Permission>,
    pub derived_from: Option<Derivation>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Derivation {
    Entry(PortId),
    Retained(ObjectRef),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RetainedRoute {
    pub access: PathAccess,
    pub dependencies: Vec<ReceiverDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReceiverDependency {
    Known(Vec<ObjectRef>),
    Unresolved(ReachBound),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ValueSource {
    Scalar(ScalarSource),
    Allocation(ObjectRef),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ValueRef {
    pub source: ValueSource,
    pub access: PathAccess,
    pub dependencies: Vec<ReceiverDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BindingPost {
    OwnSlot,
    Link(ObjectRef, RetainedRoute),
    Projected(CellPort, RetainedRoute),
}

/// A post records a nonempty cell snapshot, not a list of successive writes.
/// `None` is explicit absence: it can coexist with a value and is distinct
/// from omission.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CellPost {
    Replace(Vec<Option<ValueRef>>),
    WeakJoin(Vec<Option<ValueRef>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Occurrence {
    AtMostOnce,
    MaybeMany,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FamilyPost {
    Extend(Vec<(ValueRef, Occurrence)>),
    ReplaceAll(Vec<(ValueRef, Occurrence)>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Coverage {
    #[default]
    Complete,
    Unresolved(Footprint),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Post {
    pub bindings: Vec<(ObjectRef, BindingPost)>,
    pub cells: Vec<(CellPort, CellPost)>,
    pub families: Vec<(FamilyPort, FamilyPost)>,
    pub results: Vec<(usize, ValueRef)>,
    pub retained: Vec<(ObjectRef, StorageKind, CanonicalTypeIdentity)>,
    pub coverage: Coverage,
}

impl Post {
    fn result_slots(&self) -> impl Iterator<Item = usize> + '_ {
        let locations = match &self.coverage {
            Coverage::Unresolved(Footprint::Locations(locations)) => locations.as_slice(),
            _ => &[],
        };
        self.results
            .iter()
            .map(|(slot, _)| *slot)
            .chain(locations.iter().filter_map(|location| match location {
                OutputPort::Result(slot) => Some(*slot),
                _ => None,
            }))
    }

    pub(crate) fn visit_entry_ports(
        &self,
        include_destinations: bool,
        visit: &mut impl FnMut(PortId),
    ) {
        struct Visitor<'a, F>(&'a mut F);

        impl<F: FnMut(PortId)> Visitor<'_, F> {
            fn object(&mut self, object: &ObjectRef) {
                if let ObjectRef::Entry(port) = object {
                    (self.0)(*port);
                }
            }

            fn cell(&mut self, cell: &CellPort) {
                match cell {
                    CellPort::Terminal(port) => (self.0)(*port),
                    CellPort::Slot(object) | CellPort::Edge(object, _) => self.object(object),
                }
            }

            fn ports(&mut self, ports: &[PortId]) {
                ports.iter().copied().for_each(&mut *self.0);
            }

            fn route(&mut self, access: &PathAccess, dependencies: &[ReceiverDependency]) {
                match &access.derived_from {
                    Some(Derivation::Entry(port)) => (self.0)(*port),
                    Some(Derivation::Retained(object)) => self.object(object),
                    None => {}
                }
                for dependency in dependencies {
                    match dependency {
                        ReceiverDependency::Known(objects) => {
                            for object in objects {
                                self.object(object);
                            }
                        }
                        ReceiverDependency::Unresolved(bound) => self.ports(&bound.roots),
                    }
                }
            }

            fn value(&mut self, value: &ValueRef) {
                match &value.source {
                    ValueSource::Allocation(object) => self.object(object),
                    ValueSource::Scalar(ScalarSource::EntryValue(port)) => (self.0)(*port),
                    ValueSource::Scalar(ScalarSource::Unresolved(bound)) => match bound {
                        ScalarSourceBound::Values(ports) => self.ports(ports),
                        ScalarSourceBound::Reach(bound) => self.ports(&bound.roots),
                    },
                    ValueSource::Scalar(ScalarSource::Independent) => {}
                }
                self.route(&value.access, &value.dependencies);
            }
        }

        let mut walk = Visitor(visit);
        for (object, edit) in &self.bindings {
            if include_destinations {
                walk.object(object);
            }
            match edit {
                BindingPost::OwnSlot => {}
                BindingPost::Link(source, route) => {
                    walk.object(source);
                    walk.route(&route.access, &route.dependencies);
                }
                BindingPost::Projected(source, route) => {
                    walk.cell(source);
                    walk.route(&route.access, &route.dependencies);
                }
            }
        }
        for (cell, edit) in &self.cells {
            if include_destinations {
                walk.cell(cell);
            }
            let (CellPost::Replace(values) | CellPost::WeakJoin(values)) = edit;
            for value in values.iter().flatten() {
                walk.value(value);
            }
        }
        for (family, edit) in &self.families {
            if include_destinations {
                walk.object(&family.receiver);
            }
            let (FamilyPost::Extend(members) | FamilyPost::ReplaceAll(members)) = edit;
            for (member, _) in members {
                walk.value(member);
            }
        }
        for (_, value) in &self.results {
            walk.value(value);
        }
        if !include_destinations {
            if let Coverage::Unresolved(Footprint::WritableReach(bound)) = &self.coverage {
                walk.ports(&bound.roots);
            }
            return;
        }
        for (object, _, _) in &self.retained {
            walk.object(object);
        }
        match &self.coverage {
            Coverage::Complete => {}
            Coverage::Unresolved(Footprint::WritableReach(bound)) => walk.ports(&bound.roots),
            Coverage::Unresolved(Footprint::Locations(outputs)) => {
                for output in outputs {
                    match output {
                        OutputPort::Result(_) => {}
                        OutputPort::Binding(object) => walk.object(object),
                        OutputPort::Cell(cell) => walk.cell(cell),
                        OutputPort::Family(family) => walk.object(&family.receiver),
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Component {
    pub support: SupportId,
    pub alternatives: Vec<Post>,
}

/// Omission of a support in a reachable product denotes its identity
/// alternative. The zero-component product is therefore distinct from bottom.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Product(Option<Arc<[Component]>>);

impl Product {
    pub(crate) fn new(mut components: Vec<Component>) -> Option<Self> {
        components.sort_unstable_by_key(|component| component.support);
        if !strictly_sorted(components.iter().map(|component| component.support)) {
            return None;
        }
        for component in &mut components {
            if component.alternatives.is_empty() {
                return None;
            }
            component.alternatives.sort_unstable();
            component.alternatives.dedup();
        }
        components.retain(|component| component.alternatives.as_slice() != [Post::default()]);
        Some(Self::from_sorted(components))
    }

    pub(crate) fn components(&self) -> &[Component] {
        self.0.as_deref().unwrap_or(&[])
    }

    fn from_sorted(components: Vec<Component>) -> Self {
        Self((!components.is_empty()).then(|| Arc::from(components)))
    }

    fn contains(&self, lower: &Self) -> bool {
        let mut upper = self.components().iter().peekable();
        let preserve = Post::default();
        for component in lower.components() {
            while upper
                .peek()
                .is_some_and(|next| next.support < component.support)
            {
                let next = upper.next().expect("peeked support");
                if next.alternatives.binary_search(&preserve).is_err() {
                    return false;
                }
            }
            let Some(next) = upper.next_if(|next| next.support == component.support) else {
                return false;
            };
            if !sorted_subset(&component.alternatives, &next.alternatives) {
                return false;
            }
        }
        upper.all(|next| next.alternatives.binary_search(&preserve).is_ok())
    }

    fn union(&self, other: &Self) -> Self {
        let mut left = self.components().iter().peekable();
        let mut right = other.components().iter().peekable();
        let mut components =
            Vec::with_capacity(self.components().len().max(other.components().len()));
        while let Some(next) = match (left.peek(), right.peek()) {
            (None, None) => None,
            (Some(a), Some(b)) if a.support == b.support => {
                let a = left.next().expect("peeked component");
                let b = right.next().expect("peeked component");
                let alternatives = join_alternatives(&a.alternatives, &b.alternatives);
                Some(Component {
                    support: a.support,
                    alternatives,
                })
            }
            (Some(a), Some(b)) if a.support < b.support => left.next().map(with_preserve),
            (Some(_), None) => left.next().map(with_preserve),
            _ => right.next().map(with_preserve),
        } {
            components.push(next);
        }
        Self::from_sorted(components)
    }
}

fn join_alternatives(left: &[Post], right: &[Post]) -> Vec<Post> {
    let mut alternatives = Vec::with_capacity(left.len().max(right.len()));
    let mut left = left.iter().peekable();
    let mut right = right.iter().peekable();
    while let Some((a, b)) = left.peek().zip(right.peek()) {
        if a < b {
            alternatives.extend(left.next().cloned());
        } else {
            if a == b {
                left.next();
            }
            alternatives.extend(right.next().cloned());
        }
    }
    alternatives.extend(left.cloned());
    alternatives.extend(right.cloned());
    alternatives
}

fn with_preserve(component: &Component) -> Component {
    let mut component = component.clone();
    let preserve = Post::default();
    if let Err(index) = component.alternatives.binary_search(&preserve) {
        component.alternatives.insert(index, preserve);
    }
    component
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lane {
    #[default]
    Bottom,
    Returning(Product),
    Decision(Arc<Decision>),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Decision {
    predicate: Predicate,
    no: Lane,
    yes: Lane,
}

impl Lane {
    pub(crate) fn guarded(guard: Guard, product: Product) -> Self {
        let mut lane = Self::Returning(product);
        for (predicate, truth) in guard.0.into_iter().rev() {
            let (no, yes) = if truth {
                (Self::Bottom, lane)
            } else {
                (lane, Self::Bottom)
            };
            lane = Self::Decision(Arc::new(Decision { predicate, no, yes }));
        }
        lane.union(&Self::Bottom)
    }

    #[cfg(test)]
    pub(crate) fn for_presentation(
        &self,
        mut matches: impl FnMut(Predicate) -> bool,
    ) -> Option<&Product> {
        let mut current = self;
        loop {
            match current {
                Self::Bottom => return None,
                Self::Returning(product) => return Some(product),
                Self::Decision(test) => {
                    current = if matches(test.predicate) {
                        &test.yes
                    } else {
                        &test.no
                    }
                }
            }
        }
    }

    pub(crate) fn can_return(&self) -> bool {
        !matches!(self, Self::Bottom)
    }

    /// A projection can omit the actuals only when they cannot decide reach.
    pub(crate) fn structural_reach(&self) -> Option<bool> {
        match self {
            Self::Bottom => Some(false),
            Self::Returning(_) => Some(true),
            Self::Decision(test) => {
                let no = test.no.structural_reach()?;
                (test.yes.structural_reach()? == no).then_some(no)
            }
        }
    }

    pub(crate) fn result_extent(&self) -> usize {
        match self {
            Self::Bottom => 0,
            Self::Returning(product) => product
                .components()
                .iter()
                .flat_map(|component| &component.alternatives)
                .flat_map(|post| post.result_slots())
                .max()
                .map_or(0, |slot| slot.saturating_add(1)),
            Self::Decision(test) => test.no.result_extent().max(test.yes.result_extent()),
        }
    }

    fn visit_unresolved_result_slots(&self, visit: &mut impl FnMut(usize)) {
        match self {
            Self::Bottom => {}
            Self::Returning(product) => {
                for post in product
                    .components()
                    .iter()
                    .flat_map(|component| &component.alternatives)
                {
                    if let Coverage::Unresolved(Footprint::Locations(locations)) = &post.coverage {
                        for location in locations {
                            if let OutputPort::Result(slot) = location {
                                visit(*slot);
                            }
                        }
                    }
                }
            }
            Self::Decision(test) => {
                test.no.visit_unresolved_result_slots(visit);
                test.yes.visit_unresolved_result_slots(visit);
            }
        }
    }

    /// A partial actual can admit both arms, but it cannot admit inconsistent
    /// combinations of the same typed observations.
    pub(crate) fn visit_compatible<E>(
        &self,
        mut observe: impl FnMut(Predicate) -> Result<Option<bool>, E>,
        mut visit: impl FnMut(&[Literal], &Product) -> Result<(), E>,
    ) -> Result<(), E> {
        self.visit_arms(&mut Vec::new(), &mut observe, &mut visit)
    }

    fn visit_arms<E>(
        &self,
        path: &mut Vec<Literal>,
        observe: &mut impl FnMut(Predicate) -> Result<Option<bool>, E>,
        visit: &mut impl FnMut(&[Literal], &Product) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Bottom => Ok(()),
            Self::Returning(product) => visit(path, product),
            Self::Decision(test) => {
                let observation = observe(test.predicate)?;
                let implied = GuardFacts::under(path).meaning(test.predicate);
                for (truth, arm) in [(false, &test.no), (true, &test.yes)] {
                    if observation.is_some_and(|holds| holds != truth)
                        || implied.is_some_and(|holds| holds != truth)
                        || !arm.can_return()
                    {
                        continue;
                    }
                    path.push((test.predicate, truth));
                    arm.visit_arms(path, observe, visit)?;
                    path.pop();
                }
                Ok(())
            }
        }
    }

    /// Align the least test before joining. Tests implied by earlier typed
    /// observations have one feasible arm; they are not arbitrary don't-cares.
    pub(crate) fn union(&self, other: &Self) -> Self {
        join_lanes(self, other, &mut Vec::new(), &mut BTreeSet::new())
    }

    pub(crate) fn compose_supports(lanes: Vec<Self>) -> Self {
        let mut components = Vec::new();
        let mut guarded = Vec::new();
        for lane in lanes {
            match lane {
                Self::Bottom => return Self::Bottom,
                Self::Returning(product) => components.extend(product.components().iter().cloned()),
                Self::Decision(_) => guarded.push(lane),
            }
        }
        let plain = Self::Returning(Product::new(components).expect("disjoint boundary supports"));
        guarded
            .iter()
            .fold(plain, |product, lane| product.compose_support(lane))
    }

    /// Install an independently projected support without adding a preserve row.
    fn compose_support(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Bottom, _) | (_, Self::Bottom) => Self::Bottom,
            (Self::Returning(left), Self::Returning(right)) => {
                if left.components().is_empty() {
                    return other.clone();
                }
                if right.components().is_empty() {
                    return self.clone();
                }
                let components = left
                    .components()
                    .iter()
                    .chain(right.components())
                    .cloned()
                    .collect();
                Self::Returning(Product::new(components).expect("disjoint boundary supports"))
            }
            _ => {
                let predicate = self
                    .head()
                    .into_iter()
                    .chain(other.head())
                    .min()
                    .expect("guard");
                let arm = |truth| {
                    self.branch(predicate, truth)
                        .compose_support(other.branch(predicate, truth))
                };
                let (no, yes) = (arm(false), arm(true));
                if no == yes {
                    no
                } else {
                    Self::Decision(Arc::new(Decision { predicate, no, yes }))
                }
            }
        }
    }

    pub(crate) fn project_supports(&self, supports: &[SupportId]) -> Self {
        match self {
            Self::Bottom => Self::Bottom,
            Self::Returning(product) => {
                let components = product.components();
                let selected = supports
                    .iter()
                    .filter_map(|support| {
                        let index = components
                            .binary_search_by_key(support, |component| component.support)
                            .ok()?;
                        Some(components[index].clone())
                    })
                    .collect();
                Self::Returning(Product::new(selected).expect("projected supports are distinct"))
            }
            Self::Decision(test) => {
                let no = test.no.project_supports(supports);
                let yes = test.yes.project_supports(supports);
                if no == yes {
                    no
                } else {
                    Self::Decision(Arc::new(Decision {
                        predicate: test.predicate,
                        no,
                        yes,
                    }))
                }
            }
        }
    }

    fn head(&self) -> Option<Predicate> {
        match self {
            Self::Decision(test) => Some(test.predicate),
            _ => None,
        }
    }

    fn branch(&self, predicate: Predicate, yes: bool) -> &Self {
        match self {
            Self::Decision(test) if test.predicate == predicate => {
                if yes {
                    &test.yes
                } else {
                    &test.no
                }
            }
            _ => self,
        }
    }

    fn role_arm(&self, port: PortId, role: Role) -> &Self {
        let mut lane = self;
        while let Some(Predicate::Role(test_port, test_role)) = lane.head() {
            if test_port != port {
                break;
            }
            lane = lane.branch(Predicate::Role(port, test_role), role == test_role);
        }
        lane
    }
}

#[test]
fn structural_reach_requires_both_guard_arms_to_agree() {
    let predicate = Predicate::Writable(PortId(0));
    let arm = |truth| {
        Lane::guarded(
            Guard::new([(predicate, truth)]).unwrap(),
            Product::default(),
        )
    };
    let yes = arm(true);
    assert_eq!(yes.structural_reach(), None);
    assert_eq!(yes.union(&arm(false)).structural_reach(), Some(true));
    assert_eq!(Lane::Bottom.structural_reach(), Some(false));
}

fn join_lanes(
    left: &Lane,
    right: &Lane,
    path: &mut Vec<Literal>,
    shared: &mut BTreeSet<Arc<Decision>>,
) -> Lane {
    let Some(predicate) = left.head().into_iter().chain(right.head()).min() else {
        return match (left, right) {
            (Lane::Returning(a), Lane::Returning(b)) if a != b => Lane::Returning(a.union(b)),
            (Lane::Bottom, _) => right.clone(),
            _ => left.clone(),
        };
    };
    if let Predicate::Role(port, _) = predicate {
        // All role tests for this wrapper precede the remaining predicates.
        // Seal its three feasible arms in one fixed two-test basis.
        let arm = |role| (left.role_arm(port, role), right.role_arm(port, role));
        let same = |(a, b): (&Lane, &Lane), (c, d): (&Lane, &Lane)| {
            std::ptr::eq(a, c) && std::ptr::eq(b, d)
        };
        let slot_input = arm(Role::Slot);
        let whole_input = arm(Role::WholeAlias);
        let projected_input = arm(Role::ProjectedAlias);
        let slot = join_lanes(slot_input.0, slot_input.1, path, shared);
        let whole = if same(whole_input, slot_input) {
            slot.clone()
        } else {
            join_lanes(whole_input.0, whole_input.1, path, shared)
        };
        let projected = if same(projected_input, slot_input) {
            slot.clone()
        } else if same(projected_input, whole_input) {
            whole.clone()
        } else {
            join_lanes(projected_input.0, projected_input.1, path, shared)
        };
        let aliases = intern_decision(
            Predicate::Role(port, Role::WholeAlias),
            projected,
            whole,
            shared,
        );
        return intern_decision(Predicate::Role(port, Role::Slot), aliases, slot, shared);
    }
    let facts = GuardFacts::under(path);
    if let Some(yes) = facts.meaning(predicate) {
        return join_lanes(
            left.branch(predicate, yes),
            right.branch(predicate, yes),
            path,
            shared,
        );
    }
    let mut branch = |yes| {
        path.push((predicate, yes));
        let lane = join_lanes(
            left.branch(predicate, yes),
            right.branch(predicate, yes),
            path,
            shared,
        );
        path.pop();
        lane
    };
    let no = branch(false);
    let yes = branch(true);
    if matches!(predicate, Predicate::Equal(..)) && no != yes {
        // A branch can use different names for the same equality under the
        // parent equation. Drop the split if either arm extends to both
        // feasible contexts, rather than comparing the children structurally.
        let mut agrees = |truth| {
            path.push((predicate, truth));
            let equivalent = lane_contains(&no, &yes, path) && lane_contains(&yes, &no, path);
            path.pop();
            equivalent
        };
        let use_no = agrees(true);
        let use_yes = agrees(false);
        match (use_no, use_yes) {
            (true, true) => return no.min(yes),
            (true, false) => return no,
            (false, true) => return yes,
            (false, false) => {}
        }
    }
    intern_decision(predicate, no, yes, shared)
}

fn intern_decision(
    predicate: Predicate,
    no: Lane,
    yes: Lane,
    shared: &mut BTreeSet<Arc<Decision>>,
) -> Lane {
    if no == yes {
        return no;
    }
    let node = Decision { predicate, no, yes };
    if let Some(existing) = shared.get(&node) {
        return Lane::Decision(Arc::clone(existing));
    }
    let node = Arc::new(node);
    shared.insert(Arc::clone(&node));
    Lane::Decision(node)
}

fn lane_contains(upper: &Lane, lower: &Lane, path: &mut Vec<Literal>) -> bool {
    if matches!(lower, Lane::Bottom) || upper == lower {
        return true;
    }
    let Some(predicate) = upper.head().into_iter().chain(lower.head()).min() else {
        return match (upper, lower) {
            (Lane::Returning(upper), Lane::Returning(lower)) => upper.contains(lower),
            _ => false,
        };
    };
    let facts = GuardFacts::under(path);
    if let Some(yes) = facts.meaning(predicate) {
        return lane_contains(
            upper.branch(predicate, yes),
            lower.branch(predicate, yes),
            path,
        );
    }
    [false, true].into_iter().all(|yes| {
        path.push((predicate, yes));
        let contains = lane_contains(
            upper.branch(predicate, yes),
            lower.branch(predicate, yes),
            path,
        );
        path.pop();
        contains
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TypedCallAllocationFacts {
    pub ports: Vec<EntryPort>,
    pub support_count: usize,
    pub success: Lane,
    pub error: Lane,
    pub may_retain: Vec<usize>,
}

pub(crate) struct Shape<'a> {
    pub parameters: &'a [PublicCallParameterAccess],
    pub ports: &'a [EntryPort],
    pub support_count: usize,
    pub success_slots: Option<usize>,
    pub error_slot: bool,
}

/// Layout lookup stays with the signature owner. In particular a private
/// nominal's stable identity is not a substitute for its declaration layout.
pub(crate) trait Layout {
    type Type: Ord + Clone;

    fn parameter(&self, index: usize) -> Option<CallEndpoint<Self::Type>>;
    fn result(&self, error: bool, slot: usize) -> Option<CallEndpoint<Self::Type>>;
    fn child(&self, ty: &Self::Type, selector: CallField) -> Option<CallEndpoint<Self::Type>>;
    fn stable(&self, ty: &CanonicalTypeIdentity) -> Option<CallEndpoint<Self::Type>>;
    fn matches_identity(&self, endpoint: &Self::Type, identity: &CanonicalTypeIdentity) -> bool {
        self.stable(identity)
            .is_some_and(|known| &known.ty == endpoint)
    }
    fn children(&self, ty: &Self::Type, visit: &mut dyn FnMut(CallEndpoint<Self::Type>));
}

struct LayoutCheck<'a, L: Layout> {
    layout: &'a L,
    entries: Vec<CallEndpoint<L::Type>>,
    ports: &'a [EntryPort],
    error: bool,
}

/// Re-express a lane in the embedded port and support tables.
fn remap_lane(lane: &Lane, ports: &[PortId], supports: &[SupportId]) -> Lane {
    struct Remap<'a> {
        ports: &'a [PortId],
        supports: &'a [SupportId],
    }

    impl Remap<'_> {
        fn port(&self, port: &mut PortId) {
            *port = self.ports[port.0 as usize];
        }

        fn ports(&self, ports: &mut [PortId]) {
            for port in &mut *ports {
                self.port(port);
            }
            ports.sort_unstable();
        }

        fn object(&self, object: &mut ObjectRef) {
            if let ObjectRef::Entry(port) = object {
                self.port(port);
            }
        }

        fn cell(&self, cell: &mut CellPort) {
            match cell {
                CellPort::Terminal(port) => self.port(port),
                CellPort::Slot(object) | CellPort::Edge(object, _) => self.object(object),
            }
        }

        fn route(&self, access: &mut PathAccess, dependencies: &mut [ReceiverDependency]) {
            match &mut access.derived_from {
                Some(Derivation::Entry(port)) => self.port(port),
                Some(Derivation::Retained(object)) => self.object(object),
                None => {}
            }
            for dependency in &mut *dependencies {
                match dependency {
                    ReceiverDependency::Known(objects) => {
                        for object in &mut *objects {
                            self.object(object);
                        }
                        objects.sort_unstable();
                    }
                    ReceiverDependency::Unresolved(bound) => self.ports(&mut bound.roots),
                }
            }
            dependencies.sort_unstable();
        }

        fn value(&self, value: &mut ValueRef) {
            match &mut value.source {
                ValueSource::Allocation(object) => self.object(object),
                ValueSource::Scalar(ScalarSource::EntryValue(port)) => self.port(port),
                ValueSource::Scalar(ScalarSource::Unresolved(ScalarSourceBound::Values(ports))) => {
                    self.ports(ports);
                }
                ValueSource::Scalar(ScalarSource::Unresolved(ScalarSourceBound::Reach(bound))) => {
                    self.ports(&mut bound.roots);
                }
                ValueSource::Scalar(ScalarSource::Independent) => {}
            }
            self.route(&mut value.access, &mut value.dependencies);
        }

        fn post(&self, post: &mut Post) {
            for (target, edit) in &mut post.bindings {
                self.object(target);
                let route = match edit {
                    BindingPost::OwnSlot => continue,
                    BindingPost::Link(object, route) => {
                        self.object(object);
                        route
                    }
                    BindingPost::Projected(cell, route) => {
                        self.cell(cell);
                        route
                    }
                };
                self.route(&mut route.access, &mut route.dependencies);
            }
            for (target, edit) in &mut post.cells {
                self.cell(target);
                let (CellPost::Replace(values) | CellPost::WeakJoin(values)) = edit;
                for value in values.iter_mut().flatten() {
                    self.value(value);
                }
                values.sort_unstable();
            }
            for (target, edit) in &mut post.families {
                self.object(&mut target.receiver);
                let (FamilyPost::Extend(members) | FamilyPost::ReplaceAll(members)) = edit;
                for (value, _) in &mut *members {
                    self.value(value);
                }
                members.sort_unstable();
            }
            for (_, value) in &mut post.results {
                self.value(value);
            }
            for (object, _, _) in &mut post.retained {
                self.object(object);
            }
            match &mut post.coverage {
                Coverage::Complete => {}
                Coverage::Unresolved(Footprint::WritableReach(bound)) => {
                    self.ports(&mut bound.roots);
                }
                Coverage::Unresolved(Footprint::Locations(outputs)) => {
                    for output in &mut *outputs {
                        match output {
                            OutputPort::Result(_) => {}
                            OutputPort::Binding(object) => self.object(object),
                            OutputPort::Cell(cell) => self.cell(cell),
                            OutputPort::Family(family) => self.object(&mut family.receiver),
                        }
                    }
                    outputs.sort_unstable();
                }
            }
            post.bindings.sort_unstable();
            post.cells.sort_unstable();
            post.families.sort_unstable();
            post.retained.sort_unstable();
        }

        fn visit(&self, lane: &Lane, path: &mut Vec<Literal>, output: &mut Lane) {
            match lane {
                Lane::Bottom => {}
                Lane::Returning(product) => {
                    if let Some(guard) = Guard::new(path.iter().copied()) {
                        let mut components = product.components().to_vec();
                        for component in &mut components {
                            component.support = self.supports[component.support.0 as usize];
                            for post in &mut component.alternatives {
                                self.post(post);
                            }
                        }
                        let mapped = Lane::guarded(
                            guard,
                            Product::new(components).expect("embedded typed supports"),
                        );
                        *output = output.union(&mapped);
                    }
                }
                Lane::Decision(test) => {
                    let mut predicate = test.predicate;
                    match &mut predicate {
                        Predicate::Role(port, _) | Predicate::Writable(port) => self.port(port),
                        Predicate::Equal(_, left, right) | Predicate::DerivedFrom(left, right) => {
                            self.port(left);
                            self.port(right);
                        }
                    }
                    // Renumbering can reorder decisions; reconstruct the ordered diagram
                    // through the same canonical guarded union as ordinary extraction.
                    for (truth, arm) in [(false, &test.no), (true, &test.yes)] {
                        path.push((predicate, truth));
                        self.visit(arm, path, output);
                        path.pop();
                    }
                }
            }
        }
    }

    let mut output = Lane::Bottom;
    Remap { ports, supports }.visit(lane, &mut Vec::new(), &mut output);
    output
}

impl TypedCallAllocationFacts {
    pub(crate) fn shape<'a>(
        &'a self,
        parameters: &'a [PublicCallParameterAccess],
        success_slots: Option<usize>,
        error_slot: bool,
    ) -> Shape<'a> {
        Shape {
            parameters,
            ports: &self.ports,
            support_count: self.support_count,
            success_slots,
            error_slot,
        }
    }

    pub(crate) fn contains_transfer(&self, previous: &Self) -> bool {
        previous.ports == self.ports
            && previous.support_count == self.support_count
            && self.contains_embedded_transfer(previous, None)
    }

    fn contains_embedded_transfer(
        &self,
        previous: &Self,
        embedding: Option<(&[PortId], &[SupportId])>,
    ) -> bool {
        if !sorted_subset(&previous.may_retain, &self.may_retain) {
            return false;
        }
        let mut path = Vec::new();
        [
            (&self.success, &previous.success),
            (&self.error, &previous.error),
        ]
        .into_iter()
        .all(|(upper, lower)| match embedding {
            Some((ports, supports)) => {
                lane_contains(upper, &remap_lane(lower, ports, supports), &mut path)
            }
            None => lane_contains(upper, lower, &mut path),
        })
    }

    /// Check a strict canonical port superset. `None` is not a growth transition.
    pub(in crate::compiler_frontend) fn validate_port_growth(
        &self,
        next: &Self,
        embed_supports: impl FnOnce(&[PortId], usize) -> Option<Vec<SupportId>>,
    ) -> Option<Result<(), InfrastructureFailure>> {
        if self.ports.len() >= next.ports.len() {
            return None;
        }
        fn keys(ports: &[EntryPort]) -> impl Iterator<Item = (&EntryPort, Option<CallField>)> {
            ports.iter().map(|port| match port {
                EntryPort::Edge(holder, field) => (&ports[holder.0 as usize], Some(*field)),
                _ => (port, None),
            })
        }
        let embedding: Vec<_> = if next.ports.starts_with(&self.ports) {
            (0..self.ports.len()).map(|id| PortId(id as u32)).collect()
        } else {
            // An edge identity names its canonical value holder, not that holder's ordinal.
            let next_ids: FxHashMap<_, _> = keys(&next.ports)
                .enumerate()
                .map(|(id, key)| (key, PortId(id as u32)))
                .collect();
            keys(&self.ports)
                .map(|key| next_ids.get(&key).copied())
                .collect::<Option<_>>()?
        };

        let Some(supports) = embed_supports(&embedding, self.support_count) else {
            // A genuine split or merge has no unique embedding for guarded rows.
            // Retain the schema-independent lower bounds in that case.
            return Some(self.validate_schema_independent_transition(next));
        };
        debug_assert_eq!(supports.len(), self.support_count);
        debug_assert!(
            supports
                .iter()
                .all(|id| (id.0 as usize) < next.support_count)
        );

        let identity = embedding
            .iter()
            .enumerate()
            .all(|(old, new)| old == new.0 as usize)
            && supports
                .iter()
                .enumerate()
                .all(|(old, new)| old == new.0 as usize);
        let remap = (!identity).then_some((embedding.as_slice(), supports.as_slice()));
        Some(if next.contains_embedded_transfer(self, remap) {
            Ok(())
        } else {
            Err(allocation_error("typed call postconditions narrowed"))
        })
    }

    /// Keep the transfer facts whose identities survive a port/support schema expansion.
    /// A support can split or merge, so guarded rows are not comparable by their old ordinals.
    pub(in crate::compiler_frontend) fn validate_schema_independent_transition(
        &self,
        next: &Self,
    ) -> Result<(), InfrastructureFailure> {
        if !sorted_subset(&self.may_retain, &next.may_retain) {
            return Err(allocation_error("typed call postconditions narrowed"));
        }
        let mut unresolved = BTreeSet::new();
        for (before, after) in [(&self.success, &next.success), (&self.error, &next.error)] {
            if before.can_return() && !after.can_return() {
                return Err(allocation_error("typed call postconditions narrowed"));
            }
            // Explicit result slots are declaration coordinates, unlike guards and
            // footprints tied to entry ports. Compare may-unresolved coverage by slot.
            before.visit_unresolved_result_slots(&mut |slot| {
                unresolved.insert(slot);
            });
            if !unresolved.is_empty() {
                after.visit_unresolved_result_slots(&mut |slot| {
                    unresolved.remove(&slot);
                });
                if !unresolved.is_empty() {
                    return Err(allocation_error("typed call postconditions narrowed"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn lane(&self, error: bool) -> &Lane {
        if error { &self.error } else { &self.success }
    }

    pub(crate) fn is_transfer_bottom(&self) -> bool {
        self.may_retain.is_empty() && !self.success.can_return() && !self.error.can_return()
    }

    pub(crate) fn executed_may_retain(&self, parameter: usize) -> bool {
        self.may_retain.binary_search(&parameter).is_ok()
    }

    pub(crate) fn validate(&self, shape: &Shape<'_>) -> Result<(), InfrastructureFailure> {
        if !strictly_sorted(&self.may_retain)
            || self
                .may_retain
                .iter()
                .any(|&param| param >= shape.parameters.len())
        {
            return Err(allocation_error("invalid executed retention set"));
        }
        shape.validate_ports()?;
        shape.validate_lane(&self.success, shape.success_slots, None, &mut Vec::new())?;
        shape.validate_lane(
            &self.error,
            shape.error_slot.then_some(1),
            None,
            &mut Vec::new(),
        )
    }

    pub(crate) fn validate_transition(
        &self,
        next: &Self,
        shape: &Shape<'_>,
    ) -> Result<(), InfrastructureFailure> {
        self.validate(&self.shape(shape.parameters, shape.success_slots, shape.error_slot))?;
        next.validate(&next.shape(shape.parameters, shape.success_slots, shape.error_slot))?;
        if next.contains_transfer(self) {
            Ok(())
        } else {
            Err(allocation_error("typed call postconditions narrowed"))
        }
    }
}

impl TypedCallAllocationFacts {
    pub(crate) fn validate_layout(
        &self,
        shape: &Shape<'_>,
        layout: &impl Layout,
    ) -> Result<(), InfrastructureFailure> {
        self.validate(shape)?;
        let mut check = LayoutCheck {
            layout,
            entries: Vec::with_capacity(shape.ports.len()),
            ports: shape.ports,
            error: false,
        };
        for port in shape.ports {
            check.entries.push(
                check.entry_type(port).ok_or_else(|| {
                    allocation_error("typed entry port has no declaration endpoint")
                })?,
            );
        }
        check.lane(&self.success)?;
        check.error = true;
        check.lane(&self.error)
    }
}

impl<L: Layout> LayoutCheck<'_, L> {
    fn path(
        &self,
        mut endpoint: CallEndpoint<L::Type>,
        path: &[CallField],
    ) -> Option<CallEndpoint<L::Type>> {
        for &selector in path {
            endpoint = self.layout.child(&endpoint.ty, selector)?;
        }
        Some(endpoint)
    }

    fn entry_type(&self, port: &EntryPort) -> Option<CallEndpoint<L::Type>> {
        match port {
            EntryPort::Wrapper(parameter)
            | EntryPort::Immediate(parameter)
            | EntryPort::Terminal(parameter) => self.layout.parameter(*parameter),
            EntryPort::Value { parameter, path } => {
                self.path(self.layout.parameter(*parameter)?, path)
            }
            EntryPort::Edge(holder, selector) => {
                let root = self.entry_type(self.ports.get(holder.0 as usize)?)?;
                root.allocation.then_some(())?;
                self.layout.child(&root.ty, *selector)
            }
        }
    }

    fn exact_endpoint(&self, export: &ExportPort) -> Option<CallEndpoint<L::Type>> {
        let (root, path) = match export {
            ExportPort::Result(path) => (
                self.layout.result(self.error, path.slot),
                path.path.as_slice(),
            ),
            ExportPort::Binding(parameter) | ExportPort::PrivateTerminal(parameter) => {
                (self.layout.parameter(*parameter), &[][..])
            }
            ExportPort::Cell(path) => (self.layout.parameter(path.index), path.path.as_slice()),
        };
        self.path(root?, path)
    }

    fn object(&self, object: &ObjectRef, post: &Post) -> Option<CallEndpoint<L::Type>> {
        match object {
            ObjectRef::Entry(port) => self.entry_type(&self.ports[port.0 as usize]),
            ObjectRef::Exact(export) => self.exact_endpoint(export),
            _ => {
                let index = post
                    .retained
                    .binary_search_by(|(key, _, _)| key.cmp(object))
                    .ok()?;
                self.layout.stable(&post.retained[index].2)
            }
        }
    }

    fn cell(&self, cell: &CellPort, post: &Post) -> Option<CallEndpoint<L::Type>> {
        match cell {
            CellPort::Slot(wrapper) => self.object(wrapper, post),
            CellPort::Terminal(port) => self.entry_type(&self.ports[port.0 as usize]),
            CellPort::Edge(holder, selector) => self.field(holder, *selector, post),
        }
    }

    fn field(
        &self,
        holder: &ObjectRef,
        selector: CallField,
        post: &Post,
    ) -> Option<CallEndpoint<L::Type>> {
        let holder = self.object(holder, post)?;
        holder.allocation.then_some(())?;
        self.layout.child(&holder.ty, selector)
    }

    fn bound(&self, bound: &ReachBound, endpoint: impl Fn(&CallEndpoint<L::Type>) -> bool) -> bool {
        let mut remaining = BTreeSet::new();
        for ty in &bound.types {
            let Some(resolved) = self.layout.stable(ty).filter(&endpoint) else {
                return false;
            };
            remaining.insert(resolved.ty);
        }
        let mut pending: Vec<_> = bound
            .roots
            .iter()
            .filter_map(|port| {
                let root = &self.entries[port.0 as usize];
                (bound.region != ReachKind::Referent || root.allocation).then(|| root.ty.clone())
            })
            .collect();
        let mut visited = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            remaining.remove(&ty);
            if remaining.is_empty() {
                return true;
            }
            if bound.region != ReachKind::Cell && !visited.contains(&ty) {
                self.layout
                    .children(&ty, &mut |child| pending.push(child.ty));
                visited.insert(ty);
            }
        }
        false
    }

    fn dependencies(&self, dependencies: &[ReceiverDependency], post: &Post) -> bool {
        dependencies.iter().all(|dependency| match dependency {
            ReceiverDependency::Known(receivers) => receivers.iter().all(|receiver| {
                self.object(receiver, post)
                    .is_some_and(|endpoint| endpoint.allocation)
            }),
            ReceiverDependency::Unresolved(bound) => self.bound(bound, |ty| ty.allocation),
        })
    }

    fn value(&self, value: &ValueRef, target: &CallEndpoint<L::Type>, post: &Post) -> bool {
        let source = match &value.source {
            ValueSource::Allocation(reference) => {
                target.allocation
                    && self
                        .object(reference, post)
                        .is_some_and(|source| source.ty == target.ty)
            }
            ValueSource::Scalar(source) => {
                !target.allocation
                    && match source {
                        ScalarSource::Independent => true,
                        ScalarSource::EntryValue(port) => {
                            self.entries[port.0 as usize].ty == target.ty
                        }
                        ScalarSource::Unresolved(ScalarSourceBound::Values(ports)) => ports
                            .iter()
                            .all(|port| self.entries[port.0 as usize].ty == target.ty),
                        ScalarSource::Unresolved(ScalarSourceBound::Reach(bound)) => {
                            self.bound(bound, |source| source.ty == target.ty)
                        }
                    }
            }
        };
        source && self.dependencies(&value.dependencies, post)
    }

    fn validate_retained(&self, post: &Post) -> Result<(), InfrastructureFailure> {
        for (object, kind, stable) in &post.retained {
            let valid = match object {
                ObjectRef::Exact(export) => self.exact_endpoint(export).is_some_and(|endpoint| {
                    (*kind == StorageKind::Wrapper || endpoint.allocation)
                        && self.layout.matches_identity(&endpoint.ty, stable)
                }),
                ObjectRef::Family(..) => self
                    .layout
                    .stable(stable)
                    .is_some_and(|endpoint| *kind == StorageKind::Wrapper || endpoint.allocation),
                ObjectRef::Entry(_) => false,
            };
            if !valid {
                let lane = if self.error { "error" } else { "success" };
                return Err(allocation_error(format!(
                    "incompatible typed call layout: retained {object:?} as {stable:?} in {lane} lane"
                )));
            }
        }
        Ok(())
    }

    fn post(&self, post: &Post) -> Result<(), InfrastructureFailure> {
        let require = |condition, category| {
            if condition {
                Ok(())
            } else {
                let lane = if self.error { "error" } else { "success" };
                Err(allocation_error(format!(
                    "incompatible typed call layout: {category} in {lane} lane"
                )))
            }
        };
        self.validate_retained(post)?;
        require(
            post.bindings.iter().all(|(wrapper, edit)| {
                let Some(target) = self.object(wrapper, post) else {
                    return false;
                };
                let (source, route) = match edit {
                    BindingPost::OwnSlot => return true,
                    BindingPost::Link(wrapper, route) => (self.object(wrapper, post), route),
                    BindingPost::Projected(cell, route) => (self.cell(cell, post), route),
                };
                source.is_some_and(|source| source.ty == target.ty)
                    && self.dependencies(&route.dependencies, post)
            }),
            "binding",
        )?;
        for (target, edit) in &post.cells {
            let lane = if self.error { "error" } else { "success" };
            let endpoint = self.cell(target, post).ok_or_else(|| {
                allocation_error(format!(
                    "incompatible typed call layout: cell {target:?} in {lane} lane"
                ))
            })?;
            let (CellPost::Replace(values) | CellPost::WeakJoin(values)) = edit;
            for value in values.iter().flatten() {
                if !self.value(value, &endpoint, post) {
                    return Err(allocation_error(format!(
                        "incompatible typed call layout: cell {target:?} has payload {value:?} in {lane} lane"
                    )));
                }
            }
        }
        require(
            post.families.iter().all(|(family, edit)| {
                self.field(&family.receiver, family.selector, post)
                    .is_some_and(|target| {
                        let (FamilyPost::Extend(members) | FamilyPost::ReplaceAll(members)) = edit;
                        members
                            .iter()
                            .all(|(member, _)| self.value(member, &target, post))
                    })
            }),
            "family member",
        )?;
        require(
            post.results.iter().all(|(slot, value)| {
                self.layout
                    .result(self.error, *slot)
                    .is_some_and(|target| self.value(value, &target, post))
            }),
            "result payload",
        )?;
        require(
            match &post.coverage {
                Coverage::Complete => true,
                Coverage::Unresolved(Footprint::WritableReach(bound)) => {
                    self.bound(bound, |_| true)
                }
                Coverage::Unresolved(Footprint::Locations(locations)) => {
                    locations.iter().all(|location| {
                        match location {
                            OutputPort::Binding(wrapper) => self.object(wrapper, post),
                            OutputPort::Cell(cell) => self.cell(cell, post),
                            OutputPort::Family(family) => {
                                self.field(&family.receiver, family.selector, post)
                            }
                            OutputPort::Result(slot) => self.layout.result(self.error, *slot),
                        }
                        .is_some()
                    })
                }
            },
            "uncertainty footprint",
        )
    }

    fn lane(&self, lane: &Lane) -> Result<(), InfrastructureFailure> {
        match lane {
            Lane::Bottom => Ok(()),
            Lane::Decision(test) => {
                if let Predicate::Equal(kind, left, right) = test.predicate {
                    let left = &self.entries[left.0 as usize];
                    let right = &self.entries[right.0 as usize];
                    if left.ty != right.ty || kind == IdentityKind::Allocation && !left.allocation {
                        return Err(allocation_error("incompatible endpoints in typed guard"));
                    }
                }
                self.lane(&test.no)?;
                self.lane(&test.yes)
            }
            Lane::Returning(product) => product
                .components()
                .iter()
                .flat_map(|component| &component.alternatives)
                .try_for_each(|post| self.post(post)),
        }
    }
}

impl Shape<'_> {
    fn port(&self, id: PortId) -> Option<&EntryPort> {
        self.ports.get(id.0 as usize)
    }

    fn validate_ports(&self) -> Result<(), InfrastructureFailure> {
        let mut seen = BTreeSet::new();
        for port in self.ports {
            let valid = match port {
                EntryPort::Wrapper(param)
                | EntryPort::Immediate(param)
                | EntryPort::Terminal(param)
                | EntryPort::Value {
                    parameter: param, ..
                } => *param < self.parameters.len(),
                EntryPort::Edge(holder, _) => {
                    matches!(self.port(*holder), Some(EntryPort::Value { .. }))
                }
            };
            if !valid || !seen.insert(port) {
                return Err(allocation_error("invalid or duplicate typed entry port"));
            }
        }
        Ok(())
    }

    fn identity(&self, port: PortId, kind: IdentityKind) -> bool {
        match kind {
            IdentityKind::Wrapper => matches!(
                self.port(port),
                Some(EntryPort::Wrapper(_) | EntryPort::Immediate(_))
            ),
            IdentityKind::Cell => matches!(
                self.port(port),
                Some(EntryPort::Immediate(_) | EntryPort::Terminal(_) | EntryPort::Edge(..))
            ),
            IdentityKind::Allocation => self.entry_value(port),
        }
    }

    fn validate_lane(
        &self,
        lane: &Lane,
        slots: Option<usize>,
        earlier: Option<Predicate>,
        path: &mut Vec<Literal>,
    ) -> Result<(), InfrastructureFailure> {
        if matches!(lane, Lane::Bottom) {
            return Ok(());
        }
        let Some(slots) = slots else {
            return Err(allocation_error("undeclared typed call outcome can return"));
        };
        match lane {
            Lane::Bottom => Ok(()),
            Lane::Returning(product) => {
                let mut facts = GuardFacts::default();
                for &literal in path.iter() {
                    if !facts.assume(literal) {
                        return Err(allocation_error(
                            "returning typed product under a contradictory guard",
                        ));
                    }
                }
                self.validate_product(product, slots, &facts)
            }
            Lane::Decision(test) => {
                let predicate = test.predicate;
                let valid = match predicate {
                    Predicate::Role(port, _) => {
                        matches!(self.port(port), Some(EntryPort::Wrapper(_)))
                    }
                    Predicate::Equal(kind, left, right) => {
                        left < right && self.identity(left, kind) && self.identity(right, kind)
                    }
                    Predicate::Writable(port) => self.port(port).is_some(),
                    Predicate::DerivedFrom(left, right) => {
                        self.port(left).is_some() && self.port(right).is_some()
                    }
                };
                if !valid
                    || earlier.is_some_and(|earlier| earlier >= predicate)
                    || test.no == test.yes
                {
                    return Err(allocation_error("invalid typed guard decision"));
                }
                for (arm, yes) in [(&test.no, false), (&test.yes, true)] {
                    path.push((predicate, yes));
                    self.validate_lane(arm, Some(slots), Some(predicate), path)?;
                    path.pop();
                }
                Ok(())
            }
        }
    }

    fn validate_product(
        &self,
        product: &Product,
        slots: usize,
        facts: &GuardFacts,
    ) -> Result<(), InfrastructureFailure> {
        let mut results = BTreeSet::new();
        let mut owners = BTreeMap::new();
        let equivalents = self.cell_equivalences(facts);
        let mut reaches: Vec<(SupportId, &ReachBound)> = Vec::new();
        for component in product.components() {
            if component.support.0 as usize >= self.support_count
                || component.alternatives.is_empty()
                || !strictly_sorted(&component.alternatives)
            {
                return Err(allocation_error("invalid typed support component"));
            }
            let result_slots: BTreeSet<_> = component.alternatives[0].result_slots().collect();
            for post in &component.alternatives {
                self.validate_post(post, slots, facts)?;
                let mut locations = BTreeSet::new();
                for location in post
                    .bindings
                    .iter()
                    .map(|(target, _)| OutputPort::Binding(target.clone()))
                    .chain(
                        post.cells
                            .iter()
                            .map(|(target, _)| OutputPort::Cell(target.clone())),
                    )
                    .chain(
                        post.families
                            .iter()
                            .map(|(target, _)| OutputPort::Family(target.clone())),
                    )
                    .chain(
                        post.results
                            .iter()
                            .map(|(slot, _)| OutputPort::Result(*slot)),
                    )
                {
                    let location = Self::canonical_location(location, facts, &equivalents);
                    if !locations.insert(location.clone())
                        || owners
                            .insert(location, component.support)
                            .is_some_and(|owner| owner != component.support)
                    {
                        return Err(allocation_error(
                            "simultaneous typed posts update the same location",
                        ));
                    }
                }
                if let Coverage::Unresolved(Footprint::Locations(bounds)) = &post.coverage {
                    for location in bounds {
                        let location =
                            Self::canonical_location(location.clone(), facts, &equivalents);
                        if owners
                            .insert(location, component.support)
                            .is_some_and(|owner| owner != component.support)
                        {
                            return Err(allocation_error(
                                "typed uncertainty crosses support ownership",
                            ));
                        }
                    }
                }
                if let Coverage::Unresolved(Footprint::WritableReach(bound)) = &post.coverage {
                    for &root in &bound.roots {
                        if let Some(location) =
                            self.reach_root_location(root, bound.region, facts, &equivalents)
                            && owners
                                .insert(location, component.support)
                                .is_some_and(|owner| owner != component.support)
                        {
                            return Err(allocation_error("typed reach crosses support ownership"));
                        }
                    }
                    if reaches.iter().any(|&(support, earlier)| {
                        support != component.support
                            && earlier
                                .types
                                .iter()
                                .any(|ty| bound.types.binary_search(ty).is_ok())
                            && earlier.roots.iter().any(|&left| {
                                bound.roots.iter().any(|&right| {
                                    self.reach_roots_overlap(
                                        left,
                                        earlier.region,
                                        right,
                                        bound.region,
                                        facts,
                                        &equivalents,
                                    )
                                })
                            })
                    }) {
                        return Err(allocation_error("typed reaches cross support ownership"));
                    }
                    reaches.push((component.support, bound));
                }
                if post.result_slots().collect::<BTreeSet<_>>() != result_slots {
                    return Err(allocation_error(
                        "typed component alternatives disagree on result slots",
                    ));
                }
            }
            for slot in result_slots {
                if !results.insert(slot) {
                    return Err(allocation_error(
                        "typed result appears in more than one component",
                    ));
                }
            }
        }
        for (support, reach) in reaches {
            if owners.iter().any(|(location, &owner)| {
                owner != support
                    && reach.roots.iter().any(|&root| {
                        self.reach_covers_location(root, reach.region, location, facts)
                    })
            }) {
                return Err(allocation_error("typed reach crosses support ownership"));
            }
        }
        if results.len() == slots {
            Ok(())
        } else {
            Err(allocation_error(
                "typed outcome is missing result descriptors",
            ))
        }
    }

    fn reach_root_location(
        &self,
        root: PortId,
        region: ReachKind,
        facts: &GuardFacts,
        equivalents: &BTreeMap<CellPort, CellPort>,
    ) -> Option<OutputPort> {
        let location = match (region, self.port(root)?) {
            (ReachKind::Cell | ReachKind::EntryStructure, EntryPort::Terminal(_)) => {
                OutputPort::Cell(CellPort::Terminal(root))
            }
            (ReachKind::Cell | ReachKind::EntryStructure, EntryPort::Edge(holder, selector)) => {
                let receiver = ObjectRef::Entry(*holder);
                if selector.is_collapsed() {
                    OutputPort::Family(FamilyPort {
                        receiver,
                        selector: *selector,
                    })
                } else {
                    OutputPort::Cell(CellPort::Edge(receiver, *selector))
                }
            }
            (ReachKind::EntryStructure, EntryPort::Wrapper(_) | EntryPort::Immediate(_)) => {
                OutputPort::Binding(ObjectRef::Entry(root))
            }
            (ReachKind::Cell, EntryPort::Wrapper(_) | EntryPort::Immediate(_)) => {
                OutputPort::Cell(CellPort::Slot(self.own_slot(root, facts)?))
            }
            _ => return None,
        };
        Some(Self::canonical_location(location, facts, equivalents))
    }

    fn reach_covers_location(
        &self,
        root: PortId,
        region: ReachKind,
        location: &OutputPort,
        facts: &GuardFacts,
    ) -> bool {
        if region == ReachKind::Cell {
            return false; // The exact root location is already in the owner map.
        }
        let holder = match location {
            OutputPort::Cell(CellPort::Edge(ObjectRef::Entry(holder), _))
            | OutputPort::Family(FamilyPort {
                receiver: ObjectRef::Entry(holder),
                ..
            }) => *holder,
            _ => return false,
        };
        self.reaches_value(root, holder, facts)
    }

    fn reaches_value(&self, root: PortId, value: PortId, facts: &GuardFacts) -> bool {
        let is_below = |ancestor, descendant| {
            matches!(
                (self.port(ancestor), self.port(descendant)),
                (
                    Some(EntryPort::Value { parameter: from, path: prefix }),
                    Some(EntryPort::Value { parameter: to, path }),
                ) if from == to && path.starts_with(prefix)
            )
        };
        let directly_reaches = |value| {
            let Some(EntryPort::Value { parameter, path }) = self.port(value) else {
                return false;
            };
            match self.port(root) {
                Some(EntryPort::Value { .. }) => is_below(root, value),
                Some(
                    EntryPort::Wrapper(root)
                    | EntryPort::Immediate(root)
                    | EntryPort::Terminal(root),
                ) => root == parameter,
                Some(EntryPort::Edge(holder, selector)) => {
                    matches!(self.port(*holder),
                        Some(EntryPort::Value { parameter: from, path: prefix })
                            if from == parameter && path.strip_prefix(&prefix[..])
                                .is_some_and(|relative| relative.first() == Some(selector)))
                }
                None => false,
            }
        };
        if directly_reaches(value) {
            return true;
        }
        // Walk backwards from the accessed holder. Equality of any ancestor
        // makes all descendants behind its selectors reachable from either name.
        let mut visited = BTreeSet::new();
        let mut pending = Vec::new();
        let mut follow_equal_ancestors = |value, pending: &mut Vec<_>| {
            for (&(kind, child), &parent) in &facts.parents {
                if kind != IdentityKind::Allocation {
                    continue;
                }
                if is_below(child, value) && visited.insert(parent) {
                    pending.push(parent);
                }
                if is_below(parent, value) && visited.insert(child) {
                    pending.push(child);
                }
            }
        };
        follow_equal_ancestors(value, &mut pending);
        while let Some(value) = pending.pop() {
            if directly_reaches(value) {
                return true;
            }
            follow_equal_ancestors(value, &mut pending);
        }
        false
    }

    fn reach_roots_overlap(
        &self,
        left: PortId,
        left_region: ReachKind,
        right: PortId,
        right_region: ReachKind,
        facts: &GuardFacts,
        equivalents: &BTreeMap<CellPort, CellPort>,
    ) -> bool {
        let left_location = self.reach_root_location(left, left_region, facts, equivalents);
        let right_location = self.reach_root_location(right, right_region, facts, equivalents);
        if left_location.is_some() && left_location == right_location
            || left_location.as_ref().is_some_and(|location| {
                self.reach_covers_location(right, right_region, location, facts)
            })
            || right_location.as_ref().is_some_and(|location| {
                self.reach_covers_location(left, left_region, location, facts)
            })
        {
            return true;
        }
        match (self.port(left), self.port(right)) {
            (Some(EntryPort::Value { .. }), Some(EntryPort::Value { .. })) => {
                left_region != ReachKind::Cell
                    && right_region != ReachKind::Cell
                    && (self.reaches_value(left, right, facts)
                        || self.reaches_value(right, left, facts))
            }
            _ => left == right && left_region == right_region,
        }
    }

    fn own_slot(&self, port: PortId, facts: &GuardFacts) -> Option<ObjectRef> {
        let wrapper = match self.port(port)? {
            EntryPort::Wrapper(_) => port,
            EntryPort::Immediate(parameter) | EntryPort::Terminal(parameter) => {
                let wrapper = self
                    .ports
                    .iter()
                    .position(|port| port == &EntryPort::Wrapper(*parameter))?;
                PortId(wrapper as u32)
            }
            _ => return None,
        };
        (facts.meaning(Predicate::Role(wrapper, Role::Slot)) == Some(true))
            .then_some(ObjectRef::Entry(wrapper))
    }

    fn entry_value(&self, port: PortId) -> bool {
        matches!(self.port(port), Some(EntryPort::Value { .. }))
    }

    fn canonical_object(reference: ObjectRef, kind: IdentityKind, facts: &GuardFacts) -> ObjectRef {
        match reference {
            ObjectRef::Entry(port) => ObjectRef::Entry(facts.representative(kind, port)),
            retained => retained,
        }
    }

    fn canonical_cell(cell: CellPort, facts: &GuardFacts) -> CellPort {
        match cell {
            CellPort::Slot(wrapper) => CellPort::Slot(Self::canonical_object(
                wrapper,
                IdentityKind::Wrapper,
                facts,
            )),
            CellPort::Edge(holder, field) => CellPort::Edge(
                Self::canonical_object(holder, IdentityKind::Allocation, facts),
                field,
            ),
            terminal => terminal,
        }
    }

    fn cell_equivalences(&self, facts: &GuardFacts) -> BTreeMap<CellPort, CellPort> {
        let mut equivalents = BTreeMap::new();
        let at_entry = |id| {
            let cell = match self.port(id)? {
                EntryPort::Terminal(_) => self
                    .own_slot(id, facts)
                    .map_or(CellPort::Terminal(id), CellPort::Slot),
                EntryPort::Edge(holder, selector) if !selector.is_collapsed() => {
                    CellPort::Edge(ObjectRef::Entry(*holder), *selector)
                }
                EntryPort::Immediate(_) => CellPort::Slot(self.own_slot(id, facts)?),
                _ => return None,
            };
            Some(Self::canonical_cell(cell, facts))
        };
        if !facts.roles.is_empty() {
            for (index, port) in self.ports.iter().enumerate() {
                if matches!(port, EntryPort::Terminal(_)) {
                    let id = PortId(index as u32);
                    if let Some(slot @ CellPort::Slot(_)) = at_entry(id) {
                        equivalents.insert(CellPort::Terminal(id), slot);
                    }
                }
            }
        }
        let mut classes = BTreeMap::new();
        for &(kind, port) in facts.parents.keys() {
            if kind != IdentityKind::Cell {
                continue;
            }
            let Some(cell) = at_entry(port) else { continue };
            let class = facts.representative(kind, port);
            let first = classes
                .entry(class)
                .or_insert_with(|| at_entry(class).unwrap_or_else(|| cell.clone()));
            let a = cell_representative(&equivalents, first.clone());
            let b = cell_representative(&equivalents, cell);
            if a < b {
                equivalents.insert(b, a);
            } else if b < a {
                equivalents.insert(a, b);
            }
        }
        equivalents
    }

    fn canonical_location(
        location: OutputPort,
        facts: &GuardFacts,
        equivalents: &BTreeMap<CellPort, CellPort>,
    ) -> OutputPort {
        match location {
            OutputPort::Binding(wrapper) => OutputPort::Binding(Self::canonical_object(
                wrapper,
                IdentityKind::Wrapper,
                facts,
            )),
            OutputPort::Family(mut family) => {
                family.receiver =
                    Self::canonical_object(family.receiver, IdentityKind::Allocation, facts);
                OutputPort::Family(family)
            }
            OutputPort::Cell(cell) => OutputPort::Cell(cell_representative(
                equivalents,
                Self::canonical_cell(cell, facts),
            )),
            result => result,
        }
    }

    fn writable(&self, port: PortId) -> bool {
        let param = match self.port(port) {
            Some(EntryPort::Edge(holder, _)) => return self.writable(*holder),
            Some(
                EntryPort::Wrapper(param)
                | EntryPort::Immediate(param)
                | EntryPort::Terminal(param)
                | EntryPort::Value {
                    parameter: param, ..
                },
            ) => *param,
            None => return false,
        };
        self.parameters.get(param) == Some(&PublicCallParameterAccess::Mutable)
    }

    fn validate_retained_objects(&self, post: &Post, slots: usize) -> bool {
        strictly_sorted(post.retained.iter().map(|(reference, _, _)| reference))
            && post
                .retained
                .iter()
                .all(|(reference, kind, ty)| match reference {
                    ObjectRef::Exact(ExportPort::Result(result)) => {
                        *kind == StorageKind::Allocation
                            && result.slot < slots
                            && result.path.iter().all(|selector| !selector.is_collapsed())
                    }
                    ObjectRef::Exact(
                        ExportPort::Binding(param) | ExportPort::PrivateTerminal(param),
                    ) => *kind == StorageKind::Wrapper && *param < self.parameters.len(),
                    ObjectRef::Exact(ExportPort::Cell(path)) => {
                        *kind == StorageKind::Allocation
                            && path.index < self.parameters.len()
                            && path.path.iter().all(|selector| !selector.is_collapsed())
                    }
                    ObjectRef::Family(family_kind, family_type) => {
                        family_kind == kind && family_type == ty
                    }
                    ObjectRef::Entry(_) => false,
                })
    }

    fn validate_post(
        &self,
        post: &Post,
        slots: usize,
        facts: &GuardFacts,
    ) -> Result<(), InfrastructureFailure> {
        let writable_port =
            |port| self.writable(port) && facts.meaning(Predicate::Writable(port)) != Some(false);
        let entry_slot = |wrapper: &ObjectRef| match wrapper {
            ObjectRef::Entry(port) if matches!(self.port(*port), Some(EntryPort::Wrapper(_))) => {
                facts.meaning(Predicate::Role(*port, Role::Slot)) != Some(false)
                    || post
                        .bindings
                        .binary_search_by(|(target, _)| target.cmp(wrapper))
                        .is_ok_and(|index| matches!(post.bindings[index].1, BindingPost::OwnSlot))
            }
            _ => true,
        };
        let object = |reference: &ObjectRef, kind| match reference {
            ObjectRef::Entry(port) => self.identity(
                *port,
                match kind {
                    StorageKind::Wrapper => IdentityKind::Wrapper,
                    StorageKind::Allocation => IdentityKind::Allocation,
                },
            ),
            ObjectRef::Exact(_) | ObjectRef::Family(..) => post
                .retained
                .binary_search_by(|(key, _, _)| key.cmp(reference))
                .ok()
                .is_some_and(|index| post.retained[index].1 == kind),
        };
        let singleton = |reference: &ObjectRef| !matches!(reference, ObjectRef::Family(..));
        let writable = |reference: &ObjectRef| match reference {
            ObjectRef::Entry(port) => writable_port(*port),
            _ => true,
        };
        let cell = |cell: &CellPort, write: bool| match cell {
            CellPort::Slot(wrapper) => {
                object(wrapper, StorageKind::Wrapper)
                    && entry_slot(wrapper)
                    && (!write || writable(wrapper))
            }
            CellPort::Terminal(port) => {
                matches!(self.port(*port), Some(EntryPort::Terminal(_)))
                    && (!write || writable_port(*port))
            }
            CellPort::Edge(holder, field) => {
                !field.is_collapsed()
                    && object(holder, StorageKind::Allocation)
                    && (!write
                        || writable(holder)
                            && match holder {
                                ObjectRef::Entry(port) => {
                                    !facts.access.iter().any(|(&predicate, &holds)| {
                                        matches!(predicate, Predicate::Writable(edge) if !holds
                                && self.port(edge) == Some(&EntryPort::Edge(*port, *field)))
                                    })
                                }
                                _ => true,
                            })
            }
        };
        let family = |port: &FamilyPort| {
            port.selector.is_collapsed()
                && object(&port.receiver, StorageKind::Allocation)
                && writable(&port.receiver)
        };
        let access = |access: &PathAccess| match &access.derived_from {
            None => true,
            Some(Derivation::Entry(port)) => self.port(*port).is_some(),
            Some(Derivation::Retained(reference)) => {
                matches!(reference, ObjectRef::Exact(_)) && object(reference, StorageKind::Wrapper)
            }
        };
        let reach = |bound: &ReachBound, write: bool| {
            !bound.types.is_empty()
                && strictly_sorted(&bound.types)
                && !bound.roots.is_empty()
                && strictly_sorted(&bound.roots)
                && bound
                    .roots
                    .iter()
                    .all(|&port| self.port(port).is_some() && (!write || writable_port(port)))
        };
        let dependencies = |dependencies: &[ReceiverDependency]| {
            strictly_sorted(dependencies)
                && dependencies.iter().all(|dependency| match dependency {
                    ReceiverDependency::Known(receivers) => {
                        !receivers.is_empty()
                            && strictly_sorted(receivers)
                            && receivers
                                .iter()
                                .all(|reference| object(reference, StorageKind::Allocation))
                    }
                    ReceiverDependency::Unresolved(bound) => reach(bound, false),
                })
        };
        let retained_access = |route: &RetainedRoute| {
            route.access.restriction.is_some()
                && access(&route.access)
                && dependencies(&route.dependencies)
        };
        let value = |value: &ValueRef| {
            let source = match &value.source {
                ValueSource::Allocation(reference) => object(reference, StorageKind::Allocation),
                ValueSource::Scalar(ScalarSource::Independent) => true,
                ValueSource::Scalar(ScalarSource::EntryValue(port)) => self.entry_value(*port),
                ValueSource::Scalar(ScalarSource::Unresolved(ScalarSourceBound::Values(ports))) => {
                    !ports.is_empty()
                        && strictly_sorted(ports)
                        && ports.iter().all(|&port| self.entry_value(port))
                }
                ValueSource::Scalar(ScalarSource::Unresolved(ScalarSourceBound::Reach(bound))) => {
                    reach(bound, false)
                }
            };
            source && access(&value.access) && dependencies(&value.dependencies)
        };
        let contents = |alternatives: &[Option<ValueRef>]| {
            !alternatives.is_empty()
                && strictly_sorted(alternatives)
                && alternatives.iter().flatten().all(value)
        };
        let result = |slot| slot < slots;
        let coverage = match &post.coverage {
            Coverage::Complete => true,
            Coverage::Unresolved(Footprint::WritableReach(bound)) => reach(bound, true),
            Coverage::Unresolved(Footprint::Locations(locations)) => {
                !locations.is_empty()
                    && strictly_sorted(locations)
                    && locations.iter().all(|location| match location {
                        OutputPort::Result(slot) => result(*slot),
                        OutputPort::Binding(wrapper) => {
                            object(wrapper, StorageKind::Wrapper) && writable(wrapper)
                        }
                        OutputPort::Cell(target) => cell(target, true),
                        OutputPort::Family(port) => family(port),
                    })
            }
        };
        let valid = coverage
            && self.validate_retained_objects(post, slots)
            && strictly_sorted(post.bindings.iter().map(|(target, _)| target))
            && post.bindings.iter().all(|(target, binding)| {
                object(target, StorageKind::Wrapper)
                    && writable(target)
                    && match binding {
                        BindingPost::OwnSlot => singleton(target),
                        BindingPost::Link(source, route) => {
                            object(source, StorageKind::Wrapper) && retained_access(route)
                        }
                        BindingPost::Projected(target, route) => {
                            !matches!(target, CellPort::Slot(_))
                                && cell(target, false)
                                && retained_access(route)
                        }
                    }
            })
            && strictly_sorted(post.cells.iter().map(|(target, _)| target))
            && post.cells.iter().all(|(target, payload)| {
                cell(target, true)
                    && match payload {
                        CellPost::Replace(payload) => match target {
                            CellPort::Slot(wrapper) | CellPort::Edge(wrapper, _) => {
                                singleton(wrapper) && contents(payload)
                            }
                            CellPort::Terminal(_) => contents(payload),
                        },
                        CellPost::WeakJoin(payload) => contents(payload),
                    }
            })
            && strictly_sorted(post.families.iter().map(|(port, _)| port))
            && post.families.iter().all(|(port, edit)| {
                family(port)
                    && match edit {
                        FamilyPost::Extend(members) => {
                            !members.is_empty()
                                && strictly_sorted(members.iter().map(|(member, _)| member))
                                && members.iter().all(|(member, _)| value(member))
                        }
                        FamilyPost::ReplaceAll(members) => {
                            singleton(&port.receiver)
                                && strictly_sorted(members.iter().map(|(member, _)| member))
                                && members.iter().all(|(member, _)| value(member))
                        }
                    }
            })
            && strictly_sorted(post.results.iter().map(|(slot, _)| slot))
            && post
                .results
                .iter()
                .all(|(slot, payload)| result(*slot) && value(payload));
        if valid {
            Ok(())
        } else {
            Err(allocation_error(
                "invalid typed postcondition shape or reference",
            ))
        }
    }
}

fn cell_representative(equivalents: &BTreeMap<CellPort, CellPort>, cell: CellPort) -> CellPort {
    let Some(mut representative) = equivalents.get(&cell) else {
        return cell;
    };
    while let Some(next) = equivalents.get(representative) {
        representative = next;
    }
    representative.clone()
}
