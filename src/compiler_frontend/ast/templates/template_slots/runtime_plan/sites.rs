//! Runtime wrapper slot-site planning.
//!
//! WHAT: assigns stable IDs to concrete wrapper `$slot` occurrences and builds
//! one TIR render root per site from plan-qualified contribution markers.
//!
//! WHY: runtime applications must evaluate each source once while repeated slot
//! placeholders can still carry different `$children(..)` and `$fresh`
//! metadata. Wrapper injection keeps the complete wrapper reference and plants
//! the same TIR marker whether the splice sits directly in a site or inside
//! control flow.

use super::types::{RuntimeSlotContributionSourceDraft, RuntimeSlotSiteId};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::{
    DerivedTemplateMetadata, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateIrStore, TemplateSlotPlanId, TemplateSlotSitePlan, TemplateWrapperSetId, TirCopyState,
    TirSlotPlaceholderRef, collect_tir_slot_layout, collect_tir_slot_layout_from_root,
    copy_tir_subtree_with_active_slot_plan, push_runtime_slot_contribution_source,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};

pub(super) fn build_runtime_wrapper_site_plan(
    wrapper_tir_root: TemplateIrNodeId,
    sources: &[RuntimeSlotContributionSourceDraft],
    slot_plan_id: TemplateSlotPlanId,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<Vec<TemplateSlotSitePlan>, TemplateError> {
    RuntimeWrapperSitePlanBuilder {
        sources,
        slot_plan_id,
        store,
        copy_state,
    }
    .build_slot_sites(wrapper_tir_root)
}

struct RuntimeSlotSiteDraft {
    site: RuntimeSlotSiteId,
    placeholder: TirSlotPlaceholderRef,
}

struct RuntimeWrapperSitePlanBuilder<'a> {
    sources: &'a [RuntimeSlotContributionSourceDraft],
    slot_plan_id: TemplateSlotPlanId,
    store: &'a mut TemplateIrStore,
    copy_state: &'a mut TirCopyState,
}

