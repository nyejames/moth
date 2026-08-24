//! Typed Vec arena and parent-linked scope frames for AST local declarations.
//!
//! WHAT: `ScopeArena` owns every frame created during one AST parse context (a module,
//!       function body, template, or constant block) in a single contiguous allocation.
//!       Each frame gets a stable `ScopeFrameId` and a parent link, replacing the
//!       previous `Rc<ScopeFrame>` chain with index-based navigation.
//!
//! WHY: `Rc<ScopeFrame>` required a separate heap allocation for every frame and forced
//!      whole-frame cloning via `Rc::make_mut` whenever a child scope added a local.
//!      A typed Vec arena keeps frames in one allocation, makes parent relationships
//!      explicit, and lets `ScopeContext` clone only the current frame when necessary.
//!
//! ## Mutation rules
//!
//! - The arena is reachable through `Rc<RefCell<ScopeArena>>` held by every clone of a
//!   `ScopeContext`, but borrow guards are never exposed through parser APIs.
//! - `ScopeContext::clone()` allocates a copy of the current frame so that later
//!   `add_var` calls on the clone mutate the copy, not the original frame.
//! - Child contexts share ancestor frame IDs; only the current frame of each context is
//!   mutable from that context.

use super::*;
use crate::compiler_frontend::instrumentation::add_ast_counter;
use crate::compiler_frontend::instrumentation::frontend_counters::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Stable ID for a scope frame inside a `ScopeArena`.
///
/// IDs are created only by `ScopeArena::alloc_frame` and are valid only within the
/// arena that produced them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ScopeFrameId(u32);

impl ScopeFrameId {
    /// Return the ID as a `usize` for indexing into `ScopeArena::frames`.
    pub(crate) fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Typed Vec arena that owns all frames for one AST parse context.
///
/// WHAT: stores every scope frame in a single contiguous allocation. Frames are append-only
///       in their local declarations and immutable in their parent links.
/// WHY: avoids per-frame `Rc` allocations and makes the no-shadowing frame chain cheap to
///      traverse during name resolution.
pub(crate) struct ScopeArena {
    frames: Vec<ScopeFrame>,
}

impl ScopeArena {
    /// Build an empty arena.
    pub(crate) fn new() -> Self {
        Self { frames: Vec::new() }
    }

    /// Build an arena with pre-allocated frame storage.
    ///
    /// WHAT: reserves capacity for the arena's frame Vec before any root frame is allocated.
    /// WHY: capacity estimates should reduce Vec growth without changing frame IDs, parent links,
    ///      name lookup, or diagnostics. Initial capacity is recorded once so the capacity
    ///      counter represents total allocated frame storage, not only later growth.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let frames = Vec::with_capacity(capacity);
        let allocated_capacity = frames.capacity();

        if allocated_capacity > 0 {
            add_frontend_counter(FrontendCounter::ScopeArenaCapacity, allocated_capacity);
        }

        Self { frames }
    }

    /// Allocate a root frame with a pre-allocated declaration buffer.
    ///
    /// WHAT: seeds the root frame with capacity for declarations. The capacity is local
    ///       policy derived from the actual scope shape (for example a function parameter
    ///       count), not an externally-plumbed estimate.
    pub(crate) fn alloc_root_frame_with_capacity(&mut self, capacity: usize) -> ScopeFrameId {
        let id = self.next_id();
        let previous_capacity = self.frames.capacity();
        self.frames.push(ScopeFrame::root_with_capacity(capacity));
        self.record_frame_allocation(previous_capacity);
        id
    }

    /// Allocate a child frame linked to the provided parent.
    pub(crate) fn alloc_child_frame(&mut self, parent: ScopeFrameId) -> ScopeFrameId {
        let depth = self.frames[parent.as_usize()].depth + 1;
        let id = self.next_id();
        let previous_capacity = self.frames.capacity();
        self.frames.push(ScopeFrame::new_child(Some(parent), depth));
        self.record_frame_allocation(previous_capacity);
        id
    }

    /// Create a shallow copy of an existing frame with the same parent and declarations.
    ///
    /// WHAT: used by `ScopeContext::clone()` so that the cloned context can mutate its
    ///       own current frame without affecting the original context's frame.
    /// WHY: local declarations are stored as `Rc<Declaration>`; cloning the frame copies
    ///      only the `Rc` pointers and the name index, not the declaration payloads.
    pub(crate) fn clone_frame(&mut self, frame_id: ScopeFrameId) -> ScopeFrameId {
        let cloned = self.frames[frame_id.as_usize()].clone();
        let id = self.next_id();
        let previous_capacity = self.frames.capacity();
        self.frames.push(cloned);
        self.record_frame_allocation(previous_capacity);
        id
    }

    /// Return a shared reference to the frame with the given ID.
    pub(crate) fn frame(&self, frame_id: ScopeFrameId) -> &ScopeFrame {
        &self.frames[frame_id.as_usize()]
    }

