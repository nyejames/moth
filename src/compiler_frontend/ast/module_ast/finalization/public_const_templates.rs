//! Public const-template projection at the AST finalization boundary.
//!
//! WHAT: folds exported-capable wrapper and slot-insert constants through the canonical TIR
//! reducer, then converts unresolved slots and wrapper sets into an owned interface value.
//! WHY: consumers need const-template composition semantics, but TIR stores, IDs, overlays and
//! donor-local strings must be dropped before a completed AST or public interface leaves AST.

use super::finalizer::AstFinalizer;
use super::normalize_ast::TemplateNormalizationError;
use super::template_helpers::make_fold_context;
use crate::compiler_frontend::ast::const_values::store::{
    ConstTemplateValue, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Template, TemplateConstValueKind, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_folding::TemplateEmission;
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, SlotOccurrenceId, TemplateHelperKind, TemplatePreparation,
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TirView,
    TirViewIdentity, fold_prepared_const_template_pattern, prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::folded_value::{
    PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicTemplateSlotKey,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use rustc_hash::{FxHashMap, FxHashSet};

/// Stable const-template projections consumed by generated materialisation.
///
/// The path index lets an in-flight generated compilation rebuild the exact declaring-module
/// constant in its fresh TIR store.
pub(super) struct ProjectedConstTemplates {
    pub(super) by_path: FxHashMap<InternedPath, PublicConstTemplate>,
    pub(super) module_values: FxHashMap<InternedPath, ProjectedConstTemplateValue>,
}

/// One exact module-constant template projection produced by the finalization owner.
///
/// WHAT: pairs preparation classification with the one public template projection and folded
///       scalar/provenance result needed by the compact constant store.
/// WHY: store construction must consume this result instead of preparing and folding the same
///       TIR view a second time.
#[derive(Clone)]
pub(super) struct ProjectedConstTemplateValue {
    pub(super) kind: TemplateConstValueKind,
    pub(super) public: Option<PublicConstTemplate>,
    pub(super) folded: Option<StringId>,
    pub(super) provenance: SyntheticInterfaceProvenance,
}

impl AstFinalizer<'_, '_> {
    pub(super) fn project_const_templates(
        &self,
        string_table: &mut StringTable,
    ) -> Result<ProjectedConstTemplates, TemplateNormalizationError> {
        let mut by_path = FxHashMap::default();
        let mut module_values = FxHashMap::default();
        let store = self.context.template_ir_store.borrow();

        for declaration_id in self.environment.lookups.resolved_module_constants.iter() {
            let declaration = self
                .environment
                .lookups
                .declaration_table
                .get_by_id(declaration_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Resolved module-constant ID had no declaration-table row.",
                    )
                })?;
            if !matches!(declaration.value.kind, ExpressionKind::Template(_)) {
                continue;
            }
            let projected =
                self.project_module_template_expression(declaration, &store, string_table)?;
            if module_values
                .insert(declaration.id.clone(), projected)
                .is_some()
            {
                return Err(CompilerError::compiler_error(
                    "Module constant template projection produced duplicate declaration paths.",
                )
                .into());
            }
        }

        // Parameter and nominal-field defaults are declaration-owned compile-time values too.
        // Keep their path index alongside module constants so frozen generic metadata can
        // reconstruct a normalized const template without reopening the donor TIR store.
        for signature in self
            .environment
            .lookups
            .resolved_function_signatures_by_path
            .values()
        {
            for parameter in &signature.signature.parameters {
                let Some(projected) =
                    self.project_template_expression(parameter, &store, string_table)?
                else {
                    continue;
                };
                Self::insert_projected_template(&mut by_path, parameter, projected)?;
            }
        }
        for fields in self
            .environment
            .lookups
            .resolved_struct_fields_by_path
            .values()
        {
            for field in fields {
                let Some(projected) =
                    self.project_template_expression(field, &store, string_table)?
                else {
                    continue;
                };
                Self::insert_projected_template(&mut by_path, field, projected)?;
            }
        }

        Ok(ProjectedConstTemplates {
            by_path,
            module_values,
        })
    }

    fn project_template_expression(
        &self,
        declaration: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        store: &crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
        string_table: &mut StringTable,
    ) -> Result<Option<PublicConstTemplate>, TemplateNormalizationError> {
        let ExpressionKind::Template(template) = &declaration.value.kind else {
            return Ok(None);
        };
        Ok(self
            .project_template_value(template, store, string_table)?
            .public)
    }

    fn project_module_template_expression(
        &self,
        declaration: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        store: &crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
        string_table: &mut StringTable,
    ) -> Result<ProjectedConstTemplateValue, TemplateNormalizationError> {
        let ExpressionKind::Template(template) = &declaration.value.kind else {
            return Err(CompilerError::compiler_error(
                "Module template projection received a non-template declaration.",
            )
            .into());
        };
        self.project_template_value(template, store, string_table)
    }

    pub(super) fn project_template_value(
        &self,
        template: &Template,
        store: &crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
        string_table: &mut StringTable,
    ) -> Result<ProjectedConstTemplateValue, TemplateNormalizationError> {
        project_const_template_value(
            template,
            store,
            string_table,
            self.context.template_const_loop_iteration_limit,
        )
    }

    fn insert_projected_template(
        by_path: &mut FxHashMap<InternedPath, PublicConstTemplate>,
        declaration: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        projected: PublicConstTemplate,
    ) -> Result<(), TemplateNormalizationError> {
        if let Some(existing) = by_path.get(&declaration.id)
            && existing != &projected
        {
            return Err(CompilerError::compiler_error(
                "Public const-template projection produced conflicting values for one declaration path.",
            )
            .into());
        }
        by_path.insert(declaration.id.clone(), projected);
        Ok(())
    }
}

