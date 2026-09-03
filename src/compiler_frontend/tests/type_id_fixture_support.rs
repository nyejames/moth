//! TypeId-first test helpers for HIR lowering tests.
//!
//! WHAT: wraps AST construction that still requires `DataType` internally so
//!       HIR test files can remain free of parse-era type-syntax references.
//! WHY: production AST nodes carry `diagnostic_type` for render support;
//!      test fixtures should set canonical `TypeId`s and let this module
//!      handle the diagnostic-only placeholder.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, MultiBindTarget, MultiBindTargetKind, NodeKind, SourceLocation,
};
use crate::compiler_frontend::ast::const_values::facts::AstConstFacts;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression::{
    ConstRecordState, Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
    Operator,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::functions::{ReturnChannel, ReturnSlot};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::value_mode::ValueMode;

// ---------------------------------------------------------------------------
// Choice definition helper used by build_ast_with_choices
// ---------------------------------------------------------------------------

pub(crate) struct HirTestChoiceDefinition {
    pub(crate) nominal_path: InternedPath,
    pub(crate) variants: Vec<ChoiceVariant>,
}

// ---------------------------------------------------------------------------
// Return-slot helpers
// ---------------------------------------------------------------------------

pub(crate) use crate::compiler_frontend::tests::ast_fixture_support::fresh_success_returns;

pub(crate) fn success_return_slot(type_id: TypeId) -> ReturnSlot {
    ReturnSlot {
        value: DataType::Inferred,
        type_id: Some(type_id),
        reactive_template: None,
        channel: ReturnChannel::Success,
    }
}

pub(crate) fn error_return_slot(type_id: TypeId) -> ReturnSlot {
    ReturnSlot {
        value: DataType::Inferred,
        type_id: Some(type_id),
        reactive_template: None,
        channel: ReturnChannel::Error,
    }
}

// ---------------------------------------------------------------------------
// Parameter / declaration helpers
// ---------------------------------------------------------------------------

pub(crate) fn param_with_type_id(
    name: InternedPath,
    type_id: TypeId,
    mutable: bool,
    location: SourceLocation,
) -> Declaration {
    param_declaration(name, type_id, mutable, location)
}

pub(crate) fn param_declaration(
    name: InternedPath,
    type_id: TypeId,
    mutable: bool,
    location: SourceLocation,
) -> Declaration {
    let value_mode = if mutable {
        ValueMode::MutableOwned
    } else {
        ValueMode::ImmutableOwned
    };

    Declaration {
        id: name,
        value: Expression::new(
            ExpressionKind::NoValue,
            location,
            type_id,
            DataType::Inferred,
            value_mode,
        ),
        config_qualifier: None,
    }
}

pub(crate) fn loop_binding_with_type_id(
    name: &str,
    type_id: TypeId,
    string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
) -> Declaration {
    let location = crate::compiler_frontend::tokenizer::tokens::SourceLocation::default();
    param_with_type_id(
        InternedPath::from_single_str(name, string_table),
        type_id,
        false,
        location,
    )
}

// ---------------------------------------------------------------------------
// Expression helpers
// ---------------------------------------------------------------------------

/// A reference expression whose diagnostic type is fixed to `DataType::Inferred`.
///
/// The caller supplies the `ValueMode`. Named for the type it fixes so it cannot be confused
/// with `ast_fixture_support::immutable_reference_expr`, which fixes the mode instead.
pub(crate) fn inferred_type_reference_expr(
    name: InternedPath,
    type_id: TypeId,
    location: SourceLocation,
    value_mode: ValueMode,
) -> Expression {
    Expression::reference_with_type_id(
        name,
        DataType::Inferred,
        type_id,
        location,
        value_mode,
        ConstRecordState::RuntimeValue,
    )
}

pub(crate) fn const_record_reference_expr(
    name: InternedPath,
    type_id: TypeId,
    location: SourceLocation,
    value_mode: ValueMode,
) -> Expression {
    Expression::reference_with_type_id(
        name,
        DataType::Inferred,
        type_id,
        location,
        value_mode,
        ConstRecordState::ConstRecord,
    )
}

pub(crate) fn no_value_expr(
    type_id: TypeId,
    location: SourceLocation,
    value_mode: ValueMode,
) -> Expression {
    Expression::new(
        ExpressionKind::NoValue,
        location,
        type_id,
        DataType::Inferred,
        value_mode,
    )
}

pub(crate) fn runtime_expr(
    items: Vec<ExpressionRpnItem>,
    type_id: TypeId,
    location: SourceLocation,
    value_mode: ValueMode,
) -> Expression {
    let contains_regular_division = items.iter().any(rpn_item_has_regular_division);
    Expression::new(
        ExpressionKind::Runtime(ExpressionRpn { items }),
        location,
        type_id,
        DataType::Inferred,
        value_mode,
    )
    .with_regular_division_provenance(contains_regular_division)
}

pub(crate) fn runtime_operand_item(expression: Expression) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(expression)
}