    /// Return a mutable reference to the frame with the given ID.
    pub(crate) fn frame_mut(&mut self, frame_id: ScopeFrameId) -> &mut ScopeFrame {
        &mut self.frames[frame_id.as_usize()]
    }

    /// Resolve a name to the latest visible declaration in the frame chain.
    ///
    /// WHAT: checks the current frame first so the most recent local binding wins,
    ///       then walks parent frames until the name is found or the chain ends.
    /// WHY: Moth forbids shadowing, so there is never more than one visible binding
    ///      for a name; "latest" simply means "nearest ancestor that declares it".
    pub(crate) fn lookup(
        &self,
        frame_id: ScopeFrameId,
        name: &StringId,
    ) -> Option<(Rc<Declaration>, SourceLocation)> {
        let mut current_id = Some(frame_id);
        let mut steps: usize = 0;

        while let Some(id) = current_id {
            let frame = self.frame(id);
            if let Some(indices) = frame.local_declarations_by_name.get(name) {
                add_ast_counter(AstCounter::ScopeFrameLookupAncestorSteps, steps);
                return indices.last().map(|index| {
                    let entry = &frame.local_declarations[*index as usize];
                    (
                        Rc::clone(&entry.declaration),
                        entry.binding_location.clone(),
                    )
                });
            }

            let Some(parent_id) = frame.parent else {
                add_ast_counter(AstCounter::ScopeFrameLookupAncestorSteps, steps);
                return None;
            };
            current_id = Some(parent_id);
            steps += 1;
        }

        None
    }

    /// Return whether a declaration is an explicit compile-time constant in this frame
    /// or any ancestor frame.
    ///
    /// WHAT: walks the parent chain to find a `#` declaration matching the given path.
    /// WHY: fixed-capacity type syntax must reject foldable runtime bindings while still
    ///      allowing visible explicit constants in any ancestor scope.
    pub(crate) fn is_explicit_compile_time_constant(
        &self,
        frame_id: ScopeFrameId,
        declaration: &Declaration,
    ) -> bool {
        let mut current_id = Some(frame_id);

        while let Some(id) = current_id {
            let frame = self.frame(id);
            if frame
                .explicit_compile_time_constant_declarations
                .as_ref()
                .is_some_and(|declarations| declarations.contains(&declaration.id))
            {
                return true;
            }
            current_id = frame.parent;
        }

        false
    }

    fn next_id(&self) -> ScopeFrameId {
        ScopeFrameId(self.frames.len() as u32)
    }

    /// Record one frame allocation and any growth in the arena's frame storage.
    ///
    /// WHAT: `ActualScopeFrames` counts every frame allocated through the arena,
    ///       including clone-isolation frames. `ScopeArenaCapacity` records only
    ///       capacity growth, so repeated pushes without Vec growth do not inflate it.
    /// WHY: Phase 5 tunes capacity estimates from observed frame count and storage
    ///      pressure; these counters must be attached to the arena owner rather than
    ///      scattered across context constructors.
    fn record_frame_allocation(&self, previous_capacity: usize) {
        increment_frontend_counter(FrontendCounter::ActualScopeFrames);

        let current_capacity = self.frames.capacity();
        if current_capacity > previous_capacity {
            add_frontend_counter(
                FrontendCounter::ScopeArenaCapacity,
                current_capacity - previous_capacity,
            );
        }
    }
}

impl Default for ScopeArena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "scope_frame_tests.rs"]
mod scope_frame_tests;

/// A body-local declaration paired with its authored binding-name source location.
///
/// WHAT: stores the declaration alongside the location where the binding name was
///       authored, which differs from `Declaration::value::location` (the initializer).
/// WHY: immutable assignment diagnostics add a secondary label at the original
///      binding declaration, not at the initializer expression.
#[derive(Clone)]
pub(crate) struct LocalDeclaration {
    pub(crate) declaration: Rc<Declaration>,
    pub(crate) binding_location: SourceLocation,
}

/// One layer of local declarations in the AST scope hierarchy.
///
/// WHAT: owns the declarations authored in this scope layer and an index of their names.
///       The `parent` link is a stable `ScopeFrameId` pointing to the enclosing frame,
///       which may itself have a parent.
/// WHY: child scopes no longer need a private copy of every visible local declaration;
///      they create a new empty frame and inherit ancestor declarations through parent IDs.
#[derive(Clone)]
pub(crate) struct ScopeFrame {
    /// Declarations authored in this frame, in source order.
    ///
    /// Local declarations are stored as `LocalDeclaration` entries so that `ScopeContext::clone()`
    /// can copy the current frame without cloning the declaration payloads, while
    /// ordinary lookup can hand out a cheap `Rc<Declaration>` without holding a borrow.
    local_declarations: Vec<LocalDeclaration>,

    /// Name index for the declarations in this frame only.
    local_declarations_by_name: FxHashMap<StringId, Vec<u32>>,

