//! AST node template normalization for HIR preparation.
//!
//! WHAT: Recursively traverses AST nodes to normalize embedded templates by
//! folding compile-time constants, materializing runtime handoffs, and
//! completing template metadata. Mutates AST nodes in place to prepare them for
//! HIR.
//!
//! WHY: HIR assumes templates are semantically complete with folded constants,
//! no escaped helper artifacts, and owned runtime handoff shapes for runtime
//! templates. This normalization satisfies that AST→HIR boundary contract
//! before lowering.
//!
//! ## Normalization Strategy
//!
//! 1. **Constant Folding**: Templates with `RenderableString` const value kinds
//!    are folded into compact `StringSlice` or structural-string expressions.
//!
//! 2. **Runtime Handoff Construction**: Runtime templates receive owned runtime
//!    handoffs so HIR does not need to reconstruct template structure.
//!
//! 3. **Metadata Completion**: All templates have their kind refreshed from
//!    their final effective TIR view.
//!
//! 4. **Helper Rejection**: escaped `$insert(...)` helper templates are rejected
//!    if they reach finalization outside immediate wrapper-slot composition.
//!
//! ## AST→HIR Template Boundary
//!
//! AST owns:
//! - Template foldability decisions
//! - Constant template lowering
//! - Runtime template handoff materialization
//!
//! HIR receives:
//! - Folded constant templates as `StringSlice` or structural-string expressions
//! - Runtime templates with owned runtime handoffs
//! - No escaped helper artifacts (`TemplateType::SlotInsert`)
//! - No templates requiring formatting

use super::finalizer::AstFinalizer;
use super::template_helpers::{
    FinalizedTemplateValue, TemplateValueFinalizationInputs, finalize_template_value,
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, LoopBindings, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    assert_message_escape_diagnostic, assertion_condition_is_statically_true,
};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::eval_expression::pending_expression_item_bug;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::failure_classification::collect_expression_failure_facts;
use crate::compiler_frontend::ast::expressions::failure_facts::ImplicitFailureSource;
use crate::compiler_frontend::ast::module_ast::environment::ResolvedPublicTypeRootKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::runtime_handoff;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, RuntimeTemplateReason, TemplateHelperKind, TemplateIrStore,
    TemplatePreparation, TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase,
    TemplateTirReference, TirView, collect_effective_tir_expression_overlay_payloads,
    finalized_tir_view_for_template, owned_runtime_slot_handoff_for_prepared_view,
    owned_runtime_template_handoff_for_prepared_view, replace_expression_overlay_entries,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateSlotReason, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::profile::NumericProfile;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;

