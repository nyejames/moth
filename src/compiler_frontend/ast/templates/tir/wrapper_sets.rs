//! TIR wrapper-set and wrapper-context overlay helpers.
//!
//! WHAT: owns the conservative equivalence predicate used to deduplicate
//! `$children(..)` wrapper sets in the `TemplateIrStore` side table, and the
//! wrapper-context overlay construction that records inherited wrapper sets
//! and `$fresh` suppression for child-template occurrences on a template's
//! authoritative structural root. Wrapper-context overlays are the sole owner
//! of direct-child inherited wrappers, including children inside branch and
//! loop bodies.
//!
//! WHY: wrapper sets and wrapper-context overlays both describe how
//! `$children(..)` wrappers apply to child-template boundaries. Keeping the
//! equivalence predicate, wrapper-reference normalization, and overlay
//! construction in one module makes the wrapper application boundary explicit
//! and easy to audit without leaking store internals into the template
//! construction orchestrator.

use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirWrapperApplicationMode, TirWrapperContext, TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirReference;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, validate_context};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Returns true when two wrapper template ref vectors are equivalent.
///
/// WHAT: compares the two vectors element-wise using `TemplateWrapperReference`
/// equality. Because wrapper sets store effective refs (root + phase +
/// context), two sets are equivalent exactly when all three fields match
/// for every wrapper in the same order.
///
/// Empty wrapper vectors are always equivalent, so control-flow children that
/// receive no inherited wrappers share one side-table entry.
///
/// WHY: wrapper-set reuse must never merge wrappers that differ in dynamic
/// behavior, formatter output, slot routing, or conditional semantics. Identity
/// comparison on all three fields is the safe, precise reuse authority; no
/// intermediate content representation is inspected.
pub(crate) fn wrapper_sets_are_equivalent(
    left: &[TemplateWrapperReference],
    right: &[TemplateWrapperReference],
) -> bool {
    left.len() == right.len() && left.iter().zip(right.iter()).all(|(l, r)| l == r)
}

/// Converts a wrapper `Template` into an effective module-local wrapper reference.
///
/// WHAT: extracts the template's TIR reference (root, phase, value context)
///       and validates its overlay and template identity in the active store.
/// WHY: wrapper references carry only module-local root, phase, and overlay
///      identity because every TIR value in this AST build uses one store.
///
/// Returns `Err` when the wrapper has no valid TIR identity or its overlay or
/// template entry is missing. These are internal invariant failures.
pub(crate) fn wrapper_reference_for_template(
    template: &Template,
    current_store: &TemplateIrStore,
) -> Result<TemplateWrapperReference, CompilerError> {
    let reference = &template.tir_reference;
    validate_context(
        current_store,
        reference.context,
        "wrapper-reference normalization",
    )?;
    current_store.get_template(reference.root).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "wrapper-reference normalization: template {} was missing from the current store.",
            reference.root
        ))
    })?;

    Ok(TemplateWrapperReference::new(
        reference.root,
        reference.phase,
        reference.context,
    ))
}

// -------------------------
//  Wrapper-context overlay construction
// -------------------------

/// Attaches a wrapper-context overlay to a template's TIR reference.
///
/// WHAT: walks the owning template's structural root, finds every
/// `ChildTemplate` occurrence, and records `$fresh` suppression or inherited
/// wrapper-set context. The resulting overlay is composed with the reference's
/// current view context so downstream `TirView` resolution applies wrappers at
/// child-template boundaries without mutating shared structural roots.
///
/// WHY: wrapper-context overlay construction is TIR-owned because wrapper-set
/// composition, overlay storage, and wrapper-reference validation already
/// live here. Moving the traversal out of the template construction orchestrator
/// keeps the orchestrator focused on ordering and lets the wrapper owner enforce
/// required authority and propagate failures.
///
/// Semantics preserved from the prior local implementation:
/// - `$fresh` suppresses only the immediate parent's wrappers.
/// - Ordinary children use `Always`; structurally control-flow children use
///   `IfChildEmits`.
/// - Wrapper order is unchanged.
/// - No contexts means no wrapper overlay is attached.
///
/// Missing owning template, root node, traversed node, child store, child
/// template, or overlay composition failures return `CompilerError` instead of
/// silently skipping.
pub(crate) fn attach_wrapper_context_overlay(
    tir_reference: &mut TemplateTirReference,
    inherited_wrapper_refs: &[TemplateWrapperReference],
    store_handle: &Rc<RefCell<TemplateIrStore>>,
) -> Result<(), CompilerError> {
    // Validate ownership and read the root before mutating anything. Required
    // authority is proven before durable wrapper or overlay state is allocated.
    let root = {
        let store = store_handle.borrow();
        validate_context(
            &store,
            tir_reference.context,
            "wrapper-context current reference",
        )?;
        store
            .get_template(tir_reference.root)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "wrapper-context overlay: owning template {} not found in store.",
                    tir_reference.root
                ))
            })?
            .root
    };

    increment_ast_counter(AstCounter::TemplateTirChildWrapperCalls);

    let mut pending_contexts = Vec::new();
    {
        let store = store_handle.borrow();
        collect_wrapper_contexts(&store, root, inherited_wrapper_refs, &mut pending_contexts)?;
    }

    if pending_contexts.is_empty() {
        return Ok(());
    }

    let applied_wrapper_count = pending_contexts
        .iter()
        .filter(|context| !context.skip_parent_child_wrappers)
        .count();
    add_ast_counter(
        AstCounter::TemplateTirChildWrapperHits,
        applied_wrapper_count,
    );

    // Every inherited context uses the same ordered wrapper references. Allocate
    // or reuse that set once, after the full structural walk has validated all
    // nodes and child references.
    let inherited_wrapper_set = if pending_contexts
        .iter()
        .any(|context| !context.skip_parent_child_wrappers)
    {
        let mut store = store_handle.borrow_mut();
        let wrapper_set_id = store.push_or_reuse_wrapper_set(inherited_wrapper_refs.to_vec());
        Some(wrapper_set_id)
    } else {
        None
    };

    let contexts = pending_contexts
        .into_iter()
        .map(|context| {
            let inherited_wrapper_set = if context.skip_parent_child_wrappers {
                None
            } else {
                inherited_wrapper_set
            };

            (
                context.occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set,
                    skip_parent_child_wrappers: context.skip_parent_child_wrappers,
                    application_mode: context.application_mode,
                },
            )
        })
        .collect();

    let mut store = store_handle.borrow_mut();
    let wrapper_overlay_id =
        store.allocate_wrapper_context_overlay(TirWrapperContextOverlay { contexts })?;
    let wrapper_only_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(wrapper_overlay_id),
    };
    let merged_context = tir_reference.context.merge(wrapper_only_context);

    tir_reference.context = merged_context;
    if !tir_reference.phase.is_at_least(TemplateTirPhase::Composed) {
        tir_reference.phase = TemplateTirPhase::Composed;
    }
    Ok(())
}

