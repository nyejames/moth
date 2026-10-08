//! Runtime template append context shared by HIR template lowering modules.
//!
//! WHAT: stores the active append target plus the runtime slot source/site state
//! needed while appending AST-owned runtime-template handoff nodes.
//! WHY: render appending, slot application lowering, control-flow lowering, and aggregate
//! wrapping all share this state, but `render_append.rs` should remain focused on appending
//! owned runtime-template nodes.

use crate::compiler_frontend::ast::templates::OwnedRuntimeSlotSite;
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId;
use crate::compiler_frontend::hir::ids::LocalId;

/// Accumulator and emitted state owned by one runtime slot contribution source.
#[derive(Clone, Copy)]
pub(super) struct RuntimeSlotSourceLocals {
    pub(super) accumulator: LocalId,
    pub(super) emitted_output: LocalId,
}

/// Source locals available while HIR lowers a runtime slot application wrapper.
///
/// WHAT: pairs each AST source ID with its accumulator and structural emitted flag.
/// WHY: repeated slot sites replay source bytes and propagate structural emission
/// separately without re-lowering authored contribution expressions.
pub(super) struct RuntimeSlotSourceAccumulatorContext {
    locals_by_source: Vec<RuntimeSlotSourceLocals>,
}

impl RuntimeSlotSourceAccumulatorContext {
    pub(super) fn new() -> Self {
        Self {
            locals_by_source: Vec::new(),
        }
    }

    pub(super) fn insert(
        &mut self,
        id: RuntimeSlotContributionSourceId,
        locals: RuntimeSlotSourceLocals,
    ) {
        if self.locals_by_source.len() <= id.0 {
            self.locals_by_source.resize(id.0 + 1, locals);
        }

        self.locals_by_source[id.0] = locals;
    }

    pub(super) fn for_source(
        &self,
        id: RuntimeSlotContributionSourceId,
    ) -> Option<RuntimeSlotSourceLocals> {
        self.locals_by_source.get(id.0).copied()
    }
}

/// Policy for unresolved `OwnedRuntimeTemplateNode::Slot` placeholders.
///
/// WHAT: distinguishes the two runtime contexts in which HIR can see a slot
/// placeholder. Standalone runtime templates (e.g. helpers that are not being
/// used as slot wrappers) legitimately contain missing structural slots, which
/// render as empty strings. Active runtime slot-application wrappers, by
/// contrast, should have had every placeholder resolved to a concrete site by
/// AST slot routing; an unresolved placeholder there is an internal compiler
/// invariant breach.
/// WHY: keeps the HIR append path explicit about which no-output behavior is
/// valid and which is a transformation bug.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RuntimeSlotPlaceholderPolicy {
    MissingSlotRendersEmpty,
    RejectUnresolvedSlot,
}

/// Append target plus optional runtime-slot state for owned-node lowering.
#[derive(Clone, Copy)]
pub(super) struct RuntimeTemplateAppendContext<'a> {
    pub(super) target_accumulator: LocalId,
    pub(super) emitted_output: Option<LocalId>,
    pub(super) source_accumulators: Option<&'a RuntimeSlotSourceAccumulatorContext>,
    pub(super) slot_sites: Option<&'a [OwnedRuntimeSlotSite]>,
    pub(super) slot_placeholder_policy: RuntimeSlotPlaceholderPolicy,
}

impl<'a> RuntimeTemplateAppendContext<'a> {
    pub(super) fn new(target_accumulator: LocalId) -> Self {
        Self {
            target_accumulator,
            emitted_output: None,
            source_accumulators: None,
            slot_sites: None,
            slot_placeholder_policy: RuntimeSlotPlaceholderPolicy::MissingSlotRendersEmpty,
        }
    }

    pub(super) fn with_emitted_output(mut self, emitted_output: LocalId) -> Self {
        self.emitted_output = Some(emitted_output);
        self
    }

    pub(super) fn with_target_accumulator(mut self, target_accumulator: LocalId) -> Self {
        self.target_accumulator = target_accumulator;
        self
    }

    pub(super) fn with_runtime_slot_sites(
        mut self,
        source_accumulators: &'a RuntimeSlotSourceAccumulatorContext,
        slot_sites: &'a [OwnedRuntimeSlotSite],
    ) -> Self {
        self.source_accumulators = Some(source_accumulators);
        self.slot_sites = Some(slot_sites);
        self
    }

    /// Enables strict rejection of unresolved slot placeholders.
    ///
    /// WHAT: produces a context where an `OwnedRuntimeTemplateNode::Slot` is
    /// treated as a compiler transformation error instead of rendering empty.
    /// WHY: active runtime slot-application wrappers must resolve every slot
    /// placeholder to a concrete site; callers constructing that wrapper context
    /// opt into the stricter policy.
    pub(super) fn rejecting_unresolved_slots(mut self) -> Self {
        self.slot_placeholder_policy = RuntimeSlotPlaceholderPolicy::RejectUnresolvedSlot;
        self
    }

    pub(super) fn rejects_unresolved_slots(&self) -> bool {
        self.slot_placeholder_policy == RuntimeSlotPlaceholderPolicy::RejectUnresolvedSlot
    }

    pub(super) fn target_accumulator(&self) -> LocalId {
        self.target_accumulator
    }

    pub(super) fn emitted_output(&self) -> Option<LocalId> {
        self.emitted_output
    }
}