pub(crate) fn runtime_operator_item(
    operator: Operator,
    location: SourceLocation,
) -> ExpressionRpnItem {
    ExpressionRpnItem::Operator { operator, location }
}

pub(crate) fn runtime_function_call_item(
    name: InternedPath,
    result_type_ids: Vec<TypeId>,
    location: SourceLocation,
) -> ExpressionRpnItem {
    let expression_type_id = single_fixture_result_type_id(&result_type_ids);

    runtime_operand_item(Expression::new(
        ExpressionKind::FunctionCall {
            name,
            args: vec![],
            result_type_ids,
        },
        location,
        expression_type_id,
        DataType::Inferred,
        ValueMode::MutableOwned,
    ))
}

pub(crate) fn runtime_handled_function_call_item(
    name: InternedPath,
    result_type_ids: Vec<TypeId>,
    handling: FallibleHandling,
    location: SourceLocation,
) -> ExpressionRpnItem {
    let expression_type_id = single_fixture_result_type_id(&result_type_ids);
    let expression_handling = fixture_fallible_expression_handling(&handling);

    runtime_operand_item(Expression::new(
        ExpressionKind::HandledFallibleFunctionCall {
            name,
            args: vec![],
            result_type_ids,
            handling: expression_handling,
            propagation_location: None,
        },
        location,
        expression_type_id,
        DataType::Inferred,
        ValueMode::MutableOwned,
    ))
}

fn rpn_item_has_regular_division(item: &ExpressionRpnItem) -> bool {
    matches!(
        item,
        ExpressionRpnItem::Operator {
            operator: Operator::Divide,
            ..
        }
    )
}

fn single_fixture_result_type_id(result_type_ids: &[TypeId]) -> TypeId {
    match result_type_ids {
        [] => builtin_type_ids::NONE,
        [single] => *single,
        multiple => panic!(
            "test fixture runtime RPN call operand must have at most one result, got {}",
            multiple.len()
        ),
    }
}

fn fixture_fallible_expression_handling(handling: &FallibleHandling) -> FallibleExpressionHandling {
    match handling {
        FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
        FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
    }
}

pub(crate) fn collection_expr(
    items: Vec<Expression>,
    location: SourceLocation,
    value_mode: ValueMode,
) -> Expression {
    let contains_regular_division = items.iter().any(|item| item.contains_regular_division);
    Expression::new(
        ExpressionKind::Collection(items),
        location,
        builtin_type_ids::NONE,
        DataType::Inferred,
        value_mode,
    )
    .with_regular_division_provenance(contains_regular_division)
}

pub(crate) fn multi_bind_target(
    id: InternedPath,
    type_id: TypeId,
    value_mode: ValueMode,
    kind: MultiBindTargetKind,
    location: SourceLocation,
) -> MultiBindTarget {
    MultiBindTarget {
        id,
        type_id,
        value_mode,
        kind,
        location,
    }
}

pub(crate) fn field_access_node(
    base: Expression,
    field: crate::compiler_frontend::symbols::string_interning::StringId,
    type_id: TypeId,
    const_record_state: ConstRecordState,
    value_mode: ValueMode,
    location: SourceLocation,
) -> AstNode {
    let mut expression = Expression::new(
        ExpressionKind::FieldAccess {
            base: Box::new(base),
            field,
        },
        location.clone(),
        type_id,
        DataType::Inferred,
        value_mode,
    );
    expression.const_record_state = const_record_state;

    AstNode {
        kind: NodeKind::ExpressionStatement(expression),
        location,
        scope: InternedPath::new(),
    }
}

