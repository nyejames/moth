//! Owned runtime-template handoff materialization from TIR.
//!
//! WHAT: builds an owned, recursive runtime-template tree from an exact `TirView`
//! for the AST-to-HIR boundary.
//!
//! WHY: HIR should consume finalized runtime template metadata without holding
//! raw `TemplateIrId`, `TemplateIrNodeId`, or `TemplateSlotPlanId` values. This
//! module keeps those IDs internal to AST/TIR traversal and returns the neutral
//! owned handoff shapes defined in `runtime_handoff.rs` that HIR lowering
//! consumes directly.

use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource, OwnedRuntimeSlotSite,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateBranch, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::preparation::{
    TemplatePreparation, TemplatePreparationOutcome,
};
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, owned_folded_string_from_const_string,
};

use crate::compiler_frontend::ast::templates::tir::collect_tir_slot_schema;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
    TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirSlotResolutionKind, TirWrapperApplicationMode, TirWrapperContext,
};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::slot_plan::TemplateSlotPlan;
use crate::compiler_frontend::ast::templates::tir::view::TirView;
use crate::compiler_frontend::ast::templates::tir::view::TirViewIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use std::collections::HashSet;

/// Materializes a prepared runtime slot application from its exact view.
pub(crate) fn owned_runtime_slot_handoff_for_prepared_view(
    prepared: &TemplatePreparation,
    view: TirView<'_>,
    string_table: &StringTable,
    module_resources: Option<&ModuleResourceTable>,
) -> Result<Option<OwnedRuntimeSlotApplicationHandoff>, CompilerError> {
    validate_prepared_runtime(prepared, &view)?;
    let template_id = template_id_for_view(&view)?;
    let mut materializer = RuntimeHandoffMaterializer::new(string_table, module_resources);
    materializer.owned_runtime_slot_handoff_for_template(&view, template_id)
}

/// Materializes an ordinary runtime template from its prepared exact view.
pub(crate) fn owned_runtime_template_handoff_for_prepared_view(
    prepared: &TemplatePreparation,
    view: TirView<'_>,
    string_table: &StringTable,
    module_resources: Option<&ModuleResourceTable>,
) -> Result<OwnedRuntimeTemplateHandoff, CompilerError> {
    validate_prepared_runtime(prepared, &view)?;
    let template_id = template_id_for_view(&view)?;
    let mut materializer = RuntimeHandoffMaterializer::new(string_table, module_resources);
    materializer.owned_runtime_template_handoff_for_template(&view, template_id)
}

fn template_id_for_view(view: &TirView<'_>) -> Result<TemplateIrId, CompilerError> {
    let template_id = view.root_ref();
    view.root_template().map(|_| template_id).map_err(|_| {
        CompilerError::compiler_error(
            "TIR HIR handoff view materialization referenced a missing template.",
        )
    })
}

fn validate_prepared_runtime(
    prepared: &TemplatePreparation,
    view: &TirView<'_>,
) -> Result<(), CompilerError> {
    if prepared.identity != view.identity() {
        return Err(CompilerError::compiler_error(
            "TIR runtime handoff preparation root/phase/context identity does not match the supplied view.",
        ));
    }

    if !matches!(prepared.outcome, TemplatePreparationOutcome::Runtime(_)) {
        return Err(CompilerError::compiler_error(
            "TIR runtime handoff received a non-runtime preparation outcome.",
        ));
    }

    Ok(())
}

struct RuntimeHandoffMaterializer<'a> {
    active_views: HashSet<TirViewIdentity>,
    string_table: &'a StringTable,
    module_resources: Option<&'a ModuleResourceTable>,
}

impl<'a> RuntimeHandoffMaterializer<'a> {
    fn new(
        string_table: &'a StringTable,
        module_resources: Option<&'a ModuleResourceTable>,
    ) -> Self {
        Self {
            active_views: HashSet::new(),
            string_table,
            module_resources,
        }
    }

    fn owned_runtime_slot_handoff_for_template(
        &mut self,
        view: &TirView<'_>,
        id: TemplateIrId,
    ) -> Result<Option<OwnedRuntimeSlotApplicationHandoff>, CompilerError> {
        let template = self.get_template(view, id)?;
        let root = template.root;
        let Some(slot_plan_id) = template.runtime_slot_plan else {
            return Ok(None);
        };

        self.materialize_runtime_slot_application_by_parts(view, root, slot_plan_id, None)
            .map(Some)
    }

