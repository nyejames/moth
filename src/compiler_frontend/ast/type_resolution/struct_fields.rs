//! Struct field type resolution and default-value constant inlining.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::eval_expression::evaluate_expression;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, resolve_diagnostic_type_to_type_id_checked,
};
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use rustc_hash::FxHashSet;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use super::resolve_named_signature_type;

pub(crate) enum StructFieldResolutionError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for StructFieldResolutionError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        StructFieldResolutionError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<Box<CompilerDiagnostic>> for StructFieldResolutionError {
    fn from(diagnostic: Box<CompilerDiagnostic>) -> Self {
        StructFieldResolutionError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for StructFieldResolutionError {
    fn from(error: CompilerError) -> Self {
        StructFieldResolutionError::Infrastructure(Box::new(error))
    }
}

impl From<Box<CompilerError>> for StructFieldResolutionError {
    fn from(error: Box<CompilerError>) -> Self {
        StructFieldResolutionError::Infrastructure(error)
    }
}

impl From<ExpressionTypingError> for StructFieldResolutionError {
    fn from(error: ExpressionTypingError) -> Self {
        match error {
            ExpressionTypingError::Diagnostic(diagnostic) => {
                StructFieldResolutionError::Diagnostic(diagnostic)
            }
            ExpressionTypingError::Infrastructure(error) => {
                StructFieldResolutionError::Infrastructure(error)
            }
        }
    }
}

impl From<TemplateError> for StructFieldResolutionError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::Diagnostic(diagnostic) => {
                StructFieldResolutionError::Diagnostic(diagnostic)
            }
            TemplateError::Infrastructure(error) => {
                StructFieldResolutionError::Infrastructure(error)
            }
        }
    }
}

// ----------------------------------
//  Struct field type resolution
// ----------------------------------