/// Classify one finalization projection into the value the module constant store holds.
///
/// WHAT: maps the four const-template classifications onto the store's template payload, and
/// rejects the two that cannot be a compile-time constant value.
/// WHY: the store must not re-derive template identity, and the classification rule is the same
/// for every module constant, so it belongs beside the projection that produced it rather than
/// inline in the finalization sequence.
pub(super) fn const_template_value_from_projection(
    projected: ProjectedConstTemplateValue,
    template: &Template,
) -> Result<ConstTemplateValue, ConstValueStoreError> {
    // A template only projects when preparation classified it foldable or as a slot-insert
    // helper. Anything else is runtime-dependent: still a valid executable template value, but
    // never a compile-time constant one. `final_value_kind` alone cannot answer this, because a
    // renderable or wrapper template can carry a runtime outcome.
    let Some(public) = projected.public else {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::NonFoldableConstTemplate,
            template.location.clone(),
        )
        .into());
    };

    match projected.kind {
        TemplateConstValueKind::RenderableString => {
            let string = projected.folded.ok_or_else(|| {
                CompilerError::compiler_error(
                    "Renderable module constant template did not produce a folded string.",
                )
            })?;
            Ok(ConstTemplateValue::Folded {
                string,
                provenance: projected.provenance,
            })
        }

        // A wrapper stays visible to HIR through its folded string while its structured pieces
        // remain available to the public interface.
        TemplateConstValueKind::WrapperTemplate => {
            let folded = projected.folded.ok_or_else(|| {
                CompilerError::compiler_error(
                    "Wrapper module constant did not complete exact TIR finalization.",
                )
            })?;
            Ok(ConstTemplateValue::Public {
                template: public,
                kind: ConstValueKind::TemplateWrapper,
                hir_visible: true,
                folded: Some(folded),
                provenance: projected.provenance,
            })
        }

        // INVARIANT: `$insert(..)` helpers are composition inputs, not values. They keep their
        // public projection and stay out of the HIR constant handoff.
        TemplateConstValueKind::SlotInsertHelper => Ok(ConstTemplateValue::Public {
            template: public,
            kind: ConstValueKind::SlotInsertTemplate,
            hir_visible: false,
            folded: None,
            provenance: projected.provenance,
        }),

        // Preparation never publishes these, so the guard above already rejected them.
        TemplateConstValueKind::LoopControlSignal | TemplateConstValueKind::NonConst => {
            Err(CompilerError::compiler_error(
                "A non-const template classification reached module-constant store construction with a public projection.",
            )
            .into())
        }
    }
}