    fn owned_runtime_template_handoff_for_template(
        &mut self,
        view: &TirView<'_>,
        id: TemplateIrId,
    ) -> Result<OwnedRuntimeTemplateHandoff, CompilerError> {
        self.materialize_template(view, id, None, None)
    }

    fn materialize_template(
        &mut self,
        view: &TirView<'_>,
        id: TemplateIrId,
        active_slot_plan: Option<TemplateSlotPlanId>,
        injection: Option<(&SlotKey, &OwnedRuntimeTemplateNode)>,
    ) -> Result<OwnedRuntimeTemplateHandoff, CompilerError> {
        let identity = view.identity();
        if !self.active_views.insert(identity) {
            return Err(CompilerError::compiler_error(format!(
                "TIR handoff: exact view {identity:?} re-entered while still active."
            )));
        }

        let result = self.materialize_template_body(view, id, active_slot_plan, injection);
        self.active_views.remove(&identity);
        result
    }

    fn materialize_template_body(
        &mut self,
        view: &TirView<'_>,
        id: TemplateIrId,
        active_slot_plan: Option<TemplateSlotPlanId>,
        injection: Option<(&SlotKey, &OwnedRuntimeTemplateNode)>,
    ) -> Result<OwnedRuntimeTemplateHandoff, CompilerError> {
        let template = self.get_template(view, id)?;
        let location = template.location.clone();
        let runtime_slot_plan = template.runtime_slot_plan;
        let root = template.root;

        let body = if let Some(slot_plan_id) = runtime_slot_plan {
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(Box::new(
                self.materialize_runtime_slot_application_by_parts(
                    view,
                    root,
                    slot_plan_id,
                    injection,
                )?,
            ))
        } else {
            OwnedRuntimeTemplateBody::Render(self.materialize_node_with_injection(
                view,
                root,
                active_slot_plan,
                injection,
            )?)
        };

        Ok(OwnedRuntimeTemplateHandoff { body, location })
    }

    fn materialize_runtime_slot_application_by_parts(
        &mut self,
        view: &TirView<'_>,
        wrapper_root: TemplateIrNodeId,
        slot_plan_id: TemplateSlotPlanId,
        injection: Option<(&SlotKey, &OwnedRuntimeTemplateNode)>,
    ) -> Result<OwnedRuntimeSlotApplicationHandoff, CompilerError> {
        let slot_plan = self.get_slot_plan(view, slot_plan_id)?;
        let wrapper = self.materialize_node_with_injection(
            view,
            wrapper_root,
            Some(slot_plan_id),
            injection,
        )?;
        let contribution_sources =
            self.materialize_contribution_sources(view, slot_plan, slot_plan_id)?;
        let slot_sites = self.materialize_slot_sites(view, slot_plan, slot_plan_id)?;

        Ok(OwnedRuntimeSlotApplicationHandoff {
            wrapper,
            contribution_sources,
            slot_sites,
            location: slot_plan.location.clone(),
        })
    }

    fn materialize_contribution_sources(
        &mut self,
        view: &TirView<'_>,
        slot_plan: &TemplateSlotPlan,
        slot_plan_id: TemplateSlotPlanId,
    ) -> Result<Vec<OwnedRuntimeSlotContributionSource>, CompilerError> {
        let mut sources = Vec::with_capacity(slot_plan.contribution_sources.len());

        for source in &slot_plan.contribution_sources {
            sources.push(OwnedRuntimeSlotContributionSource {
                source: source.source,
                render_root: self.materialize_node(view, source.render_root, Some(slot_plan_id))?,
                renders_wrapper_unconditionally: source.renders_wrapper_unconditionally,
                location: source.location.clone(),
            });
        }

        Ok(sources)
    }