/// Resolve all declared struct field types against visible declarations.
pub(crate) fn resolve_struct_field_types(
    struct_path: &InternedPath,
    fields: &[Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<Vec<Declaration>, StructFieldResolutionError> {
    let mut resolved_fields =
        resolve_struct_field_type_shells(fields, type_resolution_context, string_table)?;
    resolve_struct_field_defaults(
        &mut resolved_fields,
        type_resolution_context,
        template_ir_store,
        string_table,
    )?;

    validate_resolved_field_parent_paths(struct_path, &resolved_fields)?;

    Ok(resolved_fields)
}

/// Resolve struct field shell types for constant-time constructor parsing.
///
/// WHAT: turns parsed field type annotations into semantic `TypeId`s without inlining or
/// validating default expressions.
/// WHY: constants are resolved before final nominal member definitions are written to
/// `TypeEnvironment`; their constructors still need checked semantic field types, while
/// defaults may legitimately reference constants that are not resolved until this stage runs.
pub(crate) fn resolve_struct_constructor_shell_types(
    struct_path: &InternedPath,
    fields: &[Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
) -> Result<Vec<Declaration>, StructFieldResolutionError> {
    let resolved_fields =
        resolve_struct_field_type_shells(fields, type_resolution_context, string_table)?;

    validate_resolved_field_parent_paths(struct_path, &resolved_fields)?;

    Ok(resolved_fields)
}

fn validate_resolved_field_parent_paths(
    struct_path: &InternedPath,
    resolved_fields: &[Declaration],
) -> Result<(), StructFieldResolutionError> {
    if resolved_fields.is_empty() {
        return Ok(());
    }

    for field in resolved_fields {
        let Some(parent) = field.id.parent() else {
            return Err(CompilerError::compiler_error(
                "Resolved struct field is missing its parent struct path.",
            )
            .into());
        };

        if parent != *struct_path {
            return Err(CompilerError::compiler_error(
                "Resolved struct field parent does not match the enclosing struct declaration.",
            )
            .into());
        }
    }

    Ok(())
}

fn resolve_struct_field_type_shells(
    fields: &[Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
) -> Result<Vec<Declaration>, StructFieldResolutionError> {
    // WHY: Struct fields must enter AST/HIR in fully resolved nominal form so later
    // phases do not carry unresolved `NamedType` placeholders.
    let mut resolved_fields = Vec::with_capacity(fields.len());

    for field in fields {
        let mut resolved_field = field.to_owned();

        resolved_field.value.diagnostic_type = resolve_named_signature_type(
            &field.value.diagnostic_type,
            &field.value.location,
            type_resolution_context,
            string_table,
        )?;

        let type_environment = &mut *type_resolution_context.type_environment;

        resolved_field.value.type_id = resolve_diagnostic_type_to_type_id_checked(
            &resolved_field.value.diagnostic_type,
            type_environment,
            &resolved_field.value.location,
        )?;

        resolved_fields.push(resolved_field);
    }

    Ok(resolved_fields)
}

/// Inline visible constants, then validate each final field default through the module TIR store.
///
/// WHAT: final struct definitions resolve default values after all constant headers are available.
/// WHY: constructor shells need only field types, so keeping default classification here avoids
///      lending the TIR store through an earlier type-only pass that cannot consume it.
fn resolve_struct_field_defaults(
    resolved_fields: &mut [Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<(), StructFieldResolutionError> {
    for resolved_field in resolved_fields {
        let type_environment = &mut *type_resolution_context.type_environment;
        resolved_field.value = inline_visible_constant_references(
            &resolved_field.value,
            type_resolution_context.declaration_table,
            type_resolution_context.visible_declaration_ids,
            type_environment,
            template_ir_store,
            string_table,
        )?;

        // Reference eligibility and final validation use the same module-store view authority.
        // Runtime expression rebuilding still evaluates through the active module store.
        let default_value_is_constant = resolved_field
            .value
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, template_ir_store)
            })?
            .is_compile_time_value();

        if !matches!(resolved_field.value.kind, ExpressionKind::NoValue)
            && !default_value_is_constant
        {
            return Err(CompilerDiagnostic::invalid_struct_default_value(
                resolved_field.value.location.clone(),
            )
            .into());
        }
    }

    Ok(())
}

// ----------------------------------
//  Constant inlining for field defaults
// ----------------------------------

fn inline_visible_constant_references(
    expression: &Expression,
    declaration_table: &Rc<TopLevelDeclarationTable>,
    visible_declaration_ids: Option<&Arc<FxHashSet<InternedPath>>>,
    type_environment: &mut TypeEnvironment,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<Expression, StructFieldResolutionError> {
    inline_visible_constant_references_impl(
        expression,
        declaration_table,
        visible_declaration_ids,
        type_environment,
        template_ir_store,
        string_table,
    )
}

fn inline_visible_constant_references_impl(
    expression: &Expression,
    declaration_table: &Rc<TopLevelDeclarationTable>,
    visible_declaration_ids: Option<&Arc<FxHashSet<InternedPath>>>,
    type_environment: &mut TypeEnvironment,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<Expression, StructFieldResolutionError> {
    match &expression.kind {
        // Direct reference — try to resolve to a visible compile-time constant.
        ExpressionKind::Reference(path) => {
            let inlinable_declaration = visible_compile_time_constant_reference(
                path,
                declaration_table,
                visible_declaration_ids,
                template_ir_store,
            )?;

            Ok(inlinable_declaration
                .map(|declaration| {
                    let mut resolved = declaration.value.to_owned();
                    resolved.location = expression.location.clone();
                    resolved
                })
                .unwrap_or_else(|| expression.to_owned()))
        }

        // Runtime expression — inline constants inside nested nodes, then re-evaluate.
        ExpressionKind::Runtime(rpn) => {
            let mut rewritten_items = Vec::with_capacity(rpn.items.len());

            for item in &rpn.items {
                rewritten_items.push(inline_visible_constant_references_in_rpn_item(
                    item,
                    declaration_table,
                    visible_declaration_ids,
                    type_environment,
                    template_ir_store,
                    string_table,
                )?);
            }

            let mut current_type = ExpectedType::Known(expression.type_id);

            let mut evaluation_context = ScopeContext::new(
                ContextKind::ConstantHeader,
                expression.location.scope.to_owned(),
                Rc::clone(declaration_table),
                Arc::new(ExternalPackageRegistry::new()),
                Vec::new(),
                0,
                Rc::clone(template_ir_store),
            );

            // The visibility set arrives as the same handle the scope stores, so entering the
            // field-default evaluation shares it instead of copying every visible path.
            evaluation_context.visible_declaration_ids = visible_declaration_ids.map(Arc::clone);

            let mut compatibility_cache = TypeCompatibilityCache::new();
            let mut type_interner =
                AstTypeInterner::new(type_environment, &mut compatibility_cache);
            evaluate_expression(
                &evaluation_context,
                rewritten_items,
                &mut type_interner,
                &mut current_type,
                &expression.value_mode,
                string_table,
            )
            .map_err(|_| {
                CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::StructFieldDefaultNotFoldable,
                    None,
                    expression.location.clone(),
                )
            })
            .map_err(StructFieldResolutionError::from)
        }

        // Collection — inline each element.
        ExpressionKind::Collection(elements) => {
            let mut resolved_elements = Vec::with_capacity(elements.len());

            for element in elements {
                resolved_elements.push(inline_visible_constant_references_impl(
                    element,
                    declaration_table,
                    visible_declaration_ids,
                    type_environment,
                    template_ir_store,
                    string_table,
                )?);
            }

            Ok(expression_with_inlined_kind(
                expression,
                ExpressionKind::Collection(resolved_elements),
            ))
        }

        // Struct instance — inline each field value.
        ExpressionKind::StructInstance(fields) => {
            let mut resolved_fields = Vec::with_capacity(fields.len());

            for field in fields {
                resolved_fields.push(Declaration {
                    id: field.id.to_owned(),
                    value: inline_visible_constant_references_impl(
                        &field.value,
                        declaration_table,
                        visible_declaration_ids,
                        type_environment,
                        template_ir_store,
                        string_table,
                    )?,
                });
            }

            Ok(expression_with_inlined_kind(
                expression,
                ExpressionKind::StructInstance(resolved_fields),
            ))
        }

        // Range — inline start and end.
        ExpressionKind::Range(start, end) => Ok(expression_with_inlined_kind(
            expression,
            ExpressionKind::Range(
                Box::new(inline_visible_constant_references(
                    start,
                    declaration_table,
                    visible_declaration_ids,
                    type_environment,
                    template_ir_store,
                    string_table,
                )?),
                Box::new(inline_visible_constant_references(
                    end,
                    declaration_table,
                    visible_declaration_ids,
                    type_environment,
                    template_ir_store,
                    string_table,
                )?),
            ),
        )),

        // Result construct — inline the wrapped value.
        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { variant, value } => {
            Ok(expression_with_inlined_kind(
                expression,
                ExpressionKind::FallibleCarrierConstruct {
                    variant: *variant,
                    value: Box::new(inline_visible_constant_references(
                        value,
                        declaration_table,
                        visible_declaration_ids,
                        type_environment,
                        template_ir_store,
                        string_table,
                    )?),
                },
            ))
        }

        // Coercion — inline the inner value.
        ExpressionKind::Coerced { value, to_type } => Ok(expression_with_inlined_kind(
            expression,
            ExpressionKind::Coerced {
                value: Box::new(inline_visible_constant_references(
                    value,
                    declaration_table,
                    visible_declaration_ids,
                    type_environment,
                    template_ir_store,
                    string_table,
                )?),
                to_type: *to_type,
            },
        )),

        // Everything else — no inlining needed.
        _ => Ok(expression.to_owned()),
    }
}

fn visible_compile_time_constant_reference<'a>(
    path: &InternedPath,
    declaration_table: &'a Rc<TopLevelDeclarationTable>,
    visible_declaration_ids: Option<&Arc<FxHashSet<InternedPath>>>,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<Option<&'a Declaration>, StructFieldResolutionError> {
    if let Some(declaration) = declaration_table
        .get_visible_resolved_by_path(path, visible_declaration_ids.map(Arc::as_ref))
    {
        let declaration_is_constant = expression_is_compile_time_constant_from_effective_tir(
            &declaration.value,
            template_ir_store,
        )?;

        if declaration_is_constant {
            return Ok(Some(declaration));
        }
    }

    let Some(name) = path.name() else {
        return Ok(None);
    };

    let Some(declaration) = declaration_table
        .get_visible_resolved_by_name(name, visible_declaration_ids.map(Arc::as_ref))
    else {
        return Ok(None);
    };

    if expression_is_compile_time_constant_from_effective_tir(
        &declaration.value,
        template_ir_store,
    )? {
        return Ok(Some(declaration));
    }

    Ok(None)
}

fn expression_is_compile_time_constant_from_effective_tir(
    expression: &Expression,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<bool, StructFieldResolutionError> {
    Ok(expression
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, template_ir_store)
        })?
        .is_compile_time_value())
}

fn expression_with_inlined_kind(expression: &Expression, kind: ExpressionKind) -> Expression {
    let mut rewritten = Expression::new(
        kind,
        expression.location.clone(),
        expression.type_id,
        expression.diagnostic_type.to_owned(),
        expression.value_mode.to_owned(),
    );

    // Constant-reference inlining replaces only the structural children. The surrounding value
    // keeps its previously resolved const-record, reactive, and division-provenance metadata.
    rewritten.const_record_state = expression.const_record_state;
    rewritten.reactive_source = expression.reactive_source.clone();
    rewritten.reactive_template = expression.reactive_template.clone();
    rewritten.contains_regular_division = expression.contains_regular_division;
    rewritten.synthetic_interface_provenance = expression.synthetic_interface_provenance.clone();
    rewritten
}

fn inline_visible_constant_references_in_rpn_item(
    item: &ExpressionRpnItem,
    declaration_table: &Rc<TopLevelDeclarationTable>,
    visible_declaration_ids: Option<&Arc<FxHashSet<InternedPath>>>,
    type_environment: &mut TypeEnvironment,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<ExpressionRpnItem, StructFieldResolutionError> {
    match item {
        ExpressionRpnItem::Operand(expression) => Ok(ExpressionRpnItem::Operand(
            inline_visible_constant_references_impl(
                expression,
                declaration_table,
                visible_declaration_ids,
                type_environment,
                template_ir_store,
                string_table,
            )?,
        )),
        ExpressionRpnItem::Operator { .. } => Ok(item.clone()),
    }
}