/// Prepare and fold one const template into its single finalization projection.
///
/// WHAT: classifies the template through its exact effective TIR view, then folds that view once
/// into both the owned public projection and the scalar emission.
/// WHY: the module-constant store, the public interface and generated materialisation all consume
/// the same result, so the view must be prepared and folded exactly once per template.
pub(super) fn project_const_template_value(
    template: &Template,
    store: &crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
    string_table: &mut StringTable,
    template_const_loop_iteration_limit: usize,
) -> Result<ProjectedConstTemplateValue, TemplateNormalizationError> {
    let reference = template.tir_reference;
    let view = TirView::with_minimum_phase(
        store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)?;
    let kind = prepared.facts.final_value_kind;
    let publish = matches!(prepared.outcome, TemplatePreparationOutcome::Foldable)
        || matches!(
            prepared.outcome,
            TemplatePreparationOutcome::Helper(TemplateHelperKind::SlotInsert)
        );
    if !publish {
        return Ok(ProjectedConstTemplateValue {
            kind,
            public: None,
            folded: None,
            provenance: SyntheticInterfaceProvenance::empty(),
        });
    }

    let (public, emission, provenance) = {
        let mut fold_context = make_fold_context(string_table, template_const_loop_iteration_limit);
        let mut visiting = FxHashSet::default();
        let projected =
            project_const_template_view(view, prepared, &mut fold_context, &mut visiting)?;
        (projected.template, projected.emission, projected.provenance)
    };
    let folded = match kind {
        TemplateConstValueKind::RenderableString | TemplateConstValueKind::WrapperTemplate => {
            Some(match emission {
                TemplateEmission::NoOutput => string_table.intern(""),
                TemplateEmission::Output(value) => value,
                TemplateEmission::Break(_) | TemplateEmission::Continue(_) => {
                    return Err(CompilerError::compiler_error(
                        "Folded module template emitted an unconsumed loop-control signal.",
                    )
                    .into());
                }
            })
        }
        TemplateConstValueKind::SlotInsertHelper
        | TemplateConstValueKind::LoopControlSignal
        | TemplateConstValueKind::NonConst => None,
    };

    Ok(ProjectedConstTemplateValue {
        kind,
        public: Some(public),
        folded,
        provenance,
    })
}

fn project_const_template_view(
    view: TirView<'_>,
    prepared: TemplatePreparation,
    fold_context: &mut crate::compiler_frontend::ast::templates::template_folding::TirFoldContext<
        '_,
    >,
    visiting: &mut FxHashSet<TirViewIdentity>,
) -> Result<ProjectedTemplateView, TemplateNormalizationError> {
    if !visiting.insert(view.identity()) {
        return Err(CompilerError::compiler_error(
            "Public const-template projection encountered a recursive wrapper view after preparation.",
        )
        .into());
    }

    let template = view.root_template()?;
    let kind = project_template_kind(&template.kind, fold_context.string_table)?;
    let pattern = fold_prepared_const_template_pattern(prepared, view.clone(), fold_context)?;
    let emission = pattern.emission;
    let provenance = pattern.provenance.clone();
    let mut pieces = Vec::with_capacity(pattern.pieces.len());

    for piece in pattern.pieces {
        match piece {
            FoldedConstTemplatePiece::Text(text) => {
                pieces.push(PublicConstTemplatePiece::Text(text));
            }
            FoldedConstTemplatePiece::Slot(occurrence) => {
                pieces.push(PublicConstTemplatePiece::Slot(project_slot(
                    &view,
                    occurrence,
                    fold_context,
                    visiting,
                )?));
            }
        }
    }

    let conditional_child_wrappers = project_wrapper_set(
        &view,
        template.conditional_child_wrapper_set,
        fold_context,
        visiting,
    )?;
    visiting.remove(&view.identity());

    Ok(ProjectedTemplateView {
        template: PublicConstTemplate {
            kind,
            pieces,
            conditional_child_wrappers,
        },
        emission,
        provenance,
    })
}