    fn materialize_slot_sites(
        &mut self,
        view: &TirView<'_>,
        slot_plan: &TemplateSlotPlan,
        slot_plan_id: TemplateSlotPlanId,
    ) -> Result<Vec<OwnedRuntimeSlotSite>, CompilerError> {
        let mut sites = Vec::with_capacity(slot_plan.slot_sites.len());

        for site in &slot_plan.slot_sites {
            sites.push(OwnedRuntimeSlotSite {
                site: site.site,
                render_root: self.materialize_node(view, site.render_root, Some(slot_plan_id))?,
                location: site.location.clone(),
            });
        }

        Ok(sites)
    }

    fn materialize_node(
        &mut self,
        view: &TirView<'_>,
        id: TemplateIrNodeId,
        active_slot_plan: Option<TemplateSlotPlanId>,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        self.materialize_node_with_injection(view, id, active_slot_plan, None)
    }

    /// Materializes one TIR node through the canonical handoff walker, with an
    /// optional inherited child injected at matching slot placeholders.
    ///
    /// WHAT: keeps ordinary node materialization and wrapper fill injection on
    ///       the same structural traversal, including branches, loops and
    ///       module-local child-template roots.
    /// WHY: wrapper target selection is layout-owned, so the handoff walker must
    ///      be able to replace every structural shape that schema discovery can
    ///      reach without creating a second, partial materializer.
    fn materialize_node_with_injection(
        &mut self,
        view: &TirView<'_>,
        id: TemplateIrNodeId,
        active_slot_plan: Option<TemplateSlotPlanId>,
        injection: Option<(&SlotKey, &OwnedRuntimeTemplateNode)>,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        let node = self.effective_node(view, id)?;

        let owned_node = match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                let mut owned_children = Vec::with_capacity(children.len());
                for child in children {
                    owned_children.push(self.materialize_node_with_injection(
                        view,
                        *child,
                        active_slot_plan,
                        injection,
                    )?);
                }

                Ok(OwnedRuntimeTemplateNode::Sequence {
                    children: owned_children,
                })
            }

            TemplateIrNodeKind::Text {
                text,
                byte_len: _,
                origin: _,
            } => Ok(OwnedRuntimeTemplateNode::Text {
                text: OwnedFoldedString::Text(self.string_table.resolve(*text).to_owned()),
                reactive_subscription: view.store().node_reactive_subscription(id)?.cloned(),
                location: node.location.clone(),
            }),