pub(crate) fn choice_construct_expr(
    nominal_path: InternedPath,
    tag: usize,
    fields: Vec<Declaration>,
    type_id: TypeId,
    location: SourceLocation,
    value_mode: ValueMode,
) -> Expression {
    Expression::choice_construct(
        crate::compiler_frontend::ast::expressions::expression::ChoiceConstructInput {
            nominal_path,
            tag,
            fields,
            diagnostic_type: DataType::Inferred,
            type_id,
            location,
            value_mode,
        },
    )
}

pub(crate) fn option_none_expr(
    inner_type_id: TypeId,
    type_environment: &mut TypeEnvironment,
    location: SourceLocation,
) -> Expression {
    Expression::option_none_with_type_id(
        inner_type_id,
        DataType::Inferred,
        type_environment,
        location,
    )
}

pub(crate) fn result_carrier_type_id(
    type_environment: &mut TypeEnvironment,
    success_type_id: TypeId,
    error_type_id: TypeId,
) -> TypeId {
    type_environment.intern_fallible_carrier(success_type_id, error_type_id)
}

pub(crate) fn handled_result_expr(
    value: Expression,
    handling: FallibleHandling,
    result_type_id: TypeId,
    result_type_ids: Vec<TypeId>,
    location: SourceLocation,
) -> Expression {
    let expression_handling = match &handling {
        FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
        FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
    };

    let handled_expression = Expression::handled_result_with_type_id(
        value,
        expression_handling,
        result_type_id,
        DataType::Inferred,
        location.clone(),
    );

    match handling {
        FallibleHandling::Propagate => handled_expression,
        FallibleHandling::Handler { .. } => {
            wrap_catch_expression(handled_expression, handling, result_type_ids)
        }
    }
}

// ---------------------------------------------------------------------------
// AST construction
// ---------------------------------------------------------------------------

fn register_collection_types_from_nodes(
    nodes: &mut [AstNode],
    type_environment: &mut TypeEnvironment,
) {
    for node in nodes.iter_mut() {
        register_collection_types_from_node(node, type_environment);
    }
}

fn register_collection_types_from_node(node: &mut AstNode, type_environment: &mut TypeEnvironment) {
    match &mut node.kind {
        NodeKind::Return(exprs) => {
            for expr in exprs {
                register_collection_types_from_expression(expr, type_environment);
            }
        }
        NodeKind::ReturnError(expr) => {
            register_collection_types_from_expression(expr, type_environment);
        }
        NodeKind::If(condition, then_body, else_body, _) => {
            register_collection_types_from_expression(condition, type_environment);
            register_collection_types_from_nodes(then_body, type_environment);
            if let Some(else_nodes) = else_body {
                register_collection_types_from_nodes(else_nodes, type_environment);
            }
        }
        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            register_collection_types_from_expression(scrutinee, type_environment);
            for arm in arms {
                register_collection_types_from_nodes(&mut arm.body, type_environment);
            }
            if let Some(default_nodes) = default {
                register_collection_types_from_nodes(default_nodes, type_environment);
            }
        }
        NodeKind::LexicalScope { body } => {
            register_collection_types_from_nodes(body, type_environment);
        }
        NodeKind::RangeLoop { range, body, .. } => {
            register_collection_types_from_expression(&mut range.start, type_environment);
            register_collection_types_from_expression(&mut range.end, type_environment);
            if let Some(step) = &mut range.step {
                register_collection_types_from_expression(step, type_environment);
            }
            register_collection_types_from_nodes(body, type_environment);
        }
        NodeKind::CollectionLoop { iterable, body, .. } => {
            register_collection_types_from_expression(iterable, type_environment);
            register_collection_types_from_nodes(body, type_environment);
        }
        NodeKind::WhileLoop(condition, body) => {
            register_collection_types_from_expression(condition, type_environment);
            register_collection_types_from_nodes(body, type_environment);
        }
        NodeKind::VariableDeclaration(Declaration { value, .. }) => {
            register_collection_types_from_expression(value, type_environment);
        }
        NodeKind::PushStartRuntimeFragment(expr) => {
            register_collection_types_from_expression(expr, type_environment);
        }
        NodeKind::StructDefinition(_, fields) => {
            for field in fields.iter_mut() {
                register_collection_types_from_expression(&mut field.value, type_environment);
            }
        }
        NodeKind::Function(_, _, body) => {
            register_collection_types_from_nodes(body, type_environment);
        }
        NodeKind::ThenValue(produced_values) => {
            for expr in &mut produced_values.expressions {
                register_collection_types_from_expression(expr, type_environment);
            }
        }
        _ => {}
    }
}