struct TemplateNormalizationContext<'strings> {
    template_const_loop_iteration_limit: usize,
    numeric_profile: NumericProfile,
    string_table: &'strings mut StringTable,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
    module_resources:
        Option<Rc<RefCell<crate::compiler_frontend::paths::module_resources::ModuleResourceTable>>>,
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
        &self,
        ast: &mut [AstNode],
        string_table: &mut StringTable,
    ) -> Result<(), TemplateNormalizationError> {
        for node in ast {
            let mut normalization_context = TemplateNormalizationContext {
                template_const_loop_iteration_limit: self
                    .context
                    .template_const_loop_iteration_limit,
                numeric_profile: self.context.numeric_profile,
                string_table,
                template_ir_store: Rc::clone(&self.context.template_ir_store),
                module_resources: self
                    .context
                    .file_value_resolution
                    .as_ref()
                    .map(|services| Rc::clone(&services.module_resources)),
            };
            normalize_ast_node_templates(node, &mut normalization_context)?;
        }

        Ok(())
    }

    /// Synchronize normalized emitted declaration defaults into the retained public root
    /// table, and normalize retained-only generic defaults.
    ///
    /// WHAT: after the emitted AST is normalized once (function signature parameter defaults
    /// and struct field defaults folded alongside function bodies), copy the exact normalized
    /// signatures and fields into the retained public root table so the public-interface draft
    /// reads one normalized copy. Generic free functions, generic structs and generic receiver
    /// methods have no emitted declaration node, so their retained defaults are normalized in
    /// place through the same [`normalize_expression_templates`] helper: the generic template's
    /// retained source file for generic functions, and the canonical source metadata for
    /// generic structs.
    ///
    /// WHY: folding the emitted copy and retained copy independently would create a second
    /// normalization interpretation. Synchronizing from the single emitted fold guarantees one
    /// owner. Generic declarations without an emitted node still need their defaults normalized
    /// so the draft receives no live TIR references. Root receiver entries are synchronized
    /// directly from the emitted fold and the retained generic templates; the receiver catalog
    /// itself is the emission-time lookup side-table and is not a post-finalization default
    /// owner, so it stays unrewritten. Generic receiver entries that join no root entry have no
    /// other normalization owner after that, so their defaults are validated through a
    /// discarded temporary signature copy to keep helper, type and TIR diagnostics observable
    /// even when the method is never called.
    pub(super) fn synchronize_normalized_public_defaults(
        &mut self,
        emitted_ast: &[AstNode],
        string_table: &mut StringTable,
    ) -> Result<(), TemplateNormalizationError> {
        let EmittedDeclarationDefaults {
            function_signatures_by_path: normalized_function_signatures_by_path,
            struct_fields_by_path: normalized_struct_fields_by_path,
        } = collect_emitted_declaration_defaults(emitted_ast)?;

        let generic_function_templates_by_path =
            &self.environment.lookups.generic_function_templates_by_path;
        let type_environment = &self.environment.type_environment;
        let template_const_loop_iteration_limit = self.context.template_const_loop_iteration_limit;
        let numeric_profile = self.context.numeric_profile;
        let template_ir_store = Rc::clone(&self.context.template_ir_store);

        // Synchronize root table function and struct defaults. Branch on authoritative generic
        // metadata before choosing a source: a non-generic declaration must have exactly one
        // emitted node; a generic declaration must have no emitted node and its retained defaults
        // are normalized in place.
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
                        normalize_retained_signature_defaults(
                            signature,
                            template_const_loop_iteration_limit,
                            numeric_profile,
                            &template_ir_store,
                            string_table,
                        )?;
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
                        normalize_retained_field_defaults(
                            fields,
                            template_const_loop_iteration_limit,
                            numeric_profile,
                            &template_ir_store,
                            string_table,
                        )?;
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

        // Synchronize receiver-method root entries directly from the once-normalized emitted
        // copy, preserving vector order. Root receiver entries are selected from this module's
        // parsed headers, so a non-generic entry must join exactly one emitted signature and a
        // generic entry must have none: its retained defaults normalize in place. Imported
        // receiver contracts arrive from an already-completed provider interface with their
        // defaults folded by the provider's normalization owner, so they keep the signature the
        // root-table build cloned from the catalog.
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
                normalize_retained_signature_defaults(
                    &mut root_entry.signature,
                    template_const_loop_iteration_limit,
                    numeric_profile,
                    &template_ir_store,
                    string_table,
                )?;
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

        // Uncalled private generic methods have neither an emitted node nor a public root.
        // Validate their defaults without retaining another normalized signature or rewriting
        // the emission-time catalog.
        for catalog_entry in self
            .environment
            .lookups
            .receiver_methods
            .by_function_path
            .values()
        {
            // A root receiver entry has exactly one normalized owner above; normalizing its
            // catalog copy again would be a second normalization interpretation.
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

            // Ordinary receiver methods normalize defaults through their emitted declaration node.
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

            let mut diagnostic_only_signature = catalog_entry.signature.clone();
            normalize_retained_signature_defaults(
                &mut diagnostic_only_signature,
                template_const_loop_iteration_limit,
                numeric_profile,
                &template_ir_store,
                string_table,
            )?;
        }

        self.environment.resolved_public_type_roots = root_table;

        Ok(())
    }
}

/// Normalize retained-only function signature parameter defaults through the existing helper.
///
/// WHAT: creates a [`TemplateNormalizationContext`] and folds each parameter default in place.
/// Parameters with `NoValue` (no default) are no-ops.
fn normalize_retained_signature_defaults(
    signature: &mut FunctionSignature,
    template_const_loop_iteration_limit: usize,
    numeric_profile: NumericProfile,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<(), TemplateNormalizationError> {
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit,
        numeric_profile,
        string_table,
        template_ir_store: Rc::clone(template_ir_store),
        module_resources: None,
    };

    for parameter in &mut signature.parameters {
        normalize_expression_templates(&mut parameter.value, &mut context)?;
    }

    Ok(())
}

/// Normalize retained-only struct field defaults through the existing helper.
///
/// WHAT: creates a [`TemplateNormalizationContext`] and folds each field default in place.
/// Fields with `NoValue` (no default) are no-ops.
fn normalize_retained_field_defaults(
    fields: &mut [Declaration],
    template_const_loop_iteration_limit: usize,
    numeric_profile: NumericProfile,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<(), TemplateNormalizationError> {
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit,
        numeric_profile,
        string_table,
        template_ir_store: Rc::clone(template_ir_store),
        module_resources: None,
    };

    for field in fields {
        normalize_expression_templates_with_context(
            &mut field.value,
            &mut context,
            HelperArtifactPolicy::AllowNestedHelperContent,
        )?;
    }

    Ok(())
}