    /// Declarations authored with `#` that are visible to this frame and its descendants.
    ///
    /// WHAT: body-local explicit compile-time constants live here, just like ordinary
    ///       local declarations, and participate in parent-chain lookup. `None` means the
    ///       frame declares none, which is the common case for control-flow frames.
    /// Module constants live in the top-level `ResolvedConstantSet`; this path-keyed set owns only
    /// lexical declarations, which do not have top-level declaration IDs.
    pub(crate) explicit_compile_time_constant_declarations: Option<Rc<FxHashSet<InternedPath>>>,

    /// Parent frame holding visible ancestor declarations.
    parent: Option<ScopeFrameId>,

    /// Frame depth from the root (root = 0).
    ///
    /// WHAT: used for instrumentation only; it does not affect name resolution.
    depth: usize,
}

impl ScopeFrame {
    /// Build the root frame with a pre-allocated declaration buffer.
    pub(crate) fn root_with_capacity(declarations_capacity: usize) -> Self {
        Self {
            local_declarations: Vec::with_capacity(declarations_capacity),
            local_declarations_by_name: FxHashMap::default(),
            explicit_compile_time_constant_declarations: None,
            parent: None,
            depth: 0,
        }
    }

    /// Build a child frame linked to the provided parent.
    ///
    /// WHAT: creates a new empty frame whose parent chain includes all visible ancestor
    ///       declarations without copying them.
    pub(crate) fn new_child(parent: Option<ScopeFrameId>, depth: usize) -> Self {
        Self {
            local_declarations: Vec::new(),
            local_declarations_by_name: FxHashMap::default(),
            explicit_compile_time_constant_declarations: None,
            parent,
            depth,
        }
    }

    /// Return this frame's depth in the parent chain.
    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    #[cfg(test)]
    /// Return the declarations authored in this frame only.
    pub(crate) fn local_declarations(&self) -> &[LocalDeclaration] {
        &self.local_declarations
    }

    #[cfg(test)]
    /// Return the total number of visible declarations across this frame and ancestors.
    ///
    /// WHAT: a diagnostic helper for tests and counters.
    pub(crate) fn total_declaration_count(&self, arena: &ScopeArena) -> usize {
        self.local_declarations.len()
            + self.parent.map_or(0, |parent_id| {
                arena.frame(parent_id).total_declaration_count(arena)
            })
    }

    /// Add a body-local declaration to this frame.
    ///
    /// WHAT: appends the declaration to this frame's vec, updates the name index, and
    ///       records insertion for instrumentation.
    /// WHY: callers must ensure they are mutating the correct frame; this method does
    ///      not walk the parent chain because additions always belong to the current scope.
    pub(crate) fn add_var(&mut self, declaration: Declaration, binding_location: SourceLocation) {
        if let Some(name) = declaration.id.name() {
            let index = self.local_declarations.len() as u32;
            self.local_declarations_by_name
                .entry(name)
                .or_default()
                .push(index);
        }
        self.local_declarations.push(LocalDeclaration {
            declaration: Rc::new(declaration),
            binding_location,
        });
    }

    /// Add a body-local declaration authored with `#`.
    ///
    /// WHAT: records the explicit compile-time constant flag in this frame, then inserts
    ///       the declaration like an ordinary local.
    pub(crate) fn add_compile_time_var(
        &mut self,
        declaration: Declaration,
        binding_location: SourceLocation,
    ) {
        let declarations = self
            .explicit_compile_time_constant_declarations
            .get_or_insert_with(Default::default);
        Rc::make_mut(declarations).insert(declaration.id.clone());

        self.add_var(declaration, binding_location);
    }

    /// Replace the declarations in this frame.
    ///
    /// WHAT: rebuilds the name index and the declaration vec in one step. Used when a
    ///       function or start body frame is initialised with parameter declarations.
    pub(crate) fn set_local_declarations(&mut self, declarations: Vec<Declaration>) {
        self.local_declarations_by_name = build_local_declarations_index(&declarations);
        self.local_declarations = declarations
            .into_iter()
            .map(|declaration| {
                let binding_location = declaration.value.location.clone();
                LocalDeclaration {
                    declaration: Rc::new(declaration),
                    binding_location,
                }
            })
            .collect();
    }
}

/// Build an index mapping local declaration names to their positions in `declarations`.
///
/// WHAT: enables O(1) lookup of all locals with a given name within a single frame.
///       The last registered index represents the currently visible binding in that frame.
/// WHY: avoids reverse-scanning the full declaration vec on every name resolution.
fn build_local_declarations_index(declarations: &[Declaration]) -> FxHashMap<StringId, Vec<u32>> {
    let mut index: FxHashMap<StringId, Vec<u32>> = FxHashMap::default();
    for (i, declaration) in declarations.iter().enumerate() {
        if let Some(name) = declaration.id.name() {
            index.entry(name).or_default().push(i as u32);
        }
    }
    index
}
