//! Finite call-boundary identities, guarded products and their widening order.
//!
//! A guard only partitions a relation already certified to factor by the fixed
//! supports under that guard. A joint relation must not be projected and then
//! restricted: projection has already forgotten the correlations.

use super::{
    CallField, CallParameterPath, CallResultPath, PublicCallParameterAccess, allocation_error,
    sorted_subset, strictly_sorted,
};
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
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

    /// Align the least test before joining. Tests implied by earlier typed
    /// observations have one feasible arm; they are not arbitrary don't-cares.
    pub(crate) fn union(&self, other: &Self) -> Self {
        join_lanes(self, other, &mut Vec::new(), &mut BTreeSet::new())
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

impl TypedCallAllocationFacts {
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
        self.validate(shape)?;
        next.validate(shape)?;
        if sorted_subset(&self.may_retain, &next.may_retain)
            && lane_contains(&next.success, &self.success, &mut Vec::new())
            && lane_contains(&next.error, &self.error, &mut Vec::new())
        {
            Ok(())
        } else {
            Err(allocation_error("typed call postconditions narrowed"))
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
            let first = &component.alternatives[0];
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
                if !post
                    .results
                    .iter()
                    .map(|(slot, _)| slot)
                    .eq(first.results.iter().map(|(slot, _)| slot))
                {
                    return Err(allocation_error(
                        "typed component alternatives disagree on result slots",
                    ));
                }
            }
            for &(slot, _) in &first.results {
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
                            cell(target, false) && retained_access(route)
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