fn register_collection_types_from_expression(
    expr: &mut Expression,
    type_environment: &mut TypeEnvironment,
) {
    if let ExpressionKind::Collection(items) = &mut expr.kind {
        if let Some(first) = items.first() {
            let collection_type_id = type_environment.intern_collection(first.type_id, None);
            expr.type_id = collection_type_id;
        }
        for item in items {
            register_collection_types_from_expression(item, type_environment);
        }
        return;
    }

    match &mut expr.kind {
        ExpressionKind::Runtime(rpn) => {
            for item in &mut rpn.items {
                if let ExpressionRpnItem::Operand(expression) = item {
                    register_collection_types_from_expression(expression, type_environment);
                }
            }
        }
        ExpressionKind::Copy(_) => {
            // Places carry no nested expressions that need collection-type registration.
        }
        ExpressionKind::Function(_) => {}
        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. } => {
            for arg in args {
                register_collection_types_from_expression(&mut arg.value, type_environment);
            }
        }
        ExpressionKind::StructDefinition(fields)
        | ExpressionKind::StructInstance(fields)
        | ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                register_collection_types_from_expression(&mut field.value, type_environment);
            }
        }
        ExpressionKind::Range(start, end) => {
            register_collection_types_from_expression(start, type_environment);
            register_collection_types_from_expression(end, type_environment);
        }
        _ => {}
    }
}

pub(crate) fn choice_type_id(
    path: InternedPath,
    variants: &[crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant],
) -> TypeId {
    let mut type_environment = TypeEnvironment::new();
    let variant_definitions = variants
        .iter()
        .enumerate()
        .map(|(tag, variant)| ChoiceVariantDefinition {
            name: variant.id,
            tag,
            payload: match &variant.payload {
                crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayload::Unit => {
                    ChoiceVariantPayloadDefinition::Unit
                }
                crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayload::Record {
                    fields,
                } => {
                    let field_definitions = fields
                        .iter()
                        .map(|field| FieldDefinition {
                            name: field.id.clone(),
                            type_id: field.value.type_id,
                            location: field.value.location.clone(),
                        })
                        .collect::<Vec<_>>();
                    ChoiceVariantPayloadDefinition::Record {
                        fields: field_definitions.into_boxed_slice(),
                    }
                }
            },
            location: variant.location.clone(),
        })
        .collect::<Vec<_>>();

    let definition = ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: variant_definitions.into_boxed_slice(),
        generic_parameters: None,
    };

    let (_, type_id) = type_environment.register_nominal_choice(definition);
    type_id
}

/// Builds an `Ast` after registering every struct and choice definition its nodes mention.
///
/// WHAT: the single AST fixture constructor. It walks `nodes` for struct and choice definitions
///       and registers them in a fresh `TypeEnvironment` before construction.
/// WHY:  named for what it does rather than `build_ast_with_registered_types`, because HIR lowering resolves frontend
///       `TypeId`s during declaration registration and a fixture that skipped that step would
///       fail for reasons that have nothing to do with the test's subject.
pub(crate) fn build_ast_with_registered_types(
    nodes: Vec<AstNode>,
    entry_path: InternedPath,
) -> Ast {
    build_ast_with_choices(nodes, entry_path, vec![])
}