impl RuntimeWrapperSitePlanBuilder<'_> {
    fn build_slot_sites(
        mut self,
        wrapper_tir_root: TemplateIrNodeId,
    ) -> Result<Vec<TemplateSlotSitePlan>, TemplateError> {
        let mut drafts = Vec::new();
        self.collect_site_drafts(wrapper_tir_root, &mut drafts)?;

        let mut slot_sites = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let render_root =
                self.build_site_render_root(&draft.placeholder, &draft.placeholder.location)?;
            slot_sites.push(TemplateSlotSitePlan {
                site: draft.site,
                key: draft.placeholder.key.clone(),
                render_root,
                location: draft.placeholder.location.clone(),
            });
        }

        Ok(slot_sites)
    }

    fn collect_site_drafts(
        &mut self,
        wrapper_tir_root: TemplateIrNodeId,
        drafts: &mut Vec<RuntimeSlotSiteDraft>,
    ) -> Result<(), TemplateError> {
        // Walk the TIR tree in document order to discover every unresolved slot
        // placeholder, including those nested inside child templates, branch
        // chains, and loops. TIR is the sole authority for slot-placeholder
        // discovery.
        // Site IDs are assigned in traversal order, matching the cursor-based
        // assignment the final materialization pass will use.
        let layout = collect_tir_slot_layout_from_root(self.store, wrapper_tir_root)?;

        for placeholder in layout.placeholders {
            let site = RuntimeSlotSiteId(drafts.len());
            drafts.push(RuntimeSlotSiteDraft { site, placeholder });
        }

        Ok(())
    }

    fn build_site_render_root(
        &mut self,
        placeholder: &TirSlotPlaceholderRef,
        location: &SourceLocation,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let mut fill_roots = Vec::new();

        for source in self
            .sources
            .iter()
            .filter(|source| source.source.target == placeholder.key)
        {
            let source_root = push_runtime_slot_contribution_source(
                self.store,
                self.slot_plan_id,
                source.source.source,
                location.clone(),
            );
            let wrapped_root = self.apply_site_wrappers(placeholder, source, source_root)?;
            fill_roots.push(wrapped_root);
        }

        Ok(collapse_render_roots(
            fill_roots,
            location.clone(),
            self.store,
        ))
    }

    fn apply_site_wrappers(
        &mut self,
        placeholder: &TirSlotPlaceholderRef,
        source: &RuntimeSlotContributionSourceDraft,
        source_root: TemplateIrNodeId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let shape = source.shape;

        // Source plans are distinct from site plans so repeated placeholders can
        // apply their own `$children(..)` metadata without re-evaluating the source.
        let (mut fill_root, wrapped_as_child) = match placeholder.child_wrapper_set {
            Some(child_wrapper_set)
                if !shape.skips_parent_child_wrappers()
                    && shape.is_child_template_contribution() =>
            {
                (
                    self.wrap_site_fill_with_tir_child_wrappers(source_root, child_wrapper_set)?,
                    true,
                )
            }

            _ => (source_root, shape.is_child_template_contribution()),
        };

        if !placeholder.skip_parent_child_wrappers
            && wrapped_as_child
            && let Some(applied_child_wrapper_set) = placeholder.applied_child_wrapper_set
        {
            fill_root =
                self.wrap_site_fill_with_tir_child_wrappers(fill_root, applied_child_wrapper_set)?;
        }

        Ok(fill_root)
    }

    /// Applies an ordered TIR wrapper set around a fill root.
    ///
    /// WHAT: resolves complete wrapper references from the wrapper-set side
    ///       table and iterates them forward (innermost-first).
    /// WHY: `TemplateWrapperSet::wrappers` is stored innermost-to-outermost, so
    ///      forward consumption yields the `outermost(innermost(fill))` nesting
    ///      the store contract requires.
    fn wrap_site_fill_with_tir_child_wrappers(
        &mut self,
        mut fill_root: TemplateIrNodeId,
        wrapper_set_id: TemplateWrapperSetId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let Some(wrapper_set) = self.store.get_wrapper_set(wrapper_set_id) else {
            return Err(CompilerError::compiler_error(
                "Runtime slot site planning found a slot wrapper-set ID that was not present in the TIR store.",
            )
            .into());
        };

        let wrapper_refs = wrapper_set.wrappers.clone();
        for wrapper_ref in wrapper_refs {
            add_ast_counter(AstCounter::TemplateWrapperApplications, 1);
            fill_root = self.apply_wrapper_reference(wrapper_ref, fill_root)?;
        }

        Ok(fill_root)
    }

    /// Applies one exact wrapper reference around a fill root.
    ///
    /// WHAT: injects the fill into the wrapper tree, then versions the original
    ///       wrapper template around the injected root and references that
    ///       derived template through the named structural or composed reference
    ///       transition for this application.
    /// WHY: reducing a wrapper to a bare template or node ID would drop exact
    ///      reference authority and place the rewritten tree under the
    ///      surrounding application's unrelated view.
    fn apply_wrapper_reference(
        &mut self,
        wrapper_ref: TemplateWrapperReference,
        fill_root: TemplateIrNodeId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let (wrapper_root, wrapper_location) = {
            let wrapper_template = self.store.get_template(wrapper_ref.root).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Runtime slot site planning found a slot wrapper template ref {} that was not present in the TIR store.",
                    wrapper_ref
                ))
            })?;
            (wrapper_template.root, wrapper_template.location.clone())
        };
        let layout = collect_tir_slot_layout(self.store, wrapper_ref.root)?;

        if !layout.schema.has_any_slots() {
            let wrapper_reference = wrapper_ref.into_structural_child_reference();
            let unchanged_wrapper =
                self.push_wrapper_child(wrapper_reference, wrapper_location.clone());
            return Ok(collapse_render_roots(
                vec![unchanged_wrapper, fill_root],
                wrapper_location,
                self.store,
            ));
        }

        let Some(target_key) = layout.schema.loose_fill_target_key() else {
            return Err(CompilerError::compiler_error(
                "Runtime slot site planning could not apply a TIR slot wrapper to a loose contribution.",
            )
            .into());
        };

        let injected = inject_runtime_slot_fill(
            wrapper_root,
            fill_root,
            &target_key,
            self.store,
            self.copy_state,
        )?;

        self.push_composed_wrapper_child(wrapper_ref, injected.root, wrapper_location)
    }

    fn push_composed_wrapper_child(
        &mut self,
        wrapper_ref: TemplateWrapperReference,
        new_root: TemplateIrNodeId,
        location: SourceLocation,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let derived_id = self.derive_wrapper_template(wrapper_ref.root, new_root)?;
        let reference = wrapper_ref.into_composed_child_reference(derived_id);
        Ok(self.push_wrapper_child(reference, location))
    }

    fn derive_wrapper_template(
        &mut self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
    ) -> Result<TemplateIrId, TemplateError> {
        Ok(self.store.push_structurally_derived_template(
            source,
            new_root,
            DerivedTemplateMetadata::preserve_source(),
        )?)
    }

    fn push_wrapper_child(
        &mut self,
        reference: TemplateTirChildReference,
        location: SourceLocation,
    ) -> TemplateIrNodeId {
        let occurrence_id = self.store.next_child_template_occurrence_id();
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference,
                occurrence_id,
            },
            location,
        ))
    }
}

struct RuntimeSlotInjection {
    root: TemplateIrNodeId,
    changed: bool,
}