/// Borrowed normalized declaration facts indexed for the public-root synchronization join.
///
/// The emitted AST remains the owner; only the final retained roots copy signatures and fields.
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

        NodeKind::Function(_, signature, body) => {
            for parameter in &mut signature.parameters {
                normalize_expression_templates_with_context(
                    &mut parameter.value,
                    context,
                    HelperArtifactPolicy::AllowNestedHelperContent,
                )?;
            }
            normalize_nodes(body, context)
        }

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
                if let Some(diagnostic) = assert_message_escape_diagnostic(message, &store)? {
                    return Err(diagnostic.into());
                }
            }

            // The message still needs the ordinary AST normalization path so its nested
            // templates reach the finalized boundary and retain their frontend diagnostics.
            normalize_expression_templates(message, context)?;

            let store = context.template_ir_store.borrow();
            if let Some(diagnostic) = assert_message_escape_diagnostic(message, &store)? {
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
                match &mut arm.pattern {
                    MatchPattern::Literal(expression)
                    | MatchPattern::OptionValue {
                        value: expression, ..
                    }
                    | MatchPattern::Relational {
                        value: expression, ..
                    } => normalize_expression_templates(expression, context)?,
                    MatchPattern::OptionNone { .. }
                    | MatchPattern::ChoiceVariant { .. }
                    | MatchPattern::OptionPresentCapture { .. } => {}
                }

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
/// WHAT: Handles normalization for variable declarations, assignments, and
/// struct definitions by recursively normalizing value expressions and fields.
///
/// WHY: Declaration nodes have similar structure (identifier + value) and can
/// be handled together to avoid duplication.
fn normalize_declaration_templates(
    node: &mut AstNode,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    match &mut node.kind {
        NodeKind::VariableDeclaration(declaration) => {
            normalize_expression_templates(&mut declaration.value, context)
        }

        NodeKind::Assignment { value, .. } => normalize_expression_templates(value, context),

        NodeKind::StructDefinition(_, fields) => {
            for field in fields {
                normalize_expression_templates_with_context(
                    &mut field.value,
                    context,
                    HelperArtifactPolicy::AllowNestedHelperContent,
                )?;
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
    expressions: &mut [Expression],
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
    if let Some(item_binding) = &mut bindings.item {
        normalize_expression_templates(&mut item_binding.value, context)?;
    }

    if let Some(index_binding) = &mut bindings.index {
        normalize_expression_templates(&mut index_binding.value, context)?;
    }

    Ok(())
}

fn normalize_call_argument_values(
    arguments: &mut [CallArgument],
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    for argument in arguments {
        normalize_expression_templates(&mut argument.value, context)?;
    }

    Ok(())
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
) -> Result<(), CompilerError> {
    for node in ast {
        discard_inactive_assertion_messages_in_node(node)?;
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_node(node: &mut AstNode) -> Result<(), CompilerError> {
    match &mut node.kind {
        NodeKind::Function(_, signature, body) => {
            discard_inactive_assertion_messages_in_signature(signature)?;
            discard_inactive_assertion_messages(body)?;
        }

        NodeKind::VariableDeclaration(declaration) => {
            discard_inactive_assertion_messages_in_expression(&mut declaration.value)?;
        }

        NodeKind::Return(values) => {
            for value in values {
                discard_inactive_assertion_messages_in_expression(value)?;
            }
        }

        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value) => {
            discard_inactive_assertion_messages_in_expression(value)?;
        }

        NodeKind::ThenValue(produced_values) => {
            for expression in &mut produced_values.expressions {
                discard_inactive_assertion_messages_in_expression(expression)?;
            }
        }

        NodeKind::If(condition, then_body, else_body, _) => {
            discard_inactive_assertion_messages_in_expression(condition)?;
            discard_inactive_assertion_messages(then_body)?;
            if let Some(else_body) = else_body {
                discard_inactive_assertion_messages(else_body)?;
            }
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            discard_inactive_assertion_messages_in_expression(scrutinee)?;
            for arm in arms {
                discard_inactive_assertion_messages_in_match_pattern(&mut arm.pattern)?;
                if let Some(guard) = &mut arm.guard {
                    discard_inactive_assertion_messages_in_expression(guard)?;
                }
                discard_inactive_assertion_messages(&mut arm.body)?;
            }
            if let Some(default) = default {
                discard_inactive_assertion_messages(default)?;
            }
        }

        NodeKind::LexicalScope { body } => discard_inactive_assertion_messages(body)?,

        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            discard_inactive_assertion_messages_in_loop_bindings(bindings)?;
            discard_inactive_assertion_messages_in_expression(&mut range.start)?;
            discard_inactive_assertion_messages_in_expression(&mut range.end)?;
            if let Some(step) = &mut range.step {
                discard_inactive_assertion_messages_in_expression(step)?;
            }
            discard_inactive_assertion_messages(body)?;
        }

        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            discard_inactive_assertion_messages_in_loop_bindings(bindings)?;
            discard_inactive_assertion_messages_in_expression(iterable)?;
            discard_inactive_assertion_messages(body)?;
        }

        NodeKind::WhileLoop(condition, body) => {
            discard_inactive_assertion_messages_in_expression(condition)?;
            discard_inactive_assertion_messages(body)?;
        }

        NodeKind::Assert { condition, message } => {
            discard_inactive_assertion_messages_in_expression(condition)?;
            if assertion_condition_is_statically_true(condition) {
                replace_inactive_assertion_message(message)?;
            } else {
                discard_inactive_assertion_messages_in_expression(message)?;
            }
        }

        NodeKind::StructDefinition(_, fields) => {
            for field in fields {
                discard_inactive_assertion_messages_in_expression(&mut field.value)?;
            }
        }

        NodeKind::Assignment { value, .. } | NodeKind::MultiBind { value, .. } => {
            discard_inactive_assertion_messages_in_expression(value)?;
        }

        NodeKind::Break | NodeKind::Continue => {}
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_signature(
    signature: &mut FunctionSignature,
) -> Result<(), CompilerError> {
    for parameter in &mut signature.parameters {
        discard_inactive_assertion_messages_in_expression(&mut parameter.value)?;
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_loop_bindings(
    bindings: &mut LoopBindings,
) -> Result<(), CompilerError> {
    if let Some(item) = &mut bindings.item {
        discard_inactive_assertion_messages_in_expression(&mut item.value)?;
    }
    if let Some(index) = &mut bindings.index {
        discard_inactive_assertion_messages_in_expression(&mut index.value)?;
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_match_pattern(
    pattern: &mut MatchPattern,
) -> Result<(), CompilerError> {
    match pattern {
        MatchPattern::Literal(expression)
        | MatchPattern::OptionValue {
            value: expression, ..
        }
        | MatchPattern::Relational {
            value: expression, ..
        } => discard_inactive_assertion_messages_in_expression(expression)?,

        MatchPattern::ChoiceVariant { .. }
        | MatchPattern::OptionNone { .. }
        | MatchPattern::OptionPresentCapture { .. } => {}
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_expression(
    expression: &mut Expression,
) -> Result<(), CompilerError> {
    match &mut expression.kind {
        ExpressionKind::Runtime(rpn) => {
            for item in &mut rpn.items {
                match item {
                    ExpressionRpnItem::Operand(expression) => {
                        discard_inactive_assertion_messages_in_expression(expression)?;
                    }
                    ExpressionRpnItem::Operator { .. } => {}
                    // Pending syntax never survives evaluation; this fallible elision reports the
                    // broken invariant instead of silently keeping the malformed operand.
                    ExpressionRpnItem::PendingNumericLiteral { .. }
                    | ExpressionRpnItem::PendingGroup { .. } => {
                        return Err(pending_expression_item_bug("assertion-message elision"));
                    }
                }
            }
        }

        ExpressionKind::Copy(_) => {}

        ExpressionKind::FieldAccess { base, .. } => {
            discard_inactive_assertion_messages_in_expression(base)?;
        }

        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            discard_inactive_assertion_messages_in_expression(receiver)?;
            discard_inactive_assertion_messages_in_call_arguments(args)?;
        }

        ExpressionKind::Function(signature) => {
            discard_inactive_assertion_messages_in_signature(signature)?;
        }

        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
            discard_inactive_assertion_messages_in_call_arguments(args)?;
        }

        ExpressionKind::Collection(items) => {
            for item in items {
                discard_inactive_assertion_messages_in_expression(item)?;
            }
        }

        ExpressionKind::StructInstance(fields)
        | ExpressionKind::StructDefinition(fields)
        | ExpressionKind::AnonymousConstRecord { fields }
        | ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                discard_inactive_assertion_messages_in_expression(&mut field.value)?;
            }
        }

        ExpressionKind::Range(start, end) => {
            discard_inactive_assertion_messages_in_expression(start)?;
            discard_inactive_assertion_messages_in_expression(end)?;
        }

        ExpressionKind::ValueBlock { block } => match block.as_mut() {
            ValueBlock::If(value_if) => {
                discard_inactive_assertion_messages_in_expression(&mut value_if.condition)?;
                discard_inactive_assertion_messages(&mut value_if.then_body)?;
                discard_inactive_assertion_messages(&mut value_if.else_body)?;
            }

            ValueBlock::LexicalScope(value_lexical_scope) => {
                discard_inactive_assertion_messages(&mut value_lexical_scope.body)?;
            }

            ValueBlock::Match(value_match) => {
                discard_inactive_assertion_messages_in_expression(&mut value_match.scrutinee)?;
                for arm in &mut value_match.arms {
                    discard_inactive_assertion_messages_in_match_pattern(&mut arm.pattern)?;
                    if let Some(guard) = &mut arm.guard {
                        discard_inactive_assertion_messages_in_expression(guard)?;
                    }
                    discard_inactive_assertion_messages(&mut arm.body)?;
                }
                if let Some(default) = &mut value_match.default {
                    discard_inactive_assertion_messages(default)?;
                }
            }

            ValueBlock::Catch(value_catch) => {
                discard_inactive_assertion_messages_in_expression(&mut value_catch.handled_value)?;
                discard_inactive_assertion_messages_in_fallible_handling(&mut value_catch.handler)?;
            }
        },

        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                discard_inactive_assertion_messages_in_expression(&mut entry.key)?;
                discard_inactive_assertion_messages_in_expression(&mut entry.value)?;
            }
        }

        ExpressionKind::OptionPropagation { value }
        | ExpressionKind::Coerced { value, .. }
        | ExpressionKind::HandledFallibleExpression { value, .. } => {
            discard_inactive_assertion_messages_in_expression(value)?;
        }

        ExpressionKind::Cast(cast) => {
            discard_inactive_assertion_messages_in_expression(&mut cast.source)?;
        }

        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            discard_inactive_assertion_messages_in_expression(value)?;
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            discard_inactive_assertion_messages_in_runtime_template_handoff(handoff)?;
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            discard_inactive_assertion_messages_in_runtime_slot_handoff(handoff)?;
        }

        ExpressionKind::Template(_)
        | ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Uint(_)
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::FixedScalar(_)
        | ExpressionKind::Number(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::StructuralString { .. }
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_) => {}
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_call_arguments(
    arguments: &mut [CallArgument],
) -> Result<(), CompilerError> {
    for argument in arguments {
        discard_inactive_assertion_messages_in_expression(&mut argument.value)?;
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_fallible_handling(
    handling: &mut FallibleHandling,
) -> Result<(), CompilerError> {
    if let FallibleHandling::Handler { body, .. } = handling {
        discard_inactive_assertion_messages(body)?;
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_runtime_template_handoff(
    handoff: &mut OwnedRuntimeTemplateHandoff,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_template_handoff_mut(
        handoff,
        &mut discard_inactive_assertion_messages_in_owned_runtime_node,
    )
}

fn discard_inactive_assertion_messages_in_runtime_slot_handoff(
    handoff: &mut OwnedRuntimeSlotApplicationHandoff,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_slot_application_handoff_mut(
        handoff,
        &mut discard_inactive_assertion_messages_in_owned_runtime_node,
    )
}

fn discard_inactive_assertion_messages_in_owned_runtime_node(
    node: &mut OwnedRuntimeTemplateNode,
) -> Result<(), CompilerError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            discard_inactive_assertion_messages_in_expression(expression)?;
        }

        OwnedRuntimeTemplateNode::BranchChain { branches, .. } => {
            for branch in branches {
                discard_inactive_assertion_messages_in_branch_selector(&mut branch.selector)?;
            }
        }

        OwnedRuntimeTemplateNode::Loop { header, .. } => {
            discard_inactive_assertion_messages_in_loop_header(header)?;
        }

        OwnedRuntimeTemplateNode::Sequence { .. }
        | OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::ChildTemplate { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_branch_selector(
    selector: &mut TemplateBranchSelector,
) -> Result<(), CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            discard_inactive_assertion_messages_in_expression(condition)?;
        }
        TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
            discard_inactive_assertion_messages_in_expression(scrutinee)?;
            discard_inactive_assertion_messages_in_match_pattern(pattern)?;
        }
    }
    Ok(())
}

fn discard_inactive_assertion_messages_in_loop_header(
    header: &mut TemplateLoopHeader,
) -> Result<(), CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            discard_inactive_assertion_messages_in_expression(condition)?;
        }
        TemplateLoopHeader::Range { bindings, range } => {
            discard_inactive_assertion_messages_in_loop_bindings(bindings)?;
            discard_inactive_assertion_messages_in_expression(&mut range.start)?;
            discard_inactive_assertion_messages_in_expression(&mut range.end)?;
            if let Some(step) = &mut range.step {
                discard_inactive_assertion_messages_in_expression(step)?;
            }
        }
        TemplateLoopHeader::Collection { bindings, iterable } => {
            discard_inactive_assertion_messages_in_loop_bindings(bindings)?;
            discard_inactive_assertion_messages_in_expression(iterable)?;
        }
    }
    Ok(())
}

/// Replaces a fully validated inactive assertion message with the canonical typed `none` shape.
///
/// WHAT: removes the normalized executable message, including any owned runtime handoff,
///       synthetic provenance, and TIR-backed expression identity.
/// WHY: a compile-time-true assertion remains frontend-valid but publishes no message value or
///      downstream executable fact. The existing resolved optional type identity is retained.
fn replace_inactive_assertion_message(message: &mut Expression) -> Result<(), CompilerError> {
    // This late pass only sees normalized, owned runtime handoffs, never TIR references.
    let pending = collect_expression_failure_facts(message, &TemplateIrStore::new())?;
    let mut inert_message = Expression::new(
        ExpressionKind::OptionNone,
        message.span,
        message.type_id,
        message.diagnostic_type.clone(),
        ValueMode::ImmutableOwned,
    );
    // Inactive execution publishes no message value, but private calls anywhere in the removed
    // subtree still need their origin witnesses until exact semantic summary convergence.
    // The inert node replaces the entire subtree, so it becomes the sole durable witness owner.
    for contributor in pending.implicit {
        if matches!(contributor.source, ImplicitFailureSource::PrivateCall(_)) {
            inert_message.failure_facts.record_implicit(contributor);
        }
    }
    *message = inert_message;
    Ok(())
}

#[derive(Debug)]
pub(super) enum TemplateNormalizationError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for TemplateNormalizationError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        TemplateNormalizationError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for TemplateNormalizationError {
    fn from(error: CompilerError) -> Self {
        TemplateNormalizationError::Infrastructure(Box::new(error))
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

/// Normalizes templates in expressions.
///
/// WHAT: Recursively normalizes templates embedded in expressions by folding
/// compile-time constants and materializing runtime handoffs where needed.
///
/// WHY: Expressions can contain templates at any level of nesting, so we need
/// to recursively traverse the expression tree to normalize all templates.
fn normalize_expression_templates(
    expression: &mut Expression,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    normalize_expression_templates_with_context(
        expression,
        context,
        HelperArtifactPolicy::RejectFinalHelperValue,
    )?;

    Ok(())
}

fn normalize_place_expression_templates(
    place: &mut PlaceExpression,
) -> Result<(), TemplateNormalizationError> {
    match &mut place.kind {
        PlaceExpressionKind::Local(_) => Ok(()),
        PlaceExpressionKind::Field { base, .. } => normalize_place_expression_templates(base),
    }
}

fn normalize_expression_templates_with_context(
    expression: &mut Expression,
    context: &mut TemplateNormalizationContext<'_>,
    helper_artifact_policy: HelperArtifactPolicy,
) -> Result<(), TemplateNormalizationError> {
    let template_replacement = match &mut expression.kind {
        ExpressionKind::Copy(place) => {
            normalize_place_expression_templates(place)?;
            None
        }

        ExpressionKind::Runtime(rpn) => {
            for item in &mut rpn.items {
                match item {
                    ExpressionRpnItem::Operand(expression) => {
                        normalize_expression_templates_with_context(
                            expression,
                            context,
                            helper_artifact_policy,
                        )?;
                    }
                    ExpressionRpnItem::Operator { .. } => {}
                    // Pending syntax never survives evaluation; its presence here is a broken
                    // compiler invariant, not a normalizable template position.
                    ExpressionRpnItem::PendingNumericLiteral { .. }
                    | ExpressionRpnItem::PendingGroup { .. } => {
                        return Err(pending_expression_item_bug("template normalization").into());
                    }
                }
            }
            None
        }

        ExpressionKind::FieldAccess { base, .. } => {
            normalize_expression_templates_with_context(base, context, helper_artifact_policy)?;
            None
        }

        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            normalize_expression_templates_with_context(receiver, context, helper_artifact_policy)?;
            normalize_call_argument_values(args, context)?;
            None
        }

        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. } => {
            for argument in args {
                normalize_expression_templates_with_context(
                    &mut argument.value,
                    context,
                    helper_artifact_policy,
                )?;
            }
            None
        }

        ExpressionKind::HandledFallibleHostFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { args, .. } => {
            for argument in args {
                normalize_expression_templates_with_context(
                    &mut argument.value,
                    context,
                    helper_artifact_policy,
                )?;
            }
            None
        }

        ExpressionKind::Collection(args) => {
            for argument in args {
                normalize_expression_templates_with_context(
                    argument,
                    context,
                    helper_artifact_policy,
                )?;
            }
            None
        }

        ExpressionKind::Cast(cast) => {
            normalize_expression_templates_with_context(
                &mut cast.source,
                context,
                helper_artifact_policy,
            )?;
            None
        }

        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            normalize_expression_templates_with_context(value, context, helper_artifact_policy)?;
            None
        }

        ExpressionKind::OptionPropagation { value } | ExpressionKind::Coerced { value, .. } => {
            normalize_expression_templates_with_context(value, context, helper_artifact_policy)?;
            None
        }

        ExpressionKind::HandledFallibleExpression { value, .. } => {
            normalize_expression_templates_with_context(value, context, helper_artifact_policy)?;
            None
        }

        ExpressionKind::Template(template) => {
            normalize_template_for_hir(template, context)?;

            let finalization = finalize_template_value(
                template,
                TemplateValueFinalizationInputs {
                    string_table: context.string_table,
                    template_const_loop_iteration_limit: context
                        .template_const_loop_iteration_limit,
                    numeric_profile: context.numeric_profile,
                    template_ir_store: &context.template_ir_store,
                },
                TemplatePreparationMode::Value,
            )?;

            match finalization {
                FinalizedTemplateValue::Folded(folded, provenance) => {
                    Some(NormalizedTemplateExpression::Folded(folded, provenance))
                }

                FinalizedTemplateValue::Runtime(prepared) => {
                    materialize_runtime_template_handoff_for_hir(template, context, &prepared)?
                }

                FinalizedTemplateValue::Helper(kind) => {
                    if helper_artifact_policy == HelperArtifactPolicy::RejectFinalHelperValue
                        && matches!(kind, TemplateHelperKind::SlotInsert)
                    {
                        return Err(CompilerDiagnostic::invalid_template_structure(
                            InvalidTemplateStructureReason::HelperOutsideWrapperSlot,
                            template.span,
                        )
                        .into());
                    }

                    None
                }
            }
        }

        ExpressionKind::StructDefinition(arguments)
        | ExpressionKind::StructInstance(arguments)
        | ExpressionKind::AnonymousConstRecord { fields: arguments } => {
            for argument in arguments {
                normalize_expression_templates_with_context(
                    &mut argument.value,
                    context,
                    helper_artifact_policy,
                )?;
            }
            None
        }

        ExpressionKind::Range(lower, upper) => {
            normalize_expression_templates_with_context(lower, context, helper_artifact_policy)?;
            normalize_expression_templates_with_context(upper, context, helper_artifact_policy)?;
            None
        }

        ExpressionKind::ValueBlock { block } => {
            match block.as_mut() {
                ValueBlock::If(value_if) => {
                    normalize_expression_templates_with_context(
                        &mut value_if.condition,
                        context,
                        helper_artifact_policy,
                    )?;
                    normalize_nodes(&mut value_if.then_body, context)?;
                    normalize_nodes(&mut value_if.else_body, context)?;
                }
                ValueBlock::LexicalScope(value_lexical_scope) => {
                    normalize_nodes(&mut value_lexical_scope.body, context)?;
                }
                ValueBlock::Match(value_match) => {
                    normalize_expression_templates_with_context(
                        &mut value_match.scrutinee,
                        context,
                        helper_artifact_policy,
                    )?;
                    for arm in &mut value_match.arms {
                        if let Some(guard) = &mut arm.guard {
                            normalize_expression_templates_with_context(
                                guard,
                                context,
                                helper_artifact_policy,
                            )?;
                        }
                        normalize_nodes(&mut arm.body, context)?;
                    }
                    if let Some(default_body) = &mut value_match.default {
                        normalize_nodes(default_body, context)?;
                    }
                }
                ValueBlock::Catch(value_catch) => {
                    normalize_expression_templates_with_context(
                        &mut value_catch.handled_value,
                        context,
                        helper_artifact_policy,
                    )?;
                    normalize_fallible_handling_templates(
                        &mut value_catch.handler,
                        context,
                        helper_artifact_policy,
                    )?;
                }
            }
            None
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            normalize_runtime_template_handoff_for_hir(handoff, context)?;
            increment_ast_counter(AstCounter::RuntimeTemplateHandoffsRefreshedForHir);
            None
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            normalize_runtime_slot_handoff_for_hir(handoff, context)?;
            increment_ast_counter(AstCounter::RuntimeTemplateHandoffsRefreshedForHir);
            None
        }

        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Uint(_)
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::FixedScalar(_)
        | ExpressionKind::Number(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::StructuralString { .. }
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Function(_)
        | ExpressionKind::Reference(_) => None,

        ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                normalize_expression_templates(&mut field.value, context)?;
            }
            None
        }
        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                normalize_expression_templates_with_context(
                    &mut entry.key,
                    context,
                    helper_artifact_policy,
                )?;
                normalize_expression_templates_with_context(
                    &mut entry.value,
                    context,
                    helper_artifact_policy,
                )?;
            }
            None
        }
    };

    match template_replacement {
        Some(NormalizedTemplateExpression::Folded(folded_template, fold_provenance)) => {
            let outer_provenance = expression.synthetic_interface_provenance.clone();
            expression.kind = match folded_template {
                ConstStringValue::Text(value) => ExpressionKind::StringSlice(value),
                ConstStringValue::Pieces(pieces) => ExpressionKind::StructuralString { pieces },
            };
            expression.diagnostic_type = DataType::StringSlice;
            expression.value_mode = ValueMode::ImmutableOwned;
            expression.synthetic_interface_provenance = outer_provenance.union(&fold_provenance);
        }

        Some(NormalizedTemplateExpression::RuntimeSlotApplication(handoff)) => {
            let value_mode = expression.value_mode.clone();
            let synthetic_interface_provenance = expression.synthetic_interface_provenance.clone();
            *expression = Expression::runtime_slot_application_handoff(handoff, value_mode);
            expression.synthetic_interface_provenance = synthetic_interface_provenance;
        }

        Some(NormalizedTemplateExpression::RuntimeTemplate(handoff)) => {
            let value_mode = expression.value_mode.clone();
            let synthetic_interface_provenance = expression.synthetic_interface_provenance.clone();
            *expression = Expression::runtime_template_handoff(handoff, value_mode);
            expression.synthetic_interface_provenance = synthetic_interface_provenance;
        }

        None => {}
    }

    Ok(())
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
    normalize_expression_overlays_for_template_reference(template, context)?;

    Ok(())
}

