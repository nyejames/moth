//! Binds authored constant initializers to their content in the constant store.
//!
//! WHAT: projects template-valued constant initializers through their exact TIR view, ingests the
//! result as constant content, and chooses the initializer declarations keep after binding.
//! WHY: header-time module constants and parse-time body-local constants bind here. Constant
//! content is owned by the constant store, so this binding must not depend on AST finalization,
//! which only reads the bound content afterwards.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_eval::FoldServices;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstTemplateValue, ConstValueId, ConstValueStore,
    ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionConstructionFailure, ExpressionOperationView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Template, TemplateConstValueKind, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, SlotOccurrenceId, TemplatePreparation, TemplatePreparationMode,
    TemplatePreparationOutcome, TemplateTirPhase, TirView, TirViewIdentity,
    fold_prepared_const_template_pattern, prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicTemplateSlotKey,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use rustc_hash::{FxHashMap, FxHashSet};

/// One exact module-constant template projection produced by constant binding.
///
/// WHAT: pairs preparation classification with the one public template projection and folded
///       scalar/provenance result needed by the compact constant store.
/// WHY: store construction must consume this result instead of preparing and folding the same
///       TIR view a second time.
#[derive(Clone)]
pub(crate) struct ProjectedConstTemplateValue {
    pub(crate) kind: TemplateConstValueKind,
    pub(crate) public: Option<PublicConstTemplate>,
    pub(crate) folded: Option<ConstStringValue>,
    pub(crate) provenance: SyntheticInterfaceProvenance,
}

fn collect_expression_templates(
    root: ExprId,
    builder: &AstExpressionBuilder,
    visited: &mut FxHashSet<ExprId>,
    templates: &mut Vec<Template>,
) -> Result<(), CompilerError> {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let view = builder.view(id)?;
        if let ExpressionOperationView::Template(template) = view.operation() {
            templates.push(template.clone());
        }
        view.visit_child_ids(&mut |child| pending.push(child))?;
    }
    Ok(())
}

fn template_projection_identity(
    template: &Template,
    store: &crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
) -> Result<TirViewIdentity, CompilerError> {
    let reference = template.tir_reference;
    Ok(TirView::with_minimum_phase(
        store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?
    .identity())
}

/// Capture template projections before ingestion mutably borrows the constant owner.
pub(crate) fn resolve_const_expression_content(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    type_environment: &TypeEnvironment,
    scope: &ScopeContext,
    strings: &mut StringTable,
) -> Result<ConstValueId, ConstValueStoreError> {
    let mut templates = Vec::new();
    collect_expression_templates(root, builder, &mut FxHashSet::default(), &mut templates)?;
    let store = scope.template_ir_store.borrow();
    let resources = scope
        .file_value_resolution
        .as_ref()
        .map(|services| services.module_resources.borrow());
    let mut projections = FxHashMap::default();
    for template in templates {
        let identity = template_projection_identity(&template, &store)?;
        if projections.contains_key(&identity) {
            continue;
        }
        let projected = project_const_template_value(
            &template,
            &store,
            builder,
            constants,
            FoldServices::new(
                strings,
                None,
                Some(scope.external_package_registry.as_ref()),
            ),
            scope.template_const_loop_iteration_limit,
            resources.as_deref(),
        )?;
        projections.insert(identity, projected);
    }
    constants.ingest_view(builder.view(root)?, type_environment, &mut |template| {
        let identity = template_projection_identity(template, &store)?;
        let projected = projections
            .get(&identity)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Constant ingestion has no exact-view template projection.",
                )
            })?
            .clone();
        const_template_value_from_projection(projected, template)
    })
}

/// Choose the initializer finalization reads for a compile-time declaration once its content is bound.
///
/// WHAT: an authored template initializer is kept so finalization normalises it. Any other
/// initializer is replaced by the folded constant that carries the bound content.
/// WHY: finalization inlines same-module templates and composes their slots from the authored
/// expression. A folded constant there turns a later reference such as `[inner]` into a runtime
/// reference. Module and body-local constants share this rule so they cannot diverge.
pub(crate) fn bound_constant_initializer(
    declaration: &Declaration,
    authored: ExprId,
    authored_is_template: bool,
    content: ConstValueId,
    builder: &mut AstExpressionBuilder,
) -> Result<ExprId, ExpressionConstructionFailure> {
    if authored_is_template {
        return Ok(authored);
    }
    let constant = builder.constant(
        content,
        declaration.type_id,
        builder.view(authored)?.span(),
        declaration.value_mode,
    )?;
    Ok(builder
        .preserve_authored_metadata(constant, builder.own(authored))?
        .id())
}

