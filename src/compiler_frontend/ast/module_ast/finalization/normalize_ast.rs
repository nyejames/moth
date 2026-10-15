//! Normalizes ID-backed AST roots before HIR consumption.
//!
//! Templates fold into constant content or materialize neutral runtime handoffs from the exact
//! prepared TIR view. Changed rows promote through scratch; unchanged rows retain their IDs.
//! TIR overlays receive only committed IDs and remain readable until the final-reader barrier.

use super::finalizer::AstFinalizer;
use super::template_helpers::{
    FinalizedTemplateValue, TemplateValueFinalizationInputs, finalize_template_value,
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, LoopBindings, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    assert_message_escape_diagnostic, assertion_condition_is_statically_true,
};
use crate::compiler_frontend::ast::expressions::expression::FallibleHandling;
use crate::compiler_frontend::ast::expressions::expression_rewrite::{
    promote_replacement, replacement_input, rewrite_expression_children, rewrite_place_indices,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CompletedOperation, ExprId, ExpressionConstructionFailure,
    ExpressionOperationView, OccurrenceFacts, OwnedCompletedExpressionInput, PlaceProjectionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::expressions::failure_classification::collect_expression_failure_facts;
use crate::compiler_frontend::ast::expressions::failure_facts::ImplicitFailureSource;
use crate::compiler_frontend::ast::module_ast::environment::ResolvedPublicTypeRootKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, RuntimeTemplateReason, TemplateIrStore, TemplatePreparation,
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TemplateTirReference,
    TirView, collect_effective_tir_expression_overlay_payloads, finalized_tir_view_for_template,
    owned_runtime_slot_handoff_for_prepared_view, owned_runtime_template_handoff_for_prepared_view,
    replace_expression_overlay_entries,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateSlotReason, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;

struct TemplateNormalizationContext<'strings> {
    template_const_loop_iteration_limit: usize,
    expression_builder: &'strings mut AstExpressionBuilder,
    const_values: &'strings mut ConstValueStore,
    string_table: &'strings mut StringTable,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
    module_resources:
        Option<Rc<RefCell<crate::compiler_frontend::paths::module_resources::ModuleResourceTable>>>,
    expression_pending: Vec<NormalizationFrame>,
    expression_results: Vec<(ExprId, ExprId)>,
}

enum NormalizationFrame {
    Visit(ExprId),
    Finish {
        expression: ExprId,
        child_count: usize,
    },
}

impl AstFinalizer<'_, '_> {
    /// Normalizes all templates in the AST for HIR consumption.
    ///
    /// WHAT: Traverses all AST nodes and normalizes embedded templates by
    /// folding constants and materializing runtime handoffs.
    ///
    /// WHY: Ensures HIR receives semantically complete templates without
    /// needing to understand template composition or folding rules.
    pub(super) fn normalize_ast_templates_for_hir(
        &mut self,
        ast: &mut [AstNode],
        string_table: &mut StringTable,
    ) -> Result<(), TemplateNormalizationError> {
        let mut normalization_context = TemplateNormalizationContext {
            template_const_loop_iteration_limit: self.context.template_const_loop_iteration_limit,
            string_table,
            expression_builder: &mut self.context.expression_builder,
            const_values: &mut self.context.const_values,
            template_ir_store: Rc::clone(&self.context.template_ir_store),
            module_resources: self
                .context
                .file_value_resolution
                .as_ref()
                .map(|services| Rc::clone(&services.module_resources)),
            expression_pending: Vec::new(),
            expression_results: Vec::new(),
        };
        for node in ast {
            normalize_ast_node_templates(node, &mut normalization_context)?;
        }

        Ok(())
    }