struct ProjectedTemplateView {
    template: PublicConstTemplate,
    emission: TemplateEmission,
    provenance: SyntheticInterfaceProvenance,
}

fn project_slot(
    view: &TirView<'_>,
    occurrence: SlotOccurrenceId,
    fold_context: &mut crate::compiler_frontend::ast::templates::template_folding::TirFoldContext<
        '_,
    >,
    visiting: &mut FxHashSet<TirViewIdentity>,
) -> Result<PublicConstTemplateSlot, TemplateNormalizationError> {
    let placeholder = view.slot_placeholder(occurrence).ok_or_else(|| {
        CompilerError::compiler_error(
            "Public const-template fold returned a slot occurrence absent from its TIR store.",
        )
    })?;

    Ok(PublicConstTemplateSlot {
        key: project_slot_key(&placeholder.key, fold_context.string_table),
        applied_child_wrappers: project_wrapper_set(
            view,
            placeholder.applied_child_wrapper_set,
            fold_context,
            visiting,
        )?,
        child_wrappers: project_wrapper_set(
            view,
            placeholder.child_wrapper_set,
            fold_context,
            visiting,
        )?,
        skip_parent_child_wrappers: placeholder.skip_parent_child_wrappers,
    })
}

fn project_wrapper_set(
    parent_view: &TirView<'_>,
    wrapper_set_id: Option<crate::compiler_frontend::ast::templates::tir::TemplateWrapperSetId>,
    fold_context: &mut crate::compiler_frontend::ast::templates::template_folding::TirFoldContext<
        '_,
    >,
    visiting: &mut FxHashSet<TirViewIdentity>,
) -> Result<Vec<PublicConstTemplate>, TemplateNormalizationError> {
    let Some(wrapper_set_id) = wrapper_set_id else {
        return Ok(Vec::new());
    };
    let wrapper_set = parent_view
        .store()
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            CompilerError::compiler_error("Public const-template wrapper set is missing.")
        })?;
    let references = wrapper_set.wrappers.clone();
    let mut wrappers = Vec::with_capacity(references.len());

    for reference in references {
        let wrapper_view = parent_view.wrapper(reference)?;
        let prepared = prepare_tir_view(&wrapper_view, TemplatePreparationMode::ConstRequired)?;
        wrappers.push(
            project_const_template_view(wrapper_view, prepared, fold_context, visiting)?.template,
        );
    }

    Ok(wrappers)
}

fn project_template_kind(
    kind: &TemplateType,
    string_table: &StringTable,
) -> Result<PublicConstTemplateKind, TemplateNormalizationError> {
    match kind {
        TemplateType::String | TemplateType::StringFunction => Ok(PublicConstTemplateKind::Wrapper),
        TemplateType::SlotInsert(key) => Ok(PublicConstTemplateKind::SlotInsert(project_slot_key(
            key,
            string_table,
        ))),
        _ => Err(CompilerError::compiler_error(
            "Public const-template projection received a non-wrapper template kind.",
        )
        .into()),
    }
}

fn project_slot_key(key: &SlotKey, string_table: &StringTable) -> PublicTemplateSlotKey {
    match key {
        SlotKey::Default => PublicTemplateSlotKey::Default,
        SlotKey::Named(name) => {
            PublicTemplateSlotKey::Named(string_table.resolve(*name).to_owned())
        }
        SlotKey::Positional(position) => PublicTemplateSlotKey::Positional(*position),
    }
}