pub(crate) fn build_ast_with_choices(
    mut nodes: Vec<AstNode>,
    entry_path: InternedPath,
    choice_definitions: Vec<HirTestChoiceDefinition>,
) -> Ast {
    let mut type_environment = TypeEnvironment::new();

    // Register struct definitions from AST nodes so that HIR lowering can
    // resolve frontend TypeIds during declaration registration.
    for node in &nodes {
        if let NodeKind::StructDefinition(name, fields) = &node.kind {
            let field_definitions = fields
                .iter()
                .map(|field| FieldDefinition {
                    name: field.id.clone(),
                    type_id: field.value.type_id,
                    location: field.value.location.clone(),
                })
                .collect::<Vec<_>>();

            let definition = StructTypeDefinition {
                id: NominalTypeId(0),
                path: name.clone(),
                fields: field_definitions.into_boxed_slice(),
                generic_parameters: None,
                const_record: false,
            };

            let _ = type_environment.register_nominal_struct(definition);
        }
    }

    // Register choice definitions so expression canonicalization can resolve
    // choice TypeIds before HIR lowering.
    for choice_def in &choice_definitions {
        let variant_definitions = choice_def
            .variants
            .iter()
            .enumerate()
            .map(|(tag, variant)| ChoiceVariantDefinition {
                name: variant.id,
                tag,
                payload: match &variant.payload {
                    crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayload::Unit => {
                        ChoiceVariantPayloadDefinition::Unit
                    }
                    crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayload::Record {
                        fields,
                    } => {
                        let field_definitions = fields
                            .iter()
                            .map(|field| FieldDefinition {
                                name: field.id.clone(),
                                type_id: field.value.type_id,
                                location: field.value.location.clone(),
                            })
                            .collect::<Vec<_>>();
                        ChoiceVariantPayloadDefinition::Record {
                            fields: field_definitions.into_boxed_slice(),
                        }
                    }
                },
                location: variant.location.clone(),
            })
            .collect::<Vec<_>>();

        let definition = ChoiceTypeDefinition {
            id: NominalTypeId(0),
            path: choice_def.nominal_path.clone(),
            variants: variant_definitions.into_boxed_slice(),
            generic_parameters: None,
        };

        let _ = type_environment.register_nominal_choice(definition);
    }

    // Scan AST for collection literals and register their constructed types so
    // that HIR loop lowering can resolve collection type identities.
    register_collection_types_from_nodes(&mut nodes, &mut type_environment);

    Ast {
        root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        nodes,
        const_values: ConstValueStore::default(),
        doc_fragments: vec![],
        entry_path,
        const_top_level_fragments: vec![],
        warnings: vec![],
        choice_definitions: choice_definitions
            .into_iter()
            .map(
                |definition| crate::compiler_frontend::ast::AstChoiceDefinition {
                    nominal_path: definition.nominal_path,
                },
            )
            .collect(),
        type_environment,
        const_facts: AstConstFacts::default(),
        imported_functions_by_local_path: Default::default(),
        imported_struct_definitions: vec![],
        static_if_function_provenance: Default::default(),
    }
}

/// Lower a test `Ast` into a `HirModule` and its `TypeEnvironment`.
///
/// WHAT: preserves the established two-element HIR lowering test contract by destructuring the
///       named production lowering result inside the helper. Most HIR tests only need the module
///       and type environment and should not know about extracted module metadata.
pub(crate) fn lower_ast(
    ast: Ast,
    string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
) -> Result<
    (
        HirModule,
        crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    ),
    CompilerMessages,
> {
    let result = lower_ast_with_metadata(ast, string_table)?;
    Ok((result.hir_module, result.type_environment))
}

/// Lower a test `Ast` and return the full named lowering result, including extracted non-HIR
/// module metadata.
///
/// WHAT: a narrowly named helper for tests that genuinely need to assert extracted documentation
///       fragments or rendered-path metadata. It does not widen the common `lower_ast` contract.
pub(crate) fn lower_ast_with_metadata(
    ast: Ast,
    string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
) -> Result<crate::compiler_frontend::module_metadata::HirLoweringResult, CompilerMessages> {
    let type_environment = ast.type_environment.clone();
    HirBuilder::new(
        string_table,
        type_environment,
        crate::compiler_frontend::hir::functions::HirFunctionOriginLookup::default(),
    )
    .build_hir_module(ast)
}

/// Assert that no block ends with a placeholder `Uninitialized` terminator.
pub(crate) fn assert_no_placeholder_terminators(module: &HirModule) {
    assert!(
        module
            .blocks
            .iter()
            .all(|block| !matches!(block.terminator, HirTerminator::Uninitialized)),
        "expected no placeholder Uninitialized terminators in lowered HIR"
    );
}