    /// Joins once-normalized emitted declarations into retained public roots.
    ///
    /// Defaults already name folded constant content. Generic declarations therefore need
    /// authoritative declaration-identity checks here, not a second fold.
    pub(super) fn synchronize_normalized_public_defaults(
        &mut self,
        emitted_ast: &[AstNode],
    ) -> Result<(), TemplateNormalizationError> {
        let EmittedDeclarationDefaults {
            function_signatures_by_path: normalized_function_signatures_by_path,
            struct_fields_by_path: normalized_struct_fields_by_path,
        } = collect_emitted_declaration_defaults(emitted_ast)?;

        let generic_function_templates_by_path =
            &self.environment.lookups.generic_function_templates_by_path;
        let type_environment = &self.environment.type_environment;

        // Ordinary emitted declarations provide the one normalized root copy. Generic
        // declarations retain canonical default handles from their declaration boundary.
        let mut root_table = std::mem::take(&mut self.environment.resolved_public_type_roots);
        for root in &mut root_table.roots {
            match &mut root.kind {
                ResolvedPublicTypeRootKind::Function {
                    signature,
                    generic_parameter_list_id,
                } => {
                    if let Some(root_generic_parameter_list_id) = generic_parameter_list_id.as_ref()
                    {
                        // Generic free function: must have no emitted node.
                        if normalized_function_signatures_by_path.contains_key(&root.path) {
                            return Err(CompilerError::compiler_error(format!(
                                "public default synchronization: a generic free-function root at {:?} has an emitted declaration node; generic functions must not emit an ordinary declaration",
                                root.path
                            ))
                            .into());
                        }
                        let template = generic_function_templates_by_path
                            .get(&root.path)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "public default synchronization: a generic free-function root at {:?} has no retained generic-template metadata; every generic function must carry its source file",
                                    root.path
                                ))
                            })?;
                        if template.generic_parameter_list_id != *root_generic_parameter_list_id {
                            return Err(CompilerError::compiler_error(format!(
                                "public default synchronization: a generic free-function root at {:?} has a generic-parameter-list mismatch: root GenericParameterListId({}) vs retained template GenericParameterListId({})",
                                root.path,
                                root_generic_parameter_list_id.0,
                                template.generic_parameter_list_id.0
                            ))
                            .into());
                        }
                    } else {
                        // Non-generic free function: must have exactly one emitted signature.
                        let normalized = normalized_function_signatures_by_path
                            .get(&root.path)
                            .copied()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "public default synchronization: a non-generic free-function root at {:?} has no emitted declaration node; only generic functions may omit an emitted node",
                                    root.path
                                ))
                            })?;
                        *signature = normalized.clone();
                    }
                }
                ResolvedPublicTypeRootKind::Struct { type_id, fields } => {
                    let definition = type_environment.get(*type_id).ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "public default synchronization: a struct root at {:?} references TypeId({}) which has no type definition in the type environment",
                            root.path,
                            type_id.0
                        ))
                    })?;
                    let TypeDefinition::Struct(struct_definition) = definition else {
                        return Err(CompilerError::compiler_error(format!(
                            "public default synchronization: a struct root at {:?} references TypeId({}) which is not a struct definition",
                            root.path,
                            type_id.0
                        ))
                        .into());
                    };
                    if struct_definition.generic_parameters.is_some() {
                        // Generic struct: must have no emitted node.
                        if normalized_struct_fields_by_path.contains_key(&root.path) {
                            return Err(CompilerError::compiler_error(format!(
                                "public default synchronization: a generic struct root at {:?} (TypeId({})) has an emitted declaration node; generic structs must not emit an ordinary declaration",
                                root.path,
                                type_id.0
                            ))
                            .into());
                        }
                    } else {
                        // Non-generic struct: must have exactly one emitted declaration.
                        let normalized = normalized_struct_fields_by_path
                            .get(&root.path)
                            .copied()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "public default synchronization: a non-generic struct root at {:?} (TypeId({})) has no emitted declaration node; only generic structs may omit an emitted node",
                                    root.path,
                                    type_id.0
                                ))
                            })?;
                        *fields = normalized.to_vec();
                    }
                }
                _ => {}
            }
        }

        // Collect the root receiver path set once: a duplicate is an internal invariant
        // violation, and the set classifies catalog signatures that join no root entry.
        let root_receiver_paths = unique_root_receiver_paths(&root_table.receiver_methods)?;

        // Ordinary receiver roots join one emitted signature. Generic and imported signatures
        // already carry canonical folded defaults and retain their existing contract.
        for root_entry in &mut root_table.receiver_methods {
            // Imported receiver contracts have no consumer-emitted declaration node to
            // synchronize against.
            if self
                .environment
                .lookups
                .imported_functions_by_local_path
                .contains_key(&root_entry.function_path)
            {
                continue;
            }
            if generic_function_templates_by_path.contains_key(&root_entry.function_path) {
                // Generic receiver method: must have no emitted node.
                if normalized_function_signatures_by_path.contains_key(&root_entry.function_path) {
                    return Err(CompilerError::compiler_error(format!(
                        "public default synchronization: a generic receiver method at {:?} has an emitted declaration node; generic functions must not emit an ordinary declaration",
                        root_entry.function_path
                    ))
                    .into());
                }
            } else {
                // Non-generic receiver method: must have exactly one emitted node.
                let normalized = normalized_function_signatures_by_path
                    .get(&root_entry.function_path)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "public default synchronization: a non-generic receiver method at {:?} has no emitted declaration node; only generic functions may omit an emitted node",
                            root_entry.function_path
                        ))
                    })?;
                root_entry.signature = normalized.clone();
            }
        }

        // Private generic methods have no emitted declaration, even when they have no root.
        // Check that declaration-identity invariant without cloning their folded defaults.
        for catalog_entry in self
            .environment
            .lookups
            .receiver_methods
            .by_function_path
            .values()
        {
            // Public receiver identities were checked by the root join above.
            if root_receiver_paths.contains(&catalog_entry.function_path) {
                continue;
            }

            // Imported receiver contracts keep the provider's normalized defaults.
            if self
                .environment
                .lookups
                .imported_functions_by_local_path
                .contains_key(&catalog_entry.function_path)
            {
                continue;
            }

            // Ordinary receiver methods joined their emitted declaration above.
            if !generic_function_templates_by_path.contains_key(&catalog_entry.function_path) {
                continue;
            }

            if normalized_function_signatures_by_path.contains_key(&catalog_entry.function_path) {
                return Err(CompilerError::compiler_error(format!(
                    "public default synchronization: a generic receiver method at {:?} has an emitted declaration node; generic functions must not emit an ordinary declaration",
                    catalog_entry.function_path
                ))
                .into());
            }
        }

        self.environment.resolved_public_type_roots = root_table;

        Ok(())
    }
}

/// Borrowed normalized declaration facts indexed for the public-root synchronization join.
///
/// Retained roots copy normalized emitted signatures and fields.
struct EmittedDeclarationDefaults<'ast> {
    function_signatures_by_path: FxHashMap<PathId, &'ast FunctionSignature>,
    struct_fields_by_path: FxHashMap<PathId, &'ast [Declaration]>,
}

/// Collect normalized function signatures and struct fields from the once-normalized emitted
/// AST into total lookup maps keyed by declaration path.
///
/// WHY: a duplicate emitted path is an internal invariant violation, not a silent overwrite.
/// Extracting the collection makes that invariant independently testable and keeps the
/// synchronization orchestration focused on its joins.
fn collect_emitted_declaration_defaults(
    emitted_ast: &[AstNode],
) -> Result<EmittedDeclarationDefaults<'_>, CompilerError> {
    let mut normalized_function_signatures_by_path: FxHashMap<PathId, &FunctionSignature> =
        FxHashMap::default();
    let mut normalized_struct_fields_by_path: FxHashMap<PathId, &[Declaration]> =
        FxHashMap::default();

    for node in emitted_ast {
        if let NodeKind::Function(path, signature, _) = &node.kind {
            match normalized_function_signatures_by_path.entry(path.to_owned()) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    return Err(CompilerError::compiler_error(format!(
                        "public default synchronization: duplicate emitted function declaration at path {:?}; the emitted AST must not contain two nodes with the same path",
                        path
                    )));
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(signature);
                }
            }
        } else if let NodeKind::StructDefinition(path, fields) = &node.kind {
            match normalized_struct_fields_by_path.entry(path.to_owned()) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    return Err(CompilerError::compiler_error(format!(
                        "public default synchronization: duplicate emitted struct declaration at path {:?}; the emitted AST must not contain two nodes with the same path",
                        path
                    )));
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(fields.as_slice());
                }
            }
        }
    }

    Ok(EmittedDeclarationDefaults {
        function_signatures_by_path: normalized_function_signatures_by_path,
        struct_fields_by_path: normalized_struct_fields_by_path,
    })
}

