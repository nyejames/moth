//! Public const-template projection at the AST finalization boundary.
//!
//! WHAT: folds exported-capable wrapper and slot-insert constants through the canonical TIR
//! reducer, then converts unresolved slots and wrapper sets into an owned interface value.
//! WHY: consumers need const-template composition semantics, but TIR stores, IDs, overlays and
//! donor-local strings must be dropped before a completed AST or public interface leaves AST.

use super::finalizer::AstFinalizer;
use super::normalize_ast::TemplateNormalizationError;
use super::template_helpers::make_fold_context;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::template::{SlotKey, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, SlotOccurrenceId, TemplateHelperKind, TemplatePreparation,
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TirView,
    TirViewIdentity, fold_prepared_const_template_pattern, prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::{
    PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicTemplateSlotKey,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};

/// Stable const-template projections consumed by public-interface construction and generated
/// materialisation.
///
/// The name index matches public export lookup, while the path index lets an in-flight generated
/// compilation rebuild the exact declaring-module constant in its fresh TIR store.
pub(super) struct ProjectedConstTemplates {
    pub(super) by_name: FxHashMap<String, PublicConstTemplate>,
    pub(super) by_path: FxHashMap<InternedPath, PublicConstTemplate>,
}

impl AstFinalizer<'_, '_> {
    pub(super) fn project_const_templates(
        &self,
        string_table: &mut StringTable,
    ) -> Result<ProjectedConstTemplates, TemplateNormalizationError> {
        let mut by_name = FxHashMap::default();
        let mut by_path = FxHashMap::default();
        let store = self.context.template_ir_store.borrow();

        for declaration in &self.environment.lookups.module_constants {
            let Some(projected) =
                self.project_template_expression(declaration, &store, string_table)?
            else {
                continue;
            };

            let defining_name = declaration.id.name_str(string_table).ok_or_else(|| {
                CompilerError::compiler_error(
                    "Public const-template declaration path has no defining name.",
                )
            })?;
            by_name.insert(defining_name.to_owned(), projected.clone());
            Self::insert_projected_template(&mut by_path, declaration, projected)?;
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

        Ok(ProjectedConstTemplates { by_name, by_path })
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
        let reference = template.tir_reference;
        let view = TirView::with_minimum_phase(
            store,
            reference.root,
            reference.phase,
            TemplateTirPhase::Composed,
            reference.context,
        )?;
        let prepared = prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)?;
        let publish = matches!(prepared.outcome, TemplatePreparationOutcome::Foldable)
            || matches!(
                prepared.outcome,
                TemplatePreparationOutcome::Helper(TemplateHelperKind::SlotInsert)
            );
        if !publish {
            return Ok(None);
        }

        let mut fold_context = make_fold_context(
            string_table,
            self.context.template_const_loop_iteration_limit,
        );
        let mut visiting = FxHashSet::default();
        Ok(Some(project_const_template_view(
            view,
            prepared,
            &mut fold_context,
            &mut visiting,
        )?))
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

fn project_const_template_view(
    view: TirView<'_>,
    prepared: TemplatePreparation,
    fold_context: &mut crate::compiler_frontend::ast::templates::template_folding::TirFoldContext<
        '_,
    >,
    visiting: &mut FxHashSet<TirViewIdentity>,
) -> Result<PublicConstTemplate, TemplateNormalizationError> {
    if !visiting.insert(view.identity()) {
        return Err(CompilerError::compiler_error(
            "Public const-template projection encountered a recursive wrapper view after preparation.",
        )
        .into());
    }

    let template = view.root_template()?;
    let kind = project_template_kind(&template.kind, fold_context.string_table)?;
    let pattern = fold_prepared_const_template_pattern(prepared, view.clone(), fold_context)?;
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

    Ok(PublicConstTemplate {
        kind,
        pieces,
        conditional_child_wrappers,
    })
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
        wrappers.push(project_const_template_view(
            wrapper_view,
            prepared,
            fold_context,
            visiting,
        )?);
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