/// Injects `fill_root` at every schema-reachable occurrence of `target_key`.
///
/// Fill subtrees are copied independently and receive fresh expression and
/// occurrence IDs. Versioning the wrapper structure itself preserves IDs for
/// semantic sites that still represent the same occurrence.
fn inject_runtime_slot_fill(
    wrapper_root: TemplateIrNodeId,
    fill_root: TemplateIrNodeId,
    target_key: &SlotKey,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<RuntimeSlotInjection, TemplateError> {
    let node = store.get_node(wrapper_root).cloned().ok_or_else(|| {
        CompilerError::compiler_error(
            "Child-wrapper TIR path could not read the wrapper root node.",
        )
    })?;
    let location = node.location.clone();

    match node.kind {
        TemplateIrNodeKind::Slot { placeholder } if placeholder.key == *target_key => {
            Ok(RuntimeSlotInjection {
                root: copy_tir_subtree_with_active_slot_plan(fill_root, None, store, copy_state)?,
                changed: true,
            })
        }

        TemplateIrNodeKind::Slot { .. } => Ok(RuntimeSlotInjection {
            root: empty_render_root(store, &location),
            changed: true,
        }),

        TemplateIrNodeKind::Sequence { children } => {
            let mut injected_children = Vec::with_capacity(children.len());
            let mut changed = false;
            for child_id in children {
                let injected_child =
                    inject_runtime_slot_fill(child_id, fill_root, target_key, store, copy_state)?;
                changed |= injected_child.changed;
                injected_children.push(injected_child.root);
            }

            if !changed {
                return Ok(RuntimeSlotInjection {
                    root: wrapper_root,
                    changed: false,
                });
            }

            Ok(RuntimeSlotInjection {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Sequence {
                        children: injected_children,
                    },
                    location,
                )),
                changed: true,
            })
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let child_root = store
                .get_template(reference.root)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Runtime slot site planning found child template {} missing from the TIR store.",
                        reference.root
                    ))
                })?
                .root;

            let injected =
                inject_runtime_slot_fill(child_root, fill_root, target_key, store, copy_state)?;
            if !injected.changed {
                return Ok(RuntimeSlotInjection {
                    root: wrapper_root,
                    changed: false,
                });
            }

            let derived_id = store.push_structurally_derived_template(
                reference.root,
                injected.root,
                DerivedTemplateMetadata::preserve_source(),
            )?;
            Ok(RuntimeSlotInjection {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::ChildTemplate {
                        reference: reference.with_root(derived_id),
                        occurrence_id,
                    },
                    location,
                )),
                changed: true,
            })
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let mut branch_results = Vec::with_capacity(branches.len());
            let mut changed = false;
            for branch in branches {
                let injected_body = inject_runtime_slot_fill(
                    branch.body,
                    fill_root,
                    target_key,
                    store,
                    copy_state,
                )?;
                changed |= injected_body.changed;
                branch_results.push((
                    branch.selector,
                    injected_body.root,
                    branch.location,
                    branch.selector_site_id,
                ));
            }

            let injected_fallback = match fallback {
                Some(fallback_id) => {
                    let injected_fallback = inject_runtime_slot_fill(
                        fallback_id,
                        fill_root,
                        target_key,
                        store,
                        copy_state,
                    )?;
                    changed |= injected_fallback.changed;
                    Some(injected_fallback.root)
                }
                None => None,
            };

            if !changed {
                return Ok(RuntimeSlotInjection {
                    root: wrapper_root,
                    changed: false,
                });
            }

            let injected_branches = branch_results
                .into_iter()
                .map(|(selector, body, location, selector_site_id)| {
                    crate::compiler_frontend::ast::templates::tir::TemplateIrBranch::new(
                        selector,
                        body,
                        location,
                        selector_site_id,
                    )
                })
                .collect();
            copy_state.record_control_flow();
            Ok(RuntimeSlotInjection {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::BranchChain {
                        branches: injected_branches,
                        fallback: injected_fallback,
                    },
                    location,
                )),
                changed: true,
            })
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
            ..
        } => {
            let injected_body =
                inject_runtime_slot_fill(body, fill_root, target_key, store, copy_state)?;
            let mut changed = injected_body.changed;
            let injected_aggregate = match aggregate_wrapper {
                Some(wrapper_id) => {
                    let injected_aggregate = inject_runtime_slot_fill(
                        wrapper_id, fill_root, target_key, store, copy_state,
                    )?;
                    changed |= injected_aggregate.changed;
                    Some(injected_aggregate.root)
                }
                None => None,
            };
            if !changed {
                return Ok(RuntimeSlotInjection {
                    root: wrapper_root,
                    changed: false,
                });
            }

            copy_state.record_control_flow();
            Ok(RuntimeSlotInjection {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Loop {
                        header,
                        header_sites,
                        body: injected_body.root,
                        aggregate_wrapper: injected_aggregate,
                    },
                    location,
                )),
                changed: true,
            })
        }

        _ => Ok(RuntimeSlotInjection {
            root: wrapper_root,
            changed: false,
        }),
    }
}

fn empty_render_root(store: &mut TemplateIrStore, location: &SourceLocation) -> TemplateIrNodeId {
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        location.clone(),
    ))
}

fn collapse_render_roots(
    roots: Vec<TemplateIrNodeId>,
    location: SourceLocation,
    store: &mut TemplateIrStore,
) -> TemplateIrNodeId {
    match roots.len() {
        0 => empty_render_root(store, &location),
        1 => roots[0],
        _ => store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: roots },
            location,
        )),
    }
}

#[cfg(test)]
mod sites_tests;