/// Collect receiver-root paths, rejecting duplicates before joining normalized defaults.
///
/// The set also distinguishes private catalog entries from already-normalized roots.
fn unique_root_receiver_paths(
    receiver_methods: &[ReceiverMethodEntry],
) -> Result<FxHashSet<PathId>, CompilerError> {
    let mut seen_paths: FxHashSet<PathId> = FxHashSet::default();
    for entry in receiver_methods {
        if !seen_paths.insert(entry.function_path) {
            return Err(CompilerError::compiler_error(format!(
                "public default synchronization: a root table receiver method at {:?} is duplicated; each root receiver method must join exactly one catalog entry",
                entry.function_path
            )));
        }
    }
    Ok(seen_paths)
}

/// Normalizes templates in an AST node by routing to category-specific handlers.
///
/// WHAT: Dispatcher function that routes AST nodes to specialized normalization
/// functions based on node category (control flow, declarations, calls, etc.).
///
/// WHY: Keeps the main normalization logic organized by node category while
/// providing a single entry point for recursive traversal.
fn normalize_ast_node_templates(
    node: &mut AstNode,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    let mut place_result = Ok(());
    node.visit_place_ids_mut(&mut |place| {
        if place_result.is_err() {
            return;
        }
        place_result = (|| -> Result<(), TemplateNormalizationError> {
            let indices: Vec<_> = context
                .expression_builder
                .place(*place)?
                .projections()
                .filter_map(|projection| match projection {
                    PlaceProjectionView::Index { index, .. } => Some(index.id()),
                    PlaceProjectionView::Field { .. } => None,
                })
                .collect();
            let mut replacements = Vec::with_capacity(indices.len());
            for index in indices {
                let mut replacement = index;
                normalize_expression_templates(&mut replacement, context)?;
                replacements.push((index, replacement));
            }
            let replacements = context.expression_builder.own_slice(&replacements);
            *place = rewrite_place_indices(context.expression_builder, *place, replacements)?;
            Ok(())
        })();
    });
    place_result?;
    increment_ast_counter(AstCounter::TemplateNormalizationNodesVisited);

    match &mut node.kind {
        NodeKind::If(..)
        | NodeKind::Match { .. }
        | NodeKind::LexicalScope { .. }
        | NodeKind::RangeLoop { .. }
        | NodeKind::CollectionLoop { .. }
        | NodeKind::WhileLoop(_, _) => normalize_control_flow_templates(node, context),

        NodeKind::VariableDeclaration(_)
        | NodeKind::Assignment { .. }
        | NodeKind::StructDefinition(_, _) => normalize_declaration_templates(node, context),

        NodeKind::MultiBind { value, .. } | NodeKind::ExpressionStatement(value) => {
            normalize_expression_templates(value, context)
        }

        NodeKind::Function(_, _, body) => normalize_nodes(body, context),

        NodeKind::Return(values) => normalize_expressions(values, context),

        NodeKind::ReturnError(value) => normalize_expression_templates(value, context),

        // Runtime fragment push — normalize the template expression it carries.
        NodeKind::PushStartRuntimeFragment(expression) => {
            normalize_expression_templates(expression, context)
        }

        NodeKind::Assert { condition, message } => {
            normalize_expression_templates(condition, context)?;

            {
                let store = context.template_ir_store.borrow();
                if let Some(diagnostic) = assert_message_escape_diagnostic(
                    context.expression_builder.view(*message)?,
                    &store,
                    &*context.expression_builder,
                )? {
                    return Err(diagnostic.into());
                }
            }

            // The message still needs the ordinary AST normalization path so its nested
            // templates reach the finalized boundary and retain their frontend diagnostics.
            normalize_expression_templates(message, context)?;

            let store = context.template_ir_store.borrow();
            if let Some(diagnostic) = assert_message_escape_diagnostic(
                context.expression_builder.view(*message)?,
                &store,
                &*context.expression_builder,
            )? {
                return Err(diagnostic.into());
            }

            Ok(())
        }

        // Terminal nodes (no templates to normalize)
        NodeKind::Break | NodeKind::Continue => Ok(()),
        NodeKind::ThenValue(produced_values) => {
            for expression in &mut produced_values.expressions {
                normalize_expression_templates_with_context(
                    expression,
                    context,
                    HelperArtifactPolicy::RejectFinalHelperValue,
                )?;
            }
            Ok(())
        }
    }
}

/// Normalizes templates in control flow nodes (if, match, loops).
///
/// WHAT: Handles normalization for if statements, match expressions, and all
/// loop types (range, collection, while) by recursively normalizing conditions
/// and body statements.
///
/// WHY: Control flow nodes have similar structure (condition + body) and can
/// be handled together to avoid duplication.
fn normalize_control_flow_templates(
    node: &mut AstNode,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    match &mut node.kind {
        NodeKind::If(condition, then_body, else_body, _) => {
            normalize_expression_templates(condition, context)?;
            normalize_nodes(then_body, context)?;

            if let Some(else_body) = else_body {
                normalize_nodes(else_body, context)?;
            }

            Ok(())
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            exhaustiveness: _,
        } => {
            normalize_expression_templates(scrutinee, context)?;

            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    normalize_expression_templates(guard, context)?;
                }

                normalize_nodes(&mut arm.body, context)?;
            }

            if let Some(default_body) = default {
                normalize_nodes(default_body, context)?;
            }

            Ok(())
        }

        NodeKind::LexicalScope { body } => normalize_nodes(body, context),

        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            normalize_loop_bindings(bindings, context)?;
            normalize_expression_templates(&mut range.start, context)?;
            normalize_expression_templates(&mut range.end, context)?;

            if let Some(step) = &mut range.step {
                normalize_expression_templates(step, context)?;
            }

            normalize_nodes(body, context)
        }

        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            normalize_loop_bindings(bindings, context)?;
            normalize_expression_templates(iterable, context)?;
            normalize_nodes(body, context)
        }

        NodeKind::WhileLoop(condition, body) => {
            normalize_expression_templates(condition, context)?;
            normalize_nodes(body, context)
        }

        _ => unreachable!("normalize_control_flow_templates called with non-control-flow node"),
    }
}