fn normalize_expression_overlays_for_template_reference(
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
        normalized_overrides.push((site_id, Box::new(expression)));
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
) -> Result<Vec<(ExpressionSiteId, Expression)>, TemplateNormalizationError> {
    let store = context.template_ir_store.borrow();
    let view = TirView::new(&store, reference.root, reference.phase, reference.context)?;
    let expression_payloads = collect_effective_tir_expression_overlay_payloads(&view)?;

    Ok(expression_payloads)
}

fn normalize_runtime_slot_template_expression_for_hir(
    expression: &mut Expression,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    normalize_expression_templates_with_context(
        expression,
        context,
        HelperArtifactPolicy::AllowNestedHelperContent,
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
) -> Result<Option<NormalizedTemplateExpression>, TemplateNormalizationError> {
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
        context.string_table,
        module_resources.as_deref(),
    )? {
        increment_ast_counter(AstCounter::RuntimeTemplateHandoffsMaterialized);
        return Ok(Some(NormalizedTemplateExpression::RuntimeSlotApplication(
            handoff,
        )));
    }

    let handoff = owned_runtime_template_handoff_for_prepared_view(
        prepared,
        view,
        context.string_table,
        module_resources.as_deref(),
    )?;

    increment_ast_counter(AstCounter::RuntimeTemplateHandoffsMaterialized);
    Ok(Some(NormalizedTemplateExpression::RuntimeTemplate(handoff)))
}

