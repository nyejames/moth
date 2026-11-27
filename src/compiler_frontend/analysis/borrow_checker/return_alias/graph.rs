//! Indexed value paths and same-execution sharing used only while extracting a call lane.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationSource, CallField, CallParameterPath, CallSharingCondition,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Path = Vec<CallField>;
pub(super) type Conditions = BTreeSet<CallSharingCondition>;
pub(super) type Pairs = BTreeMap<(Cell, Cell), Conditions>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owner {
    Local(u32),
    /// A projected alias's captured holder, keyed by its acquisition site so alternatives
    /// joined from different acquisitions keep their own holder and selector.
    Alias(u32, Site),
    Store(CallParameterPath),
    Scratch(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Cell {
    pub owner: Owner,
    pub path: Path,
}

impl Cell {
    pub fn root(owner: Owner) -> Self {
        Self {
            owner,
            path: Vec::new(),
        }
    }

    pub fn at(&self, path: &[CallField]) -> Self {
        let mut cell = self.clone();
        cell.path.extend_from_slice(path);
        cell
    }

    pub fn below<'a>(&'a self, root: &Self) -> Option<&'a [CallField]> {
        if self.owner != root.owner {
            return None;
        }
        self.path.strip_prefix(root.path.as_slice())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Site {
    Value(u32),
    Operation(u32, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Origin {
    Parameter(CallParameterPath),
    Fresh(Site, usize, Path),
    // ABI topology is allowed to share across slots and compatible paths of one call.
    Opaque(Site, TypeId, Option<(usize, Path)>),
}

impl Origin {
    pub fn into_source(self) -> CallAllocationSource {
        match self {
            Self::Parameter(path) => CallAllocationSource::Parameter(path),
            Self::Fresh(..) => CallAllocationSource::Fresh,
            Self::Opaque(..) => CallAllocationSource::OpaqueFresh,
        }
    }

    fn site(&self) -> Option<Site> {
        match *self {
            Self::Fresh(site, ..) | Self::Opaque(site, ..) => Some(site),
            Self::Parameter(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Node {
    pub ty: TypeId,
    pub sources: BTreeSet<Origin>,
}

/// A snapshot to be installed in one parallel transfer. `refs` transport existing pair
/// relationships; `births` introduce new ones at a parameter, construction or call event.
/// Neither is reconstructed from source rows at a control-flow join.
#[derive(Debug)]
pub(super) struct Spec {
    pub cell: Cell,
    pub node: Node,
    pub refs: Vec<Cell>,
    pub births: Vec<Origin>,
    pub detached: bool,
}

impl Spec {
    pub fn born(cell: Cell, ty: TypeId, origin: Origin) -> Self {
        Self {
            cell,
            node: Node {
                ty,
                sources: BTreeSet::from([origin.clone()]),
            },
            refs: Vec::new(),
            births: vec![origin],
            detached: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Graph {
    pub nodes: BTreeMap<Cell, Node>,
    pub pairs: Pairs,
    seen_sites: BTreeSet<Site>,
    repeated: BTreeSet<Site>,
    many: BTreeSet<Origin>,
    next_temporary: usize,
    pub aliases: BTreeMap<u32, BTreeSet<Cell>>,
}

impl Graph {
    pub fn temporary(&mut self) -> Cell {
        let temporary = Cell::root(Owner::Scratch(self.next_temporary));
        self.next_temporary += 1;
        temporary
    }

    pub fn retire_temporaries(&mut self) {
        if self.next_temporary == 0 {
            return;
        }
        self.next_temporary = 0;
        if !self
            .nodes
            .last_key_value()
            .is_some_and(|(cell, _)| matches!(cell.owner, Owner::Scratch(_)))
        {
            return;
        }
        self.nodes
            .retain(|cell, _| !matches!(cell.owner, Owner::Scratch(_)));
        self.pairs.retain(|(a, b), _| {
            !matches!(a.owner, Owner::Scratch(_)) && !matches!(b.owner, Owner::Scratch(_))
        });
    }

    /// Retire the local's name, except when a live write-through alias still uses it as
    /// a binding cell or needs its captured projected holder. The caller retries the latter.
    pub fn retire_local(&mut self, id: u32) -> bool {
        self.aliases.remove(&id);
        let owned = |owner: &Owner| matches!(owner, Owner::Local(local) | Owner::Alias(local, _) if *local == id);
        if self
            .aliases
            .values()
            .flatten()
            .any(|cell| owned(&cell.owner))
        {
            return false;
        }
        self.clear_local(id);
        true
    }

    /// Clear the local's own cells and every projected holder it captured.
    pub fn clear_local(&mut self, id: u32) {
        let owned = |cell: &Cell| matches!(cell.owner, Owner::Local(local) | Owner::Alias(local, _) if local == id);
        // Cells order by owner first, so each owner family starts at its least root.
        let present = |owner| {
            self.nodes
                .range(Cell::root(owner)..)
                .next()
                .is_some_and(|(cell, _)| owned(cell))
        };
        if !present(Owner::Local(id)) && !present(Owner::Alias(id, Site::Value(0))) {
            return;
        }
        self.nodes.retain(|cell, _| !owned(cell));
        self.pairs.retain(|(a, b), _| !owned(a) && !owned(b));
    }

    pub fn clear(&mut self, roots: &[Cell]) {
        if roots.iter().all(|root| !self.nodes.contains_key(root)) {
            return;
        }
        let retain = |cell: &Cell| roots.iter().all(|root| cell.below(root).is_none());
        self.nodes.retain(|cell, _| retain(cell));
        self.pairs.retain(|(a, b), _| retain(a) && retain(b));
    }

    /// Merge a control-flow predecessor's state.
    pub fn join(&mut self, other: Self) -> bool {
        use std::collections::btree_map::Entry;
        let mut changed = false;
        for (cell, node) in other.nodes {
            match self.nodes.entry(cell) {
                Entry::Occupied(mut entry) => {
                    let sources = &mut entry.get_mut().sources;
                    let len = sources.len();
                    sources.extend(node.sources);
                    changed |= len != sources.len();
                }
                Entry::Vacant(entry) => {
                    entry.insert(node);
                    changed = true;
                }
            }
        }
        for (pair, conditions) in other.pairs {
            let sources = self.pairs.entry(pair).or_default();
            let len = sources.len();
            sources.extend(conditions);
            changed |= len != sources.len();
        }
        // A local with no alias entry is a plain slot on that path. Keep that alternative as the
        // local's own cell, so a write through the joined alias stays weak and also reaches
        // the slot, as the live checker's slot-or-alias mode does.
        for (local, ours) in &mut self.aliases {
            if !other.aliases.contains_key(local) {
                changed |= ours.insert(Cell::root(Owner::Local(*local)));
            }
        }
        for (local, mut cells) in other.aliases {
            match self.aliases.entry(local) {
                Entry::Occupied(mut entry) => {
                    let ours = entry.get_mut();
                    let len = ours.len();
                    ours.append(&mut cells);
                    changed |= len != ours.len();
                }
                Entry::Vacant(entry) => {
                    cells.insert(Cell::root(Owner::Local(local)));
                    entry.insert(cells);
                    changed = true;
                }
            }
        }
        let repeated = self.repeated.len();
        let many = self.many.len();
        self.seen_sites.extend(other.seen_sites);
        self.repeated.extend(other.repeated);
        self.many.extend(other.many);
        changed || repeated != self.repeated.len() || many != self.many.len()
    }

    pub fn conditions(&self, a: &Cell, b: &Cell) -> Conditions {
        if a == b {
            return self
                .nodes
                .contains_key(a)
                .then_some(CallSharingCondition::Always)
                .into_iter()
                .collect();
        }
        self.pairs.get(&pair(a, b)).cloned().unwrap_or_default()
    }

    pub fn snapshot(&self, target: &Cell, root: &Cell, dest: &mut Vec<Spec>) {
        dest.extend(self.nodes.range(root.clone()..).map_while(|(cell, node)| {
            let relative = cell.below(root)?;
            let node = node.clone();
            Some(Spec {
                cell: target.at(relative),
                node,
                refs: vec![cell.clone()],
                births: Vec::new(),
                detached: false,
            })
        }));
    }

    pub fn capture(&mut self, root: &Cell) -> Cell {
        let target = self.temporary();
        let mut rows = Vec::new();
        self.snapshot(&target, root, &mut rows);
        self.install(rows, &[], None);
        target
    }

    fn same_single_allocation(&self, a: &Cell, b: &Cell) -> bool {
        if !self.exact(a) || !self.exact(b) {
            return false;
        }
        self.nodes[a].sources.first() == self.nodes[b].sources.first()
    }

    fn exact(&self, cell: &Cell) -> bool {
        self.nodes.get(cell).is_some_and(|node| {
            let (Some(origin), 1) = (node.sources.first(), node.sources.len()) else { return false };
            !matches!(origin, Origin::Opaque(..))
                && !origin.site().is_some_and(|site| self.repeated.contains(&site))
                && !matches!(origin, Origin::Parameter(path) if path.path.iter().any(|step| step.is_collapsed()))
                && !matches!(origin, Origin::Fresh(_, _, route) if route.iter().any(|step| step.is_collapsed()))
                && !cell.path.iter().any(|step| step.is_collapsed())
        })
    }

    /// Update the cell reached through a captured holder, not the syntactic receiver binding.
    /// A parameter source names the *entry* holder; descendants of a newly stored wrapper
    /// remain underneath its store snapshot and never become writes to the old holder.
    ///
    /// Returns `false` when no allocation reaches `holder` yet. Every holder has selectors, so
    /// its type is heap-allocated and an absent node is bottom, not an untracked value: the
    /// caller must treat a definite write as an incomplete path instead of a no-op, because a
    /// later widening that resolves the holder would make the same write strong.
    ///
    /// Monotonicity under widening inputs: a cell is strongly replaced only when it and the
    /// holder both hold exactly one non-repeated allocation, the same one (must-alias). Growing
    /// sources, repeated sites or pairs can only remove cells from that set, and a weak target
    /// only gains rows. Strong eligibility therefore never depends on pair bookkeeping order.
    #[must_use]
    pub fn write(
        &mut self,
        holder: &Cell,
        key: CallField,
        value: &Cell,
        weak: bool,
        allowed: impl Fn(usize) -> bool,
    ) -> bool {
        let Some(node) = self.nodes.get(holder) else {
            return false;
        };
        if key.is_collapsed() {
            self.many.extend(node.sources.iter().cloned());
        }
        let strong_holder = !weak && !key.is_collapsed() && self.exact(holder);
        let mut targets = BTreeMap::new();
        for (cell, candidate) in &self.nodes {
            if candidate.ty != node.ty {
                continue;
            }
            let strong = strong_holder && self.same_single_allocation(cell, holder);
            if strong || cell == holder || self.pairs.contains_key(&pair(cell, holder)) {
                targets.insert(cell.at(&[key]), strong);
            }
        }
        for origin in &node.sources {
            if let Origin::Parameter(mut path) = origin.clone()
                && allowed(path.index)
            {
                path.path.push(key);
                targets.insert(Cell::root(Owner::Store(path)), strong_holder);
            }
        }
        let mut rows = Vec::new();
        let mut clear = Vec::new();
        for (target, strong) in targets {
            self.snapshot(&target, value, &mut rows);
            if strong {
                clear.push(target);
            }
        }
        self.install(rows, &clear, None);
        true
    }

    /// Remove the keyed contents of an exact holder. Like [`Self::write`], returns `false` when
    /// no allocation reaches the receiver yet, so the definite receiver access stays incomplete.
    #[must_use]
    pub fn erase(
        &mut self,
        holder: &Cell,
        keys: &[CallField],
        singleton: Option<impl FnOnce(&Origin) -> bool>,
    ) -> bool {
        if !self.nodes.contains_key(holder) {
            return false;
        }
        if !self.exact(holder) {
            return true;
        }
        let origin = self.nodes[holder].sources.first().unwrap();
        if let Some(is_singleton) = singleton
            && (self.many.contains(origin) || !is_singleton(origin))
        {
            return true;
        }
        let mut cells = self
            .nodes
            .keys()
            .filter(|cell| self.same_single_allocation(holder, cell))
            .flat_map(|cell| keys.iter().map(|&key| cell.at(&[key])))
            .collect::<Vec<_>>();
        if let Origin::Parameter(path) = origin {
            cells.extend(keys.iter().map(|&key| {
                let mut target = path.clone();
                target.path.push(key);
                Cell::root(Owner::Store(target))
            }));
        }
        self.clear(&cells);
        true
    }

    /// Apply one binding, construction, store, or multiple simultaneous edge destinations.
    /// A known call supplies its own complete new/new relation; argument substitution must not
    /// derive additional pairs by intersecting the callee's already-joined path marginals.
    pub fn install(&mut self, rows: Vec<Spec>, clear: &[Cell], known_pairs: Option<Pairs>) {
        if rows.is_empty() {
            self.clear(clear);
            return;
        }
        let has_known_pairs = known_pairs.is_some();
        let mut update = Graph {
            pairs: known_pairs.unwrap_or_default(),
            ..Graph::default()
        };
        for (index, row) in rows.iter().enumerate() {
            let can_recur = row.births.iter().any(|birth| {
                birth
                    .site()
                    .is_none_or(|site| self.seen_sites.contains(&site))
            });
            update
                .seen_sites
                .extend(row.births.iter().filter_map(Origin::site));
            let keep = |cell: &Cell| {
                row.cell != *cell && !clear.iter().any(|root| cell.below(root).is_some())
            };
            let use_refs = !row.detached && !row.refs.is_empty();
            if use_refs && self.pairs.is_empty() {
                // With no tracked relationships, an old cell only aliases itself.
                for source in row.refs.iter().filter(|source| keep(source)) {
                    update
                        .pairs
                        .entry(pair(&row.cell, source))
                        .or_default()
                        .insert(CallSharingCondition::Always);
                }
            }
            if can_recur || (use_refs && !self.pairs.is_empty()) {
                for (other, node) in &self.nodes {
                    if row.node.ty != node.ty {
                        continue;
                    }
                    let mut conditions = Conditions::new();
                    if can_recur {
                        for birth in &row.births {
                            for origin in &node.sources {
                                if birth == origin
                                    && let Some(site) = birth.site()
                                {
                                    update.repeated.insert(site);
                                }
                                conditions.extend(origin_condition(birth, origin));
                            }
                        }
                    }
                    if !keep(other) {
                        continue;
                    }
                    if use_refs {
                        for source in &row.refs {
                            conditions.extend(self.conditions(source, other));
                        }
                    }
                    if !conditions.is_empty() {
                        update
                            .pairs
                            .entry(pair(&row.cell, other))
                            .or_default()
                            .extend(conditions);
                    }
                }
            }
            if has_known_pairs {
                continue;
            }
            for other in &rows[..index] {
                if row.cell == other.cell || row.node.ty != other.node.ty {
                    continue;
                }
                let mut conditions = Conditions::new();
                if row.detached == other.detached {
                    for a in &row.refs {
                        for b in &other.refs {
                            conditions.extend(self.conditions(a, b));
                        }
                    }
                }
                for a in &row.births {
                    for b in &other.births {
                        conditions.extend(origin_condition(a, b));
                    }
                }
                if !conditions.is_empty() {
                    update
                        .pairs
                        .entry(pair(&row.cell, &other.cell))
                        .or_default()
                        .extend(conditions);
                }
            }
        }
        self.clear(clear);
        for row in rows {
            use std::collections::btree_map::Entry;
            match self.nodes.entry(row.cell) {
                Entry::Occupied(mut entry) => entry.get_mut().sources.extend(row.node.sources),
                Entry::Vacant(entry) => {
                    entry.insert(row.node);
                }
            }
        }
        // `update` holds only relationships and site bookkeeping; it is not a CFG predecessor.
        for (pair, conditions) in update.pairs {
            self.pairs.entry(pair).or_default().extend(conditions);
        }
        self.seen_sites.extend(update.seen_sites);
        self.repeated.extend(update.repeated);
    }
}

pub(super) fn pair(a: &Cell, b: &Cell) -> (Cell, Cell) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

fn origin_condition(a: &Origin, b: &Origin) -> Option<CallSharingCondition> {
    if a == b {
        return Some(CallSharingCondition::Always);
    }
    match (a, b) {
        (Origin::Parameter(a), Origin::Parameter(b)) if a <= b => {
            Some(CallSharingCondition::Parameters(a.clone(), b.clone()))
        }
        (Origin::Parameter(a), Origin::Parameter(b)) => {
            Some(CallSharingCondition::Parameters(b.clone(), a.clone()))
        }
        _ => None,
    }
}