/// Normalizes templates in declaration and assignment nodes.
///
/// WHAT: handles variable declarations, assignments and struct definitions by normalising
/// value expressions and fields through the completed-expression walker.
///
/// WHY: Declaration nodes have similar structure (identifier + value) and can
/// be handled together to avoid duplication.
fn normalize_declaration_templates(
    node: &mut AstNode,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    match &mut node.kind {
        NodeKind::VariableDeclaration(declaration) => {
            if let Some(initializer) = declaration.value.initializer_mut() {
                normalize_expression_templates(initializer, context)?;
            }
            Ok(())
        }

        NodeKind::Assignment { value, .. } => normalize_expression_templates(value, context),

        NodeKind::StructDefinition(_, fields) => {
            for field in fields {
                if let Some(initializer) = field.value.initializer_mut() {
                    normalize_expression_templates_with_context(
                        initializer,
                        context,
                        HelperArtifactPolicy::AllowNestedHelperContent,
                    )?;
                }
            }
            Ok(())
        }

        _ => unreachable!("normalize_declaration_templates called with non-declaration node"),
    }
}

/// Normalizes templates in fallible handling constructs.
///
/// WHAT: Handles normalization for fallible handling by recursively normalizing handler bodies.
///
/// WHY: Fallible handlers can contain templates that must be normalized for HIR.
fn normalize_fallible_handling_templates(
    handling: &mut FallibleHandling,
    context: &mut TemplateNormalizationContext<'_>,
    _helper_artifact_policy: HelperArtifactPolicy,
) -> Result<(), TemplateNormalizationError> {
    match handling {
        FallibleHandling::Handler { body, .. } => normalize_nodes(body, context),
        FallibleHandling::Propagate => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelperArtifactPolicy {
    RejectFinalHelperValue,
    AllowNestedHelperContent,
}

fn normalize_nodes(
    nodes: &mut [AstNode],
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    for node in nodes {
        normalize_ast_node_templates(node, context)?;
    }

    Ok(())
}

fn normalize_expressions(
    expressions: &mut [ExprId],
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    for expression in expressions {
        normalize_expression_templates(expression, context)?;
    }

    Ok(())
}

fn normalize_loop_bindings(
    bindings: &mut LoopBindings,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    if let Some(item_binding) = &mut bindings.item
        && let Some(initializer) = item_binding.value.initializer_mut()
    {
        normalize_expression_templates(initializer, context)?;
    }

    if let Some(index_binding) = &mut bindings.index
        && let Some(initializer) = index_binding.value.initializer_mut()
    {
        normalize_expression_templates(initializer, context)?;
    }

    Ok(())
}

#[derive(Default)]
struct AssertionDiscardScratch {
    pending: Vec<AssertionDiscardFrame>,
    results: Vec<(ExprId, ExprId)>,
}

enum AssertionDiscardFrame {
    Visit(ExprId),
    Finish {
        original: ExprId,
        expression: ExprId,
        child_count: usize,
    },
}

/// Discards compile-time-inactive assertion messages after final AST type validation.
///
/// WHAT: walks the normalized AST and replaces only messages whose assertion condition is the
///       literal `true` with the canonical typed `none` expression.
/// WHY: normalized authored messages must remain visible to the authoritative finalized-TIR and
///       TypeId validation pass. This AST-owned cleanup runs after that pass and before const
///       facts, HIR, generated requests, link facts, target facts, or backend work can observe
///       the completed AST.
pub(super) fn discard_inactive_assertion_messages(
    ast: &mut [AstNode],
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<(), TemplateNormalizationError> {
    let mut scratch = AssertionDiscardScratch::default();
    discard_inactive_assertion_messages_in_nodes(
        ast,
        expression_builder,
        const_values,
        &mut scratch,
    )
}

fn discard_inactive_assertion_messages_in_nodes(
    ast: &mut [AstNode],
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    scratch: &mut AssertionDiscardScratch,
) -> Result<(), TemplateNormalizationError> {
    for node in ast {
        discard_statement_assertions(node, expression_builder, const_values)?;
        let mut result = Ok(());
        node.visit_expression_ids_mut(&mut |expression| {
            if result.is_ok() {
                result = discard_inactive_assertion_messages_in_expression(
                    expression,
                    expression_builder,
                    const_values,
                    scratch,
                );
            }
        });
        result?;
        let mut place_result = Ok(());
        node.visit_place_ids_mut(&mut |place| {
            if place_result.is_err() {
                return;
            }
            place_result = (|| -> Result<(), TemplateNormalizationError> {
                let indices: Vec<_> = expression_builder
                    .place(*place)?
                    .projections()
                    .filter_map(|projection| match projection {
                        PlaceProjectionView::Index { index, .. } => Some(index.id()),
                        PlaceProjectionView::Field { .. } => None,
                    })
                    .collect();
                let mut replacements = Vec::with_capacity(indices.len());
                for index in indices {
                    let mut replacement = index;
                    discard_inactive_assertion_messages_in_expression(
                        &mut replacement,
                        expression_builder,
                        const_values,
                        scratch,
                    )?;
                    replacements.push((index, replacement));
                }
                let replacements = expression_builder.own_slice(&replacements);
                *place = rewrite_place_indices(expression_builder, *place, replacements)?;
                Ok(())
            })();
        });
        place_result?;
    }
    Ok(())
}

fn discard_statement_assertions(
    node: &mut AstNode,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
) -> Result<(), TemplateNormalizationError> {
    if let NodeKind::Assert { condition, message } = &mut node.kind
        && assertion_condition_is_statically_true(expressions.view(*condition)?, constants)
    {
        replace_inactive_assertion_message(message, expressions, constants)?;
    }
    let mut visit_body = |body: &mut [AstNode]| -> Result<(), TemplateNormalizationError> {
        for node in body {
            discard_statement_assertions(node, expressions, constants)?;
        }
        Ok(())
    };
    match &mut node.kind {
        NodeKind::Function(_, _, body)
        | NodeKind::LexicalScope { body }
        | NodeKind::RangeLoop { body, .. }
        | NodeKind::CollectionLoop { body, .. }
        | NodeKind::WhileLoop(_, body) => visit_body(body)?,
        NodeKind::If(_, then_body, else_body, _) => {
            visit_body(then_body)?;
            if let Some(body) = else_body {
                visit_body(body)?;
            }
        }
        NodeKind::Match { arms, default, .. } => {
            for arm in arms {
                visit_body(&mut arm.body)?;
            }
            if let Some(body) = default {
                visit_body(body)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_expression(
    expression: &mut ExprId,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    scratch: &mut AssertionDiscardScratch,
) -> Result<(), TemplateNormalizationError> {
    // Value-body walks reuse scratch without consuming the enclosing expression's prefix.
    let pending_base = scratch.pending.len();
    let root = expressions.view(*expression)?;
    match root.operation() {
        ExpressionOperationView::Constant(_) | ExpressionOperationView::Reference(_) => {
            return Ok(());
        }
        ExpressionOperationView::Copy(_) => {
            root.visit_children(|child| {
                scratch
                    .pending
                    .push(AssertionDiscardFrame::Visit(child.id()));
            });
            let child_count = scratch.pending.len() - pending_base;
            if child_count == 0 {
                return Ok(());
            }
            scratch.pending.push(AssertionDiscardFrame::Finish {
                original: *expression,
                expression: *expression,
                child_count,
            });
            scratch.pending[pending_base..].reverse();
        }
        _ => scratch
            .pending
            .push(AssertionDiscardFrame::Visit(*expression)),
    }
    while scratch.pending.len() > pending_base {
        let frame = scratch
            .pending
            .pop()
            .expect("active scratch suffix is non-empty");
        match frame {
            AssertionDiscardFrame::Visit(original) => {
                let needs_block_visit = match expressions.view(original)?.operation() {
                    ExpressionOperationView::Constant(_)
                    | ExpressionOperationView::Reference(_) => {
                        scratch.results.push((original, original));
                        continue;
                    }
                    ExpressionOperationView::Copy(_) => false,
                    _ => true,
                };
                let mut normalized = original;
                if needs_block_visit {
                    discard_inactive_assertion_messages_in_block(
                        &mut normalized,
                        expressions,
                        constants,
                        scratch,
                    )?;
                }
                let pending = &mut scratch.pending;
                let start = pending.len();
                expressions
                    .view(normalized)?
                    .visit_children(|child| pending.push(AssertionDiscardFrame::Visit(child.id())));
                let child_count = pending.len() - start;
                if child_count == 0 {
                    scratch.results.push((original, normalized));
                    continue;
                }
                pending.push(AssertionDiscardFrame::Finish {
                    original,
                    expression: normalized,
                    child_count,
                });
                pending[start..].reverse();
            }
            AssertionDiscardFrame::Finish {
                original,
                expression: normalized,
                child_count,
            } => {
                let start = scratch.results.len() - child_count;
                let normalized = expressions.own(normalized);
                let replacements = expressions.own_slice(&scratch.results[start..]);
                let replacement =
                    rewrite_expression_children(expressions, normalized, replacements)?;
                scratch.results.truncate(start);
                scratch.results.push((original, replacement.id()));
            }
        }
    }
    if let Some((_, replacement)) = scratch.results.pop() {
        *expression = replacement;
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_block(
    expression: &mut ExprId,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    scratch: &mut AssertionDiscardScratch,
) -> Result<(), TemplateNormalizationError> {
    let block = match expressions.view(*expression)?.operation() {
        ExpressionOperationView::ValueBlock(block) => Some(block.clone()),
        _ => None,
    };
    if let Some(mut block) = block {
        let mut old_ids = Vec::new();
        block.visit_expression_ids(&mut |id| old_ids.push(id));
        let mut old_places = Vec::new();
        block.visit_place_ids(&mut |id| old_places.push(id));
        let mut bodies = |nodes: &mut [AstNode]| {
            discard_inactive_assertion_messages_in_nodes(nodes, expressions, constants, scratch)
        };
        match &mut block {
            ValueBlock::If(value) => {
                bodies(&mut value.then_body)?;
                bodies(&mut value.else_body)?;
            }
            ValueBlock::LexicalScope(value) => bodies(&mut value.body)?,
            ValueBlock::Match(value) => {
                for arm in &mut value.arms {
                    bodies(&mut arm.body)?;
                }
                if let Some(body) = &mut value.default {
                    bodies(body)?;
                }
            }
            ValueBlock::Catch(value) => {
                if let FallibleHandling::Handler { body, .. } = &mut value.handler {
                    bodies(body)?;
                }
            }
        }
        let mut new_ids = Vec::new();
        block.visit_expression_ids(&mut |id| new_ids.push(id));
        let mut new_places = Vec::new();
        block.visit_place_ids(&mut |id| new_places.push(id));
        if old_ids != new_ids || old_places != new_places {
            let input = replacement_input(
                expressions.view(*expression)?,
                CompletedOperation::ValueBlock(block),
            );
            *expression = promote_replacement(expressions, input)?.id();
        }
    }
    Ok(())
}

fn replace_inactive_assertion_message(
    message: &mut ExprId,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
) -> Result<(), TemplateNormalizationError> {
    let view = expressions.view(*message)?;
    if let ExpressionOperationView::Constant(value) = view.operation()
        && matches!(
            constants
                .payload(value)
                .ok_or_else(|| CompilerError::compiler_error(
                    "assertion message constant ID is outside its owner"
                ))?,
            ConstValuePayload::OptionNone
        )
    {
        return Ok(());
    }
    let pending = collect_expression_failure_facts(view, &TemplateIrStore::new(), &*expressions)?;
    let type_id = view.value_type_id()?;
    let none = constants.insert_resolved(type_id, ConstValuePayload::OptionNone, view.span())?;
    let mut classification = view.classification();
    classification.const_record_state = ConstRecordState::RuntimeValue;
    classification.contains_regular_division = false;
    let mut input = OwnedCompletedExpressionInput {
        operation: CompletedOperation::Constant(none),
        success: view.success_shape(),
        error_type: None,
        span: view.span(),
        value_mode: ValueMode::ImmutableOwned,
        classification,
        occurrence: None,
        failure: None,
    };
    for contributor in pending.implicit {
        if matches!(contributor.source, ImplicitFailureSource::PrivateCall(_)) {
            input
                .failure
                .get_or_insert_with(Default::default)
                .record_implicit(contributor);
        }
    }
    *message = promote_replacement(expressions, input)?.id();
    Ok(())
}

#[derive(Debug)]
pub(in crate::compiler_frontend::ast) enum TemplateNormalizationError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<InfrastructureErrors>),
}

impl From<CompilerDiagnostic> for TemplateNormalizationError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        TemplateNormalizationError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for TemplateNormalizationError {
    fn from(error: CompilerError) -> Self {
        TemplateNormalizationError::Infrastructure(error.into())
    }
}

impl From<Box<InfrastructureErrors>> for TemplateNormalizationError {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        Self::Infrastructure(errors)
    }
}

impl From<TemplateError> for TemplateNormalizationError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::Diagnostic(diagnostic) => {
                TemplateNormalizationError::Diagnostic(diagnostic)
            }
            TemplateError::Infrastructure(error) => {
                TemplateNormalizationError::Infrastructure(error)
            }
        }
    }
}

impl From<ExpressionConstructionFailure> for TemplateNormalizationError {
    fn from(error: ExpressionConstructionFailure) -> Self {
        match error {
            ExpressionConstructionFailure::Diagnostic(error) => error.into(),
            ExpressionConstructionFailure::Invariant(error) => error.into(),
        }
    }
}

impl From<ConstValueStoreError> for TemplateNormalizationError {
    fn from(error: ConstValueStoreError) -> Self {
        match error {
            ConstValueStoreError::Diagnostic(error) => error.into(),
            ConstValueStoreError::Infrastructure(error) => {
                TemplateNormalizationError::Infrastructure(error)
            }
        }
    }
}

/// Normalizes templates in expressions.
///
/// WHAT: normalises templates embedded in expressions by folding compile-time constants
/// and materialising runtime handoffs where needed.
///
/// WHY: Expressions can contain templates at any level of nesting, so an explicit
/// work stack visits the completed expression DAG without nesting thread frames.
fn normalize_expression_templates(
    expression: &mut ExprId,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    normalize_expression_templates_with_context(
        expression,
        context,
        HelperArtifactPolicy::RejectFinalHelperValue,
    )
}

fn normalize_expression_templates_with_context(
    expression: &mut ExprId,
    context: &mut TemplateNormalizationContext<'_>,
    helper_artifact_policy: HelperArtifactPolicy,
) -> Result<(), TemplateNormalizationError> {
    // Nested template and value-body calls use only their own suffix of the shared scratch.
    let pending_base = context.expression_pending.len();
    let root = context.expression_builder.view(*expression)?;
    match root.operation() {
        ExpressionOperationView::Constant(_) | ExpressionOperationView::Reference(_) => {
            return Ok(());
        }
        ExpressionOperationView::Copy(_) => {
            root.visit_children(|child| {
                context
                    .expression_pending
                    .push(NormalizationFrame::Visit(child.id()));
            });
            let child_count = context.expression_pending.len() - pending_base;
            if child_count == 0 {
                return Ok(());
            }
            context.expression_pending.push(NormalizationFrame::Finish {
                expression: *expression,
                child_count,
            });
            context.expression_pending[pending_base..].reverse();
        }
        _ => context
            .expression_pending
            .push(NormalizationFrame::Visit(*expression)),
    }
    while context.expression_pending.len() > pending_base {
        let frame = context
            .expression_pending
            .pop()
            .expect("active scratch suffix is non-empty");
        match frame {
            NormalizationFrame::Visit(original) => {
                let needs_normalization =
                    match context.expression_builder.view(original)?.operation() {
                        ExpressionOperationView::Constant(_)
                        | ExpressionOperationView::Reference(_) => {
                            context.expression_results.push((original, original));
                            continue;
                        }
                        ExpressionOperationView::Copy(_) => false,
                        _ => true,
                    };
                let mut normalized = original;
                if needs_normalization
                    && normalize_expression_template_node(
                        &mut normalized,
                        context,
                        helper_artifact_policy,
                    )?
                {
                    context.expression_results.push((original, normalized));
                    continue;
                }
                let pending = &mut context.expression_pending;
                let children_start = pending.len();
                context
                    .expression_builder
                    .view(original)?
                    .visit_children(|child| {
                        pending.push(NormalizationFrame::Visit(child.id()));
                    });
                let child_count = pending.len() - children_start;
                if child_count == 0 {
                    if matches!(
                        context.expression_builder.view(original)?.operation(),
                        ExpressionOperationView::RuntimeTemplate(_)
                            | ExpressionOperationView::RuntimeSlotApplication(_),
                    ) {
                        increment_ast_counter(AstCounter::RuntimeTemplateHandoffsRefreshedForHir);
                    }
                    context.expression_results.push((original, original));
                    continue;
                }
                pending.push(NormalizationFrame::Finish {
                    expression: original,
                    child_count,
                });
                pending[children_start..].reverse();
            }
            NormalizationFrame::Finish {
                expression: original,
                child_count,
            } => {
                let start = context.expression_results.len() - child_count;
                let original = context.expression_builder.own(original);
                let replacements = context
                    .expression_builder
                    .own_slice(&context.expression_results[start..]);
                let normalized = rewrite_expression_children(
                    context.expression_builder,
                    original,
                    replacements,
                )?;
                context.expression_results.truncate(start);
                if matches!(
                    context
                        .expression_builder
                        .view_owned(normalized)?
                        .operation(),
                    ExpressionOperationView::RuntimeTemplate(_)
                        | ExpressionOperationView::RuntimeSlotApplication(_),
                ) {
                    increment_ast_counter(AstCounter::RuntimeTemplateHandoffsRefreshedForHir);
                }
                context
                    .expression_results
                    .push((original.id(), normalized.id()));
            }
        }
    }
    if let Some((_, normalized)) = context.expression_results.pop() {
        *expression = normalized;
    }
    Ok(())
}

/// Normalise template and statement-bearing nodes before ordinary expression children.
fn normalize_expression_template_node(
    expression: &mut ExprId,
    context: &mut TemplateNormalizationContext<'_>,
    helper_artifact_policy: HelperArtifactPolicy,
) -> Result<bool, TemplateNormalizationError> {
    let template = match context.expression_builder.view(*expression)?.operation() {
        ExpressionOperationView::Template(template) => Some(template.clone()),
        _ => None,
    };
    if let Some(mut template) = template {
        normalize_template_for_hir(&mut template, context)?;
        let finalized = finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: context.string_table,
                expression_builder: context.expression_builder,
                const_values: context.const_values,
                template_const_loop_iteration_limit: context.template_const_loop_iteration_limit,
                template_ir_store: &context.template_ir_store,
            },
            TemplatePreparationMode::Value,
        )?;
        let replacement = match finalized {
            FinalizedTemplateValue::Folded(value, provenance) => {
                NormalizedTemplateExpression::Folded(value, provenance)
            }
            FinalizedTemplateValue::Runtime(prepared) => {
                materialize_runtime_template_handoff_for_hir(&template, context, &prepared)?
            }
            FinalizedTemplateValue::SlotInsertHelper => {
                if helper_artifact_policy == HelperArtifactPolicy::RejectFinalHelperValue {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::HelperOutsideWrapperSlot,
                        template.span,
                    )
                    .into());
                }
                return Ok(true);
            }
        };
        let mut input = context
            .expression_builder
            .view(*expression)?
            .to_owned_completed_input(context.expression_builder)?;
        input.operation = match replacement {
            NormalizedTemplateExpression::Folded(value, provenance) => {
                let type_id = context
                    .expression_builder
                    .view(*expression)?
                    .value_type_id()?;
                let value = context.const_values.insert_resolved(
                    type_id,
                    ConstValuePayload::String(value),
                    input.span,
                )?;
                let occurrence = input.occurrence.get_or_insert_with(|| OccurrenceFacts {
                    provenance: SyntheticInterfaceProvenance::default(),
                    diagnostic_spelling: None,
                });
                occurrence.provenance = occurrence.provenance.union(&provenance);
                input.value_mode = ValueMode::ImmutableOwned;
                CompletedOperation::Constant(value)
            }
            NormalizedTemplateExpression::RuntimeTemplate(mut handoff) => {
                normalize_runtime_template_handoff_for_hir(&mut handoff, context)?;
                CompletedOperation::RuntimeTemplate(handoff)
            }
            NormalizedTemplateExpression::RuntimeSlotApplication(mut handoff) => {
                normalize_runtime_slot_handoff_for_hir(&mut handoff, context)?;
                CompletedOperation::RuntimeSlotApplication(handoff)
            }
        };
        *expression = promote_replacement(context.expression_builder, input)?.id();
        return Ok(true);
    }

    let block = match context.expression_builder.view(*expression)?.operation() {
        ExpressionOperationView::ValueBlock(block) => Some(block.clone()),
        _ => None,
    };
    if let Some(mut block) = block {
        let mut original_ids = Vec::new();
        block.visit_expression_ids(&mut |id| original_ids.push(id));
        let mut original_places = Vec::new();
        block.visit_place_ids(&mut |id| original_places.push(id));
        match &mut block {
            ValueBlock::If(value) => {
                normalize_expression_templates_with_context(
                    &mut value.condition,
                    context,
                    helper_artifact_policy,
                )?;
                normalize_nodes(&mut value.then_body, context)?;
                normalize_nodes(&mut value.else_body, context)?;
            }
            ValueBlock::LexicalScope(value) => normalize_nodes(&mut value.body, context)?,
            ValueBlock::Match(value) => {
                normalize_expression_templates_with_context(
                    &mut value.scrutinee,
                    context,
                    helper_artifact_policy,
                )?;
                for arm in &mut value.arms {
                    if let Some(guard) = &mut arm.guard {
                        normalize_expression_templates_with_context(
                            guard,
                            context,
                            helper_artifact_policy,
                        )?;
                    }
                    normalize_nodes(&mut arm.body, context)?;
                }
                if let Some(body) = &mut value.default {
                    normalize_nodes(body, context)?;
                }
            }
            ValueBlock::Catch(value) => {
                normalize_expression_templates_with_context(
                    &mut value.handled_value,
                    context,
                    helper_artifact_policy,
                )?;
                normalize_fallible_handling_templates(
                    &mut value.handler,
                    context,
                    helper_artifact_policy,
                )?;
            }
        }
        let mut normalized_ids = Vec::new();
        block.visit_expression_ids(&mut |id| normalized_ids.push(id));
        let mut normalized_places = Vec::new();
        block.visit_place_ids(&mut |id| normalized_places.push(id));
        if original_ids != normalized_ids || original_places != normalized_places {
            let input = replacement_input(
                context.expression_builder.view(*expression)?,
                CompletedOperation::ValueBlock(block),
            );
            *expression = promote_replacement(context.expression_builder, input)?.id();
        }
        return Ok(true);
    }

    Ok(false)
}
enum NormalizedTemplateExpression {
    Folded(ConstStringValue, SyntheticInterfaceProvenance),
    RuntimeTemplate(OwnedRuntimeTemplateHandoff),
    RuntimeSlotApplication(OwnedRuntimeSlotApplicationHandoff),
}