fn normalize_runtime_slot_handoff_for_hir(
    handoff: &mut OwnedRuntimeSlotApplicationHandoff,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    runtime_handoff::walk_owned_runtime_slot_application_handoff_mut(handoff, &mut |event| {
        normalize_owned_runtime_template_node_for_hir(event, context)
    })
}

fn normalize_runtime_template_handoff_for_hir(
    handoff: &mut OwnedRuntimeTemplateHandoff,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    runtime_handoff::walk_owned_runtime_template_handoff_mut(handoff, &mut |event| {
        normalize_owned_runtime_template_node_for_hir(event, context)
    })
}

fn normalize_owned_runtime_template_node_for_hir(
    node: &mut OwnedRuntimeTemplateNode,
    context: &mut TemplateNormalizationContext<'_>,
) -> Result<(), TemplateNormalizationError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            normalize_runtime_slot_template_expression_for_hir(expression, context)?;
        }

        OwnedRuntimeTemplateNode::Sequence { .. }
        | OwnedRuntimeTemplateNode::ChildTemplate { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::BranchChain { .. }
        | OwnedRuntimeTemplateNode::Loop { .. }
        | OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }

    Ok(())
}
#[cfg(test)]
#[path = "tests/normalize_ast_tests.rs"]
mod normalize_ast_tests;