            TemplateIrNodeKind::DynamicExpression {
                expression,
                origin: _,
                reactive_subscription,
                site_id,
            } => {
                let effective_expression =
                    self.effective_expression(view, *site_id, expression.as_ref())?;

                if let ExpressionKind::StructuralString { pieces } = &effective_expression.kind {
                    let resources = self.module_resources.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "TIR runtime handoff reached a structural string without the issuing module resource table.",
                        )
                    })?;
                    let value = ConstStringValue::Pieces(pieces.clone());
                    let text = owned_folded_string_from_const_string(
                        &value,
                        resources,
                        self.string_table,
                    )?;
                    Ok(OwnedRuntimeTemplateNode::Text {
                        text,
                        reactive_subscription: reactive_subscription.clone(),
                        location: node.location.clone(),
                    })
                } else {
                    Ok(OwnedRuntimeTemplateNode::DynamicExpression {
                        expression: Box::new(effective_expression),
                        reactive_subscription: reactive_subscription.clone(),
                    })
                }
            }

            TemplateIrNodeKind::ChildTemplate {
                reference,
                occurrence_id,
            } => {
                let wrapper_context =
                    self.effective_wrapper_context_for_occurrence(view, *occurrence_id)?;
                let child_handoff = self.materialize_child_template_node(
                    view,
                    reference,
                    active_slot_plan,
                    injection,
                )?;

                if let Some(context) = wrapper_context {
                    self.apply_wrapper_context_overlay_to_child_handoff(
                        view,
                        &context,
                        child_handoff,
                        &node.location,
                    )
                } else {
                    Ok(child_handoff)
                }
            }

            TemplateIrNodeKind::BranchChain { branches, fallback } => {
                let mut owned_branches = Vec::with_capacity(branches.len());
                for branch in branches {
                    let body = self.materialize_node_with_injection(
                        view,
                        branch.body,
                        active_slot_plan,
                        injection,
                    )?;

                    owned_branches.push(OwnedRuntimeTemplateBranch {
                        selector: self.effective_branch_selector(
                            view,
                            &branch.selector,
                            branch.selector_site_id,
                        )?,
                        body,
                        location: branch.location.clone(),
                    });
                }

                let fallback = if let Some(fallback_id) = fallback {
                    Some(Box::new(self.materialize_node_with_injection(
                        view,
                        *fallback_id,
                        active_slot_plan,
                        injection,
                    )?))
                } else {
                    None
                };

                Ok(OwnedRuntimeTemplateNode::BranchChain {
                    branches: owned_branches,
                    fallback,
                    location: node.location.clone(),
                })
            }

            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper,
                ..
            } => {
                let body_node =
                    self.materialize_node_with_injection(view, *body, active_slot_plan, injection)?;

                let aggregate_wrapper = if let Some(wrapper_id) = aggregate_wrapper {
                    Some(Box::new(self.materialize_node_with_injection(
                        view,
                        *wrapper_id,
                        active_slot_plan,
                        injection,
                    )?))
                } else {
                    None
                };

                Ok(OwnedRuntimeTemplateNode::Loop {
                    header: self.effective_loop_header(view, header, *header_sites)?,
                    body: Box::new(body_node),
                    aggregate_wrapper,
                    location: node.location.clone(),
                })
            }

            TemplateIrNodeKind::AggregateOutput => Ok(OwnedRuntimeTemplateNode::AggregateOutput),

            TemplateIrNodeKind::LoopControl { kind } => Ok(OwnedRuntimeTemplateNode::LoopControl {
                kind: *kind,
                location: node.location.clone(),
            }),

            TemplateIrNodeKind::RuntimeSlotSite { plan, site } => {
                if Some(*plan) != active_slot_plan {
                    return Err(CompilerError::compiler_error(
                        "TIR HIR handoff materialization found a runtime slot site outside its owning slot application.",
                    ));
                }

                let slot_plan = self.get_slot_plan(view, *plan)?;
                let Some(stored) = slot_plan.slot_sites.get(site.0) else {
                    return Err(CompilerError::compiler_error(
                        "TIR HIR handoff materialization found an out-of-range runtime slot site.",
                    ));
                };
                if stored.site != *site {
                    return Err(CompilerError::compiler_error(
                        "TIR HIR handoff materialization found a slot site whose stored identity does not match its index.",
                    ));
                }

                Ok(OwnedRuntimeTemplateNode::RuntimeSlotSite { site: *site })
            }

            TemplateIrNodeKind::RuntimeSlotContributionSource { plan, source } => {
                if Some(*plan) != active_slot_plan {
                    return Err(CompilerError::compiler_error(
                        "TIR HIR handoff materialization found a runtime slot contribution marker outside its owning slot application.",
                    ));
                }
                let slot_plan = self.get_slot_plan(view, *plan)?;
                let Some(stored) = slot_plan.contribution_sources.get(source.0) else {
                    return Err(CompilerError::compiler_error(
                        "TIR HIR handoff materialization found an out-of-range runtime slot contribution source.",
                    ));
                };
                if stored.source != *source {
                    return Err(CompilerError::compiler_error(
                        "TIR HIR handoff materialization found a contribution source whose stored identity does not match its index.",
                    ));
                }

                Ok(OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { source: *source })
            }

            TemplateIrNodeKind::Slot { placeholder } => {
                if let Some((fill_target_key, child_handoff)) = injection
                    && placeholder.key == *fill_target_key
                {
                    return Ok(child_handoff.clone());
                }

                if let Some(resolution) =
                    self.effective_slot_resolution_for_occurrence(view, placeholder.occurrence_id)?
                    && let TirSlotResolutionKind::Resolved { sources } = &resolution.kind
                {
                    return self.materialize_resolved_slot_sources(
                        view,
                        sources,
                        &node.location,
                        active_slot_plan,
                    );
                }

                Ok(OwnedRuntimeTemplateNode::Slot {
                    location: node.location.clone(),
                })
            }

            TemplateIrNodeKind::InsertContribution { template } => {
                let helper_view = view.structural_helper(*template)?;
                let helper_handoff =
                    self.materialize_template(&helper_view, *template, active_slot_plan, None)?;
                Ok(OwnedRuntimeTemplateNode::ChildTemplate {
                    template: Box::new(helper_handoff),
                })
            }
        }?;

        increment_ast_counter(AstCounter::RuntimeSlotHandoffOwnedNodesMaterialized);
        Ok(owned_node)
    }

    fn get_template<'store>(
        &self,
        view: &TirView<'store>,
        id: TemplateIrId,
    ) -> Result<&'store TemplateIr, CompilerError> {
        view.store().get_template(id).ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR HIR handoff materialization referenced a missing template.",
            )
        })
    }

    fn get_node<'store>(
        &self,
        view: &TirView<'store>,
        id: TemplateIrNodeId,
    ) -> Result<&'store TemplateIrNode, CompilerError> {
        view.store().get_node(id).ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR HIR handoff materialization referenced a missing node.",
            )
        })
    }

    fn effective_node<'store>(
        &self,
        view: &TirView<'store>,
        id: TemplateIrNodeId,
    ) -> Result<&'store TemplateIrNode, CompilerError> {
        self.get_node(view, id)
    }

    /// Resolves the effective expression for a site from the current exact view.
    ///
    /// WHAT: reads the complete root overlay through `TirView` and falls back
    ///       to the structural expression when the site has no override.
    fn effective_expression(
        &self,
        view: &TirView<'_>,
        site_id: ExpressionSiteId,
        fallback: &Expression,
    ) -> Result<Expression, CompilerError> {
        Ok(self
            .effective_expression_for_site(view, site_id)?
            .unwrap_or_else(|| fallback.clone()))
    }

    fn effective_expression_for_site(
        &self,
        view: &TirView<'_>,
        site_id: ExpressionSiteId,
    ) -> Result<Option<Expression>, CompilerError> {
        Ok(view.effective_expression_for_site(site_id)?.cloned())
    }

    /// Resolves the effective wrapper context for a child-template occurrence,
    /// preferring the override carried by the current exact view.
    ///
    /// WHAT: reads the active value-carried view context and resolves its
    ///       wrapper-context overlay ID through the module store, returning a
    ///       clone of the wrapper context for `occurrence_id` if one exists.
    ///       Returns `None` when there is no view context or no wrapper-context
    ///       overlay. A missing active overlay is an internal error.
    /// WHY: this mirrors `effective_expression_for_site` for the wrapper-context
    ///      dimension so child-template handoff can apply inherited `$children(..)`
    ///      wrappers and `$fresh` suppression without mutating the structural root.
    fn effective_wrapper_context_for_occurrence(
        &self,
        view: &TirView<'_>,
        occurrence_id: ChildTemplateOccurrenceId,
    ) -> Result<Option<TirWrapperContext>, CompilerError> {
        Ok(view.effective_wrapper_context(occurrence_id)?.cloned())
    }

    /// Resolves the effective slot resolution for a slot occurrence,
    /// preferring the resolution carried by the current exact view.
    ///
    /// WHAT: reads the active value-carried view context and resolves its
    ///       slot-resolution overlay ID through the module store, returning a
    ///       clone of the `TirSlotResolution` for `occurrence_id` if one exists.
    ///       Returns `None` when there is no view context or no slot-resolution
    ///       overlay. A missing active overlay is an internal error.
    /// WHY: this mirrors `effective_expression_for_site` and
    ///      `effective_wrapper_context_for_occurrence` for the slot-resolution
    ///      dimension so handoff materialization can render resolved slot fills
    ///      from the final effective view instead of treating every structural
    ///      `Slot` node as a no-output placeholder.
    fn effective_slot_resolution_for_occurrence(
        &self,
        view: &TirView<'_>,
        occurrence_id: SlotOccurrenceId,
    ) -> Result<Option<super::overlays::TirSlotResolution>, CompilerError> {
        Ok(view.effective_slot_resolution(occurrence_id)?.cloned())
    }

    fn effective_branch_selector(
        &self,
        view: &TirView<'_>,
        selector: &TemplateBranchSelector,
        site_id: ExpressionSiteId,
    ) -> Result<TemplateBranchSelector, CompilerError> {
        let Some(expression) = self.effective_expression_for_site(view, site_id)? else {
            return Ok(selector.clone());
        };

        Ok(match selector {
            TemplateBranchSelector::Bool(_) => TemplateBranchSelector::Bool(expression),
            TemplateBranchSelector::OptionPresentCapture { pattern, .. } => {
                TemplateBranchSelector::OptionPresentCapture {
                    scrutinee: expression,
                    pattern: pattern.clone(),
                }
            }
        })
    }

    fn effective_loop_header(
        &self,
        view: &TirView<'_>,
        header: &TemplateLoopHeader,
        header_sites: TemplateLoopHeaderExpressionSites,
    ) -> Result<TemplateLoopHeader, CompilerError> {
        Ok(match (header, header_sites) {
            (
                TemplateLoopHeader::Conditional { condition },
                TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
            ) => TemplateLoopHeader::Conditional {
                condition: Box::new(
                    self.effective_expression_for_site(view, site_id)?
                        .unwrap_or_else(|| condition.as_ref().clone()),
                ),
            },

            (
                TemplateLoopHeader::Range { bindings, range },
                TemplateLoopHeaderExpressionSites::Range { start, end, step },
            ) => {
                let mut range = range.as_ref().clone();
                if let Some(expression) = self.effective_expression_for_site(view, start)? {
                    range.start = expression;
                }
                if let Some(expression) = self.effective_expression_for_site(view, end)? {
                    range.end = expression;
                }
                match (&range.step, step) {
                    (None, None) => {}
                    (Some(_), Some(step_site_id)) => {
                        if let Some(expression) =
                            self.effective_expression_for_site(view, step_site_id)?
                        {
                            range.step = Some(expression);
                        }
                    }
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "TIR HIR handoff materialization found a loop range header/site step shape mismatch.",
                        ));
                    }
                }

                TemplateLoopHeader::Range {
                    bindings: bindings.clone(),
                    range: Box::new(range),
                }
            }

            (
                TemplateLoopHeader::Collection { bindings, iterable },
                TemplateLoopHeaderExpressionSites::Collection { iterable: site_id },
            ) => TemplateLoopHeader::Collection {
                bindings: bindings.clone(),
                iterable: Box::new(
                    self.effective_expression_for_site(view, site_id)?
                        .unwrap_or_else(|| iterable.as_ref().clone()),
                ),
            },

            _ => {
                return Err(CompilerError::compiler_error(
                    "TIR HIR handoff materialization found a loop header shape mismatch.",
                ));
            }
        })
    }

    /// Materializes a `ChildTemplate` node into an owned runtime handoff node.
    fn materialize_child_template_node(
        &mut self,
        view: &TirView<'_>,
        reference: &TemplateTirChildReference,
        active_slot_plan: Option<TemplateSlotPlanId>,
        injection: Option<(&SlotKey, &OwnedRuntimeTemplateNode)>,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        let child_view = view.structural_child(*reference)?;
        self.materialize_child_template_node_with_view(
            reference.root,
            child_view,
            active_slot_plan,
            injection,
        )
    }

    fn materialize_child_template_node_with_view(
        &mut self,
        template_id: TemplateIrId,
        child_view: TirView<'_>,
        active_slot_plan: Option<TemplateSlotPlanId>,
        injection: Option<(&SlotKey, &OwnedRuntimeTemplateNode)>,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        let handoff =
            self.materialize_template(&child_view, template_id, active_slot_plan, injection);

        Ok(OwnedRuntimeTemplateNode::ChildTemplate {
            template: Box::new(handoff?),
        })
    }

    /// Materializes a list of resolved slot sources into owned runtime handoff
    /// nodes.
    ///
    /// WHAT: a single source becomes one owned node; multiple sources become a
    ///       `Sequence` of child-template handoffs in deterministic source order.
    /// WHY: repeated slots and multi-source contributions are represented by a
    ///      list of sources in the overlay; the handoff must preserve that order
    ///      without inventing new node kinds.
    fn materialize_resolved_slot_sources(
        &mut self,
        view: &TirView<'_>,
        sources: &[TemplateIrId],
        location: &SourceLocation,
        active_slot_plan: Option<TemplateSlotPlanId>,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        if sources.is_empty() {
            return Ok(OwnedRuntimeTemplateNode::Slot {
                location: location.to_owned(),
            });
        }

        if sources.len() == 1 {
            return self.materialize_resolved_slot_source(view, &sources[0], active_slot_plan);
        }

        let mut children = Vec::with_capacity(sources.len());
        for source in sources {
            children.push(self.materialize_resolved_slot_source(view, source, active_slot_plan)?);
        }

        Ok(OwnedRuntimeTemplateNode::Sequence { children })
    }

    /// Materializes one resolved slot source into an owned runtime handoff node.
    ///
    /// WHAT: enters the resolved source exactly once, then materializes that exact
    ///       view as the existing owned child-template handoff shape.
    /// WHY: slot-resolution overlays carry bare `TemplateIrId` sources. Their
    ///      phase and context are supplied by the active parent view, so a
    ///      synthetic child reference would apply the structural transition twice.
    fn materialize_resolved_slot_source(
        &mut self,
        view: &TirView<'_>,
        source: &TemplateIrId,
        active_slot_plan: Option<TemplateSlotPlanId>,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        let source_view = view.resolved_slot_source(*source)?;
        self.materialize_child_template_node_with_view(*source, source_view, active_slot_plan, None)
    }

    /// Applies a wrapper-context overlay entry to an already-materialized child
    /// handoff node.
    ///
    /// WHAT: validates the wrapper-context shape, honors `$fresh` suppression, and
    ///       resolves the inherited wrapper set into module-local wrapper refs before
    ///       wrapping the child handoff. `IfChildEmits` becomes a neutral
    ///       `ConditionalWrapper` node so HIR can use its existing emitted-output
    ///       guard without seeing TIR overlay state.
    /// WHY: this is the runtime-handoff analogue of
    ///      `apply_wrapper_context_overlay_to_child_emission` in `fold/wrappers.rs`.
    fn apply_wrapper_context_overlay_to_child_handoff(
        &mut self,
        view: &TirView<'_>,
        context: &TirWrapperContext,
        child_handoff: OwnedRuntimeTemplateNode,
        child_location: &SourceLocation,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        if context.skip_parent_child_wrappers {
            return Ok(child_handoff);
        }

        let Some(wrapper_set_ref) = context.inherited_wrapper_set else {
            return Ok(child_handoff);
        };

        let wrapper_set = view
            .store()
            .get_wrapper_set(wrapper_set_ref)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "TIR HIR handoff: inherited wrapper set referenced by overlay is missing.",
                )
            })?;

        match context.application_mode {
            TirWrapperApplicationMode::Always => self.apply_wrapper_templates_around_child_handoff(
                view,
                &wrapper_set.wrappers,
                child_handoff,
            ),

            TirWrapperApplicationMode::IfChildEmits => self
                .apply_conditional_wrapper_templates_around_child_handoff(
                    view,
                    &wrapper_set.wrappers,
                    child_handoff,
                    child_location,
                ),
        }
    }

    /// Wraps a child handoff node in each inherited wrapper template.
    ///
    /// WHAT: iterates wrappers forward (innermost-first), composing each
    ///       wrapper around the current wrapped child. The result is an owned
    ///       runtime node that represents wrapper-text-around-child.
    /// WHY: `TemplateWrapperSet::wrappers` is stored innermost-to-outermost, so
    ///      forward iteration yields the `outermost(innermost(child))` nesting
    ///      that mirrors `fold_conditional_child_wrappers_around_emission` and
    ///      the structural `wrap_tir_node_in_wrappers` order.
    fn apply_wrapper_templates_around_child_handoff(
        &mut self,
        view: &TirView<'_>,
        wrapper_references: &[TemplateWrapperReference],
        child_handoff: OwnedRuntimeTemplateNode,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        let mut current = child_handoff;
        for wrapper_reference in wrapper_references.iter() {
            current = self.apply_single_wrapper_template_around_child_handoff(
                view,
                *wrapper_reference,
                current,
            )?;
        }
        Ok(current)
    }

    /// Builds one output-conditioned wrapper node for an inherited wrapper set.
    ///
    /// WHAT: materializes all wrappers forward (innermost-first) around an
    ///       `AggregateOutput` marker, then pairs that wrapper tree with the
    ///       original child in `ConditionalWrapper`.
    /// WHY: `IfChildEmits` is a runtime structural condition. HIR already knows
    ///      how to append aggregate wrappers only when a source accumulator
    ///      emitted output, so the handoff should expose that neutral shape
    ///      instead of TIR overlay state. Forward consumption preserves the
    ///      innermost-to-outermost store order so the outermost wrapper is the
    ///      final layer around the marker.
    fn apply_conditional_wrapper_templates_around_child_handoff(
        &mut self,
        view: &TirView<'_>,
        wrapper_references: &[TemplateWrapperReference],
        child_handoff: OwnedRuntimeTemplateNode,
        child_location: &SourceLocation,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        if wrapper_references.is_empty() {
            return Ok(child_handoff);
        }

        let mut wrapper = OwnedRuntimeTemplateNode::AggregateOutput;
        for wrapper_reference in wrapper_references.iter() {
            wrapper = self.apply_single_wrapper_template_around_child_handoff(
                view,
                *wrapper_reference,
                wrapper,
            )?;
        }

        Ok(OwnedRuntimeTemplateNode::ConditionalWrapper {
            child: Box::new(child_handoff),
            wrapper: Box::new(wrapper),
            location: child_location.to_owned(),
        })
    }

    /// Materializes a wrapper around a child handoff using the given materializer.
    ///
    /// WHAT: consolidates wrapper materialization into one path that uses a
    ///       materializer reference for the module-local wrapper template.
    /// WHY: eliminates the duplicated `match fill_target_key` block while
    ///      preserving wrapper materialization semantics.
    fn materialize_wrapper_with_child(
        materializer: &mut RuntimeHandoffMaterializer,
        view: &TirView<'_>,
        wrapper_root: TemplateIrNodeId,
        fill_target_key: Option<SlotKey>,
        child_handoff: OwnedRuntimeTemplateNode,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        match fill_target_key {
            Some(fill_target_key) => materializer.materialize_node_with_injection(
                view,
                wrapper_root,
                None,
                Some((&fill_target_key, &child_handoff)),
            ),
            None => {
                let wrapper_content = materializer.materialize_node(view, wrapper_root, None)?;
                Ok(OwnedRuntimeTemplateNode::Sequence {
                    children: vec![wrapper_content, child_handoff],
                })
            }
        }
    }

    /// Wraps one wrapper template around a child handoff node.
    ///
    /// WHAT: materializes the wrapper template's content, then either injects the
    ///       child at the wrapper's loose-fill slot or appends it after wrapper
    ///       content when the schema has no loose-fill target (slot-less or
    ///       named-only wrappers).
    ///       Runtime slot-plan wrappers are rejected because inherited `$children(..)`
    ///       wrappers must be ordinary render templates.
    /// WHY: this produces the same owned shape as TIR wrapper composition
    ///      without exposing TIR identity across the HIR boundary.
    fn apply_single_wrapper_template_around_child_handoff(
        &mut self,
        view: &TirView<'_>,
        wrapper_reference: TemplateWrapperReference,
        child_handoff: OwnedRuntimeTemplateNode,
    ) -> Result<OwnedRuntimeTemplateNode, CompilerError> {
        let wrapper_store = view.store();

        let (wrapper_root, has_runtime_slot_plan) = wrapper_store
            .get_template(wrapper_reference.root)
            .map(|template| (template.root, template.runtime_slot_plan.is_some()))
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR HIR handoff: wrapper template {} not found in the store.",
                    wrapper_reference.root
                ))
            })?;

        if has_runtime_slot_plan {
            return Err(CompilerError::compiler_error(
                "TIR HIR handoff: inherited wrapper template declares a runtime slot plan.",
            ));
        }

        let schema = collect_tir_slot_schema(wrapper_store, wrapper_reference.root)?;
        let fill_target_key = schema.loose_fill_target_key();

        let wrapper_view = view.wrapper(wrapper_reference)?;
        Self::materialize_wrapper_with_child(
            self,
            &wrapper_view,
            wrapper_root,
            fill_target_key,
            child_handoff,
        )
    }

    fn get_slot_plan<'store>(
        &self,
        view: &TirView<'store>,
        id: TemplateSlotPlanId,
    ) -> Result<&'store TemplateSlotPlan, CompilerError> {
        view.store().get_slot_plan(id).ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR HIR handoff materialization referenced a missing slot plan.",
            )
        })
    }
}