/// Normalizes a template for HIR consumption.
///
/// WHAT: normalizes every expression payload reachable from the template's root
///       TIR reference, including control-flow selectors and loop headers.
///
/// WHY:
/// - Runtime templates may contain compile-time child templates after wrapper/head
///   composition. We fold those pieces now so HIR sees finalized chunks.
/// - AST may fold compile-time subtemplates inside a runtime template, but must preserve
///   the enclosing runtime template whenever any runtime chunk remains.
/// - Only escaped helper artifacts are invalid after AST composition.
/// - The enclosing expression replacement builds the owned runtime handoff from
///   the normalized template so HIR receives a neutral payload without depending
///   on AST template internals.
fn normalize_template_for_hir(
    template: &mut Template,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    // Keep normalized payloads in the shared view context consumed
    // by the finalized effective view and runtime handoff materializer. This
    // preserves shared TIR nodes while covering dynamic expressions, selectors,
    // loop headers, and every reachable control-flow body from one root pass.
    let reference = template.tir_reference;
    // Same-store is now the only path: every TIR reference is local to this
    // module store, so expression-overlay payloads are always collected. Phase
    // promotion to Finalized is gated separately below, so parsed references
    // can receive normalized overlays without becoming finalized views.
    let should_mark_finalized = reference.phase.is_at_least(TemplateTirPhase::Composed);
    let expression_payloads = collect_expression_overlay_payloads(&reference, context)?;
    if expression_payloads.is_empty() && reference.context.expression_overlay.is_none() {
        if should_mark_finalized {
            template.tir_reference.phase = TemplateTirPhase::Finalized;
        }
        return Ok(());
    }

    let mut normalized_overrides = Vec::with_capacity(expression_payloads.len());
    for (site_id, mut expression) in expression_payloads {
        normalize_expression_templates_with_context(
            &mut expression,
            context,
            HelperArtifactPolicy::AllowNestedHelperContent,
        )?;
        normalized_overrides.push((site_id, expression));
    }

    let mut store = context.template_ir_store.borrow_mut();
    template.tir_reference.context =
        replace_expression_overlay_entries(&mut store, reference.context, normalized_overrides)?;
    if should_mark_finalized {
        template.tir_reference.phase = TemplateTirPhase::Finalized;
    }

    Ok(())
}