/// Validated occurrence context collected before wrapper-set allocation.
struct PendingWrapperContext {
    occurrence_id: ChildTemplateOccurrenceId,
    skip_parent_child_wrappers: bool,
    application_mode: TirWrapperApplicationMode,
}

/// Recursively collects wrapper contexts for child-template occurrences in the
/// structural tree rooted at `node_id`.
///
/// WHAT: traverses `Sequence`, `BranchChain`, and `Loop` structural nodes to
///       find `ChildTemplate` occurrences. For each occurrence, resolves the
///       child template's metadata directly from the module store and records
///       `$fresh` suppression or inherited wrapper-set context.
///
/// WHY: wrapper context belongs to the occurrence in the owning structural
///      tree. This traversal does not recurse into a child's own root — it only
///      walks the structural containers that surround child-template
///      occurrences. The store is borrowed immutably for the entire traversal
///      and no mutation occurs until `collect_wrapper_contexts` returns, so the
///      node kind can be matched directly without cloning child vectors into a
///      transient fact enum.
fn collect_wrapper_contexts(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    inherited_wrapper_refs: &[TemplateWrapperReference],
    contexts: &mut Vec<PendingWrapperContext>,
) -> Result<(), CompilerError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "wrapper-context overlay: traversed TIR node {} not found in store.",
            node_id
        ))
    })?;

    match &node.kind {
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let metadata = resolve_child_wrapper_metadata(store, reference)?;
            if metadata.skip_parent_child_wrappers {
                contexts.push(PendingWrapperContext {
                    occurrence_id: *occurrence_id,
                    skip_parent_child_wrappers: true,
                    application_mode: TirWrapperApplicationMode::Always,
                });
            } else if !inherited_wrapper_refs.is_empty() {
                let application_mode = if metadata.has_control_flow {
                    TirWrapperApplicationMode::IfChildEmits
                } else {
                    TirWrapperApplicationMode::Always
                };
                contexts.push(PendingWrapperContext {
                    occurrence_id: *occurrence_id,
                    skip_parent_child_wrappers: false,
                    application_mode,
                });
            }
        }
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                collect_wrapper_contexts(store, *child_id, inherited_wrapper_refs, contexts)?;
            }
        }
        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                collect_wrapper_contexts(store, branch.body, inherited_wrapper_refs, contexts)?;
            }
            if let Some(fallback_id) = fallback {
                collect_wrapper_contexts(store, *fallback_id, inherited_wrapper_refs, contexts)?;
            }
        }
        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_wrapper_contexts(store, *body, inherited_wrapper_refs, contexts)?;
            if let Some(wrapper_id) = aggregate_wrapper {
                collect_wrapper_contexts(store, *wrapper_id, inherited_wrapper_refs, contexts)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Wrapper-relevant metadata for a child-template occurrence, resolved from the
/// child's TIR entry.
struct ChildWrapperMetadata {
    has_control_flow: bool,
    skip_parent_child_wrappers: bool,
}

/// Resolves child-template metadata for wrapper-context decisions.
///
/// WHAT: reads `has_control_flow` and `skip_parent_child_wrappers` from the
///       child's TIR entry. References use the already-held store directly
///       to avoid `RefCell` re-entry.
///
/// WHY: wrapper context belongs to the occurrence in the owning structural
///      tree, not to the child's own root. Only the child's metadata is needed
///      to decide `$fresh` suppression and application mode.
fn resolve_child_wrapper_metadata(
    current_store: &TemplateIrStore,
    reference: &TemplateTirChildReference,
) -> Result<ChildWrapperMetadata, CompilerError> {
    validate_context(
        current_store,
        reference.context,
        "wrapper-context child reference",
    )?;
    let child = current_store.get_template(reference.root).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "wrapper-context overlay: child template {} not found in current store.",
            reference.root
        ))
    })?;
    Ok(ChildWrapperMetadata {
        has_control_flow: child.summary.has_control_flow(),
        skip_parent_child_wrappers: child.style.skip_parent_child_wrappers,
    })
}