/// Classify one template projection into the value the module constant store holds.
///
/// WHAT: maps const-template classifications onto the store's template payload and rejects the
/// `NonConst` classification.
/// WHY: the store must not re-derive template identity, and the classification rule is the same
/// for every module constant, so it belongs beside the projection that produced it rather than
/// inline at each binding site.
pub(crate) fn const_template_value_from_projection(
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
            template.span,
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
                value: string,
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

        // Preparation never publishes non-const values, so the guard above already rejected them.
        TemplateConstValueKind::NonConst => {
            Err(CompilerError::compiler_error(
                "A non-const template classification reached module-constant store construction with a public projection.",
            )
            .into())
        }
    }
}

/// Prepare and fold one const template into its single projection.
///
/// WHAT: classifies the template through its exact effective TIR view, then folds that view once
/// into both the owned public projection and the scalar emission.
/// WHY: the module-constant store, the public interface and generated materialisation all consume
/// the same result, so the view must be prepared and folded exactly once per template.
pub(crate) fn project_const_template_value(
    template: &Template,
    store: &crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    fold_services: FoldServices<'_>,
    template_const_loop_iteration_limit: usize,
    module_resources: Option<&ModuleResourceTable>,
) -> Result<ProjectedConstTemplateValue, ConstValueStoreError> {
    let FoldServices {
        string_table,
        scope: _,
        registry,
    } = fold_services;
    let reference = template.tir_reference;
    let view = TirView::with_minimum_phase(
        store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    let prepared = prepare_tir_view(
        &view,
        &*expression_builder,
        &*const_values,
        TemplatePreparationMode::ConstRequired,
        registry,
    )?;
    let kind = prepared.facts.final_value_kind;
    let publish = matches!(prepared.outcome, TemplatePreparationOutcome::Foldable)
        || matches!(
            prepared.outcome,
            TemplatePreparationOutcome::SlotInsertHelper
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
        let mut fold_context = TirFoldContext::new(
            &mut *string_table,
            expression_builder,
            const_values,
            registry,
            template_const_loop_iteration_limit,
        );
        let mut visiting = FxHashSet::default();
        let projected = project_const_template_view(
            view,
            prepared,
            &mut fold_context,
            &mut visiting,
            module_resources,
        )?;
        (projected.template, projected.emission, projected.provenance)
    };
    let folded = match kind {
        TemplateConstValueKind::RenderableString | TemplateConstValueKind::WrapperTemplate => {
            Some(match emission {
                TemplateEmission::NoOutput => ConstStringValue::Text(string_table.intern("")),
                TemplateEmission::Output(value) => value,
            })
        }
        TemplateConstValueKind::SlotInsertHelper | TemplateConstValueKind::NonConst => None,
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
    fold_context: &mut TirFoldContext<'_>,
    visiting: &mut FxHashSet<TirViewIdentity>,
    module_resources: Option<&ModuleResourceTable>,
) -> Result<ProjectedTemplateView, ConstValueStoreError> {
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
    let mut pieces = Vec::with_capacity(pattern.pieces.len());
    let mut string_run = Vec::new();
    let mut run_has_structural = false;
    let provenance = pattern.provenance.clone();
    for piece in pattern.pieces {
        match piece {
            FoldedConstTemplatePiece::Text(text) => {
                string_run.push(ConstStringPiece::Text(
                    fold_context.string_table.intern(&text),
                ));
            }
            FoldedConstTemplatePiece::Resource(resource) => {
                string_run.push(ConstStringPiece::Resource(resource));
                run_has_structural = true;
            }
            FoldedConstTemplatePiece::SiteRoot => {
                string_run.push(ConstStringPiece::SiteRoot);
                run_has_structural = true;
            }
            FoldedConstTemplatePiece::Slot(occurrence) => {
                if let Some(text) = project_owned_string_run(
                    &mut string_run,
                    run_has_structural,
                    module_resources,
                    fold_context.string_table,
                )? {
                    pieces.push(PublicConstTemplatePiece::Text(text));
                }
                run_has_structural = false;
                pieces.push(PublicConstTemplatePiece::Slot(project_slot(
                    &view,
                    occurrence,
                    fold_context,
                    visiting,
                    module_resources,
                )?));
            }
        }
    }
    if let Some(text) = project_owned_string_run(
        &mut string_run,
        run_has_structural,
        module_resources,
        fold_context.string_table,
    )? {
        pieces.push(PublicConstTemplatePiece::Text(text));
    }

    let conditional_child_wrappers = project_wrapper_set(
        &view,
        template.conditional_child_wrapper_set,
        fold_context,
        visiting,
        module_resources,
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
/// Intermediate owned projection of one exact template view.
///
/// WHAT: keeps the public composition tree, folded structural emission and provenance together
///       until the caller stores the final module-constant projection.
/// WHY: all three results come from one prepared/folded view and must not be recomputed through
///      separate TIR traversals.
struct ProjectedTemplateView {
    template: PublicConstTemplate,
    emission: TemplateEmission,
    provenance: SyntheticInterfaceProvenance,
}

/// Converts one contiguous non-slot projection run to the shared owned string vocabulary.
///
/// WHAT: resolves module-local text IDs and, for structural runs, delegates resource-origin
///       conversion to [`crate::compiler_frontend::folded_value::owned_folded_string_from_const_string`].
/// WHY: const-template projections outlive the donor TIR store, so only stable resource origins may
///      cross the public boundary and text coalescing must stop at each structural anchor.
fn project_owned_string_run(
    string_run: &mut Vec<ConstStringPiece>,
    has_structural: bool,
    module_resources: Option<&ModuleResourceTable>,
    string_table: &StringTable,
) -> Result<Option<OwnedFoldedString>, ConstValueStoreError> {
    if string_run.is_empty() {
        return Ok(None);
    }

    if has_structural {
        let resources = module_resources.ok_or_else(|| {
            CompilerError::compiler_error(
                "Const-template projection reached a structural string without the issuing module resource table.",
            )
        })?;
        let value = ConstStringValue::Pieces(std::mem::take(string_run));
        return Ok(Some(
            crate::compiler_frontend::folded_value::owned_folded_string_from_const_string(
                &value,
                resources,
                string_table,
            )?,
        ));
    }

    let mut text = String::new();
    for piece in string_run.drain(..) {
        match piece {
            ConstStringPiece::Text(text_id) => {
                text.push_str(string_table.resolve(text_id));
            }
            ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                return Err(CompilerError::compiler_error(
                    "Const-template projection string-run invariant violated: a structural piece reached text-only conversion.",
                )
                .into());
            }
        }
    }
    Ok(Some(OwnedFoldedString::Text(text)))
}

/// Projects one slot occurrence and its nested wrapper contexts.
///
/// WHAT: converts the module-local slot key and recursively projects both wrapper sets while
///       retaining the slot's authored occurrence identity.
/// WHY: public const-template consumers need composition structure after the donor TIR store is
///      released, but no local TIR ID may cross the boundary.
fn project_slot(
    view: &TirView<'_>,
    occurrence: SlotOccurrenceId,
    fold_context: &mut TirFoldContext<'_>,
    visiting: &mut FxHashSet<TirViewIdentity>,
    module_resources: Option<&ModuleResourceTable>,
) -> Result<PublicConstTemplateSlot, ConstValueStoreError> {
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
            module_resources,
        )?,
        child_wrappers: project_wrapper_set(
            view,
            placeholder.child_wrapper_set,
            fold_context,
            visiting,
            module_resources,
        )?,
        skip_parent_child_wrappers: placeholder.skip_parent_child_wrappers,
    })
}
/// Projects one wrapper-set reference list recursively.
///
/// WHAT: follows each module-local wrapper view and reuses the same structural string conversion
///       and cycle guard as its parent template.
/// WHY: nested wrappers are authored composition, not rendered text, and must remain portable.
fn project_wrapper_set(
    parent_view: &TirView<'_>,
    wrapper_set_id: Option<crate::compiler_frontend::ast::templates::tir::TemplateWrapperSetId>,
    fold_context: &mut TirFoldContext<'_>,
    visiting: &mut FxHashSet<TirViewIdentity>,
    module_resources: Option<&ModuleResourceTable>,
) -> Result<Vec<PublicConstTemplate>, ConstValueStoreError> {
    let Some(wrapper_set_id) = wrapper_set_id else {
        return Ok(Vec::new());
    };
    let wrapper_set = parent_view
        .store()
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            CompilerError::compiler_error("Public const-template wrapper set is missing.")
        })?;
    let references = &wrapper_set.wrappers;
    let mut wrappers = Vec::with_capacity(references.len());

    for reference in references {
        let wrapper_view = parent_view.wrapper(*reference)?;
        let prepared = prepare_tir_view(
            &wrapper_view,
            &*fold_context.expression_builder,
            &*fold_context.const_values,
            TemplatePreparationMode::ConstRequired,
            fold_context.external_package_registry,
        )?;
        wrappers.push(
            project_const_template_view(
                wrapper_view,
                prepared,
                fold_context,
                visiting,
                module_resources,
            )?
            .template,
        );
    }

    Ok(wrappers)
}

fn project_template_kind(
    kind: &TemplateType,
    string_table: &StringTable,
) -> Result<PublicConstTemplateKind, ConstValueStoreError> {
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