fn collect_expression_overlay_payloads(
    reference: &TemplateTirReference,
    context: &TemplateNormalizationContext<'_>,
) -> Result<Vec<(ExpressionSiteId, ExprId)>, TemplateNormalizationError> {
    let store = context.template_ir_store.borrow();
    let view = TirView::new(&store, reference.root, reference.phase, reference.context)?;
    let expression_payloads =
        collect_effective_tir_expression_overlay_payloads(&view, context.expression_builder)?;

    Ok(expression_payloads)
}

fn normalize_runtime_slot_template_expression_for_hir(
    expression: &mut ExprId,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    normalize_expression_templates_with_context(
        expression,
        context,
        HelperArtifactPolicy::RejectFinalHelperValue,
    )
}

/// Materializes the neutral AST-to-HIR payload from one prepared runtime view.
///
/// WHAT: passes the module string and resource authorities into the TIR handoff so owned nodes
///       carry stable structural string pieces beyond donor-store lifetime.
/// WHY: runtime handoff is a carry boundary; it must preserve resources and site roots until HIR's
///      owning output policy decides how to render or reject them.
fn materialize_runtime_template_handoff_for_hir(
    template: &Template,
    context: &mut TemplateNormalizationContext<'_>,
    prepared: &TemplatePreparation,
) -> Result<NormalizedTemplateExpression, TemplateNormalizationError> {
    let store_handle = Rc::clone(&context.template_ir_store);
    let store = store_handle.borrow();
    let view = finalized_tir_view_for_template(template, &store)?;
    let module_resources = context
        .module_resources
        .as_ref()
        .map(|resources| resources.borrow());

    if matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Runtime(RuntimeTemplateReason::SlotContribution,)
    ) {
        return Err(CompilerDiagnostic::invalid_template_slot(
            InvalidTemplateSlotReason::InsertOutsideParentSlot,
            None,
            template.span,
        )
        .into());
    }

    if let Some(handoff) = owned_runtime_slot_handoff_for_prepared_view(
        prepared,
        view.clone(),
        context.expression_builder,
        context.const_values,
        context.string_table,
        module_resources.as_deref(),
    )? {
        increment_ast_counter(AstCounter::RuntimeTemplateHandoffsMaterialized);
        return Ok(NormalizedTemplateExpression::RuntimeSlotApplication(
            handoff,
        ));
    }

    let handoff = owned_runtime_template_handoff_for_prepared_view(
        prepared,
        view,
        context.expression_builder,
        context.const_values,
        context.string_table,
        module_resources.as_deref(),
    )?;

    increment_ast_counter(AstCounter::RuntimeTemplateHandoffsMaterialized);
    Ok(NormalizedTemplateExpression::RuntimeTemplate(handoff))
}

fn normalize_runtime_slot_handoff_for_hir(
    handoff: &mut OwnedRuntimeSlotApplicationHandoff,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    let mut result = Ok(());
    handoff.visit_expression_ids_mut(&mut |expression| {
        if result.is_ok() {
            result = normalize_runtime_slot_template_expression_for_hir(expression, context);
        }
    });
    result
}

fn normalize_runtime_template_handoff_for_hir(
    handoff: &mut OwnedRuntimeTemplateHandoff,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    let mut result = Ok(());
    handoff.visit_expression_ids_mut(&mut |expression| {
        if result.is_ok() {
            result = normalize_runtime_slot_template_expression_for_hir(expression, context);
        }
    });
    result
}
#[cfg(test)]
#[path = "tests/normalize_ast_tests.rs"]
mod normalize_ast_tests;

#[cfg(test)]
#[path = "tests/normalize_ast_stack_tests.rs"]
mod stack_tests;
