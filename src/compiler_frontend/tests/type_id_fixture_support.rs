//! Canonical type and phase-owner fixtures for AST-to-HIR tests.
//!
//! Completed expressions are appended to one explicit builder and frozen with their constants
//! and issuing type environment. No fixture repairs semantic types after publication.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, DeclarationValue, MultiBindTarget, MultiBindTargetKind, NodeKind,
};
use crate::compiler_frontend::ast::const_values::facts::AstConstFacts;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolve_expression_result_type;
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, CollectionExpressionType,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ConstructorField, ExprId,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::statements::functions::{ReturnChannel, ReturnSlot};
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

// ---------------------------------------------------------------------------
// Choice definition helper used by build_ast_with_choices
// ---------------------------------------------------------------------------

pub(crate) struct HirTestChoiceDefinition {
    pub(crate) nominal_path: PathId,
    pub(crate) variants: Vec<ChoiceVariant>,
}

// ---------------------------------------------------------------------------
// Return-slot helpers
// ---------------------------------------------------------------------------

pub(crate) fn error_return_slot(type_id: TypeId) -> ReturnSlot {
    ReturnSlot {
        type_id: Some(type_id),
        channel: ReturnChannel::Error,
    }
}

// ---------------------------------------------------------------------------
// Parameter / declaration helpers
// ---------------------------------------------------------------------------

pub(crate) fn loop_binding_with_type_id(name: PathId, type_id: TypeId) -> Declaration {
    Declaration {
        id: name,
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    }
}

// ---------------------------------------------------------------------------
// Expression helpers
// ---------------------------------------------------------------------------

pub(crate) fn const_record_reference_expr(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    builder
        .reference_with_type_id(
            name,
            type_id,
            span,
            value_mode,
            ConstRecordState::ConstRecord,
        )
        .expect("fixture reference fits expression store")
        .id()
}

/// Resolve transient fixture RPN once, then retain only completed operation nodes.
pub(crate) fn runtime_expr(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    mut items: Vec<ExpressionRpnItem>,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    type_environment: &TypeEnvironment,
) -> ExprId {
    let mut string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let resolved = resolve_expression_result_type(
        &mut items,
        span,
        None,
        builder,
        const_values,
        &mut string_table,
        type_environment,
        &path_fork,
    )
    .unwrap_or_else(|_| panic!("runtime fixture operators should resolve"));
    assert_eq!(resolved.type_id, type_id, "runtime fixture result type");
    let mut stack = Vec::new();
    for item in items {
        match item {
            ExpressionRpnItem::Operand(id) => stack.push(builder.own(id)),
            ExpressionRpnItem::ResolvedOperator(operation) => {
                let right = if operation.operator.required_values() == 2 {
                    Some(
                        stack
                            .pop()
                            .expect("resolved binary fixture has right operand"),
                    )
                } else {
                    None
                };
                let left = stack.pop().expect("resolved fixture has left operand");
                let id = builder
                    .resolved_operation(left, right, operation, value_mode)
                    .expect("resolved fixture operation fits store");
                stack.push(id);
            }
            _ => panic!("fixture RPN must contain only resolved completed operands"),
        }
    }
    assert_eq!(stack.len(), 1, "fixture RPN must produce one root");
    stack.pop().expect("one fixture root").id()
}

pub(crate) fn runtime_operand_item(expression: ExprId) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(expression)
}

pub(crate) fn runtime_operator_item(
    operator: Operator,
    span: Option<SourceSpan>,
) -> ExpressionRpnItem {
    ExpressionRpnItem::Operator { operator, span }
}

pub(crate) fn runtime_function_call_item(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    result_type_ids: Vec<TypeId>,
    span: Option<SourceSpan>,
) -> ExpressionRpnItem {
    runtime_operand_item(
        builder
            .function_call_with_typed_arguments(name, vec![], result_type_ids, span)
            .expect("fixture call fits expression store")
            .id(),
    )
}

pub(crate) fn runtime_handled_function_call_item(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    result_type_ids: Vec<TypeId>,
    error_type: TypeId,
    handling: FallibleHandling,
    span: Option<SourceSpan>,
) -> ExpressionRpnItem {
    runtime_operand_item(
        builder
            .handled_fallible_function_call_with_typed_arguments(
                name,
                vec![],
                result_type_ids,
                error_type,
                fixture_fallible_expression_handling(&handling),
                span,
            )
            .expect("fixture handled call fits expression store")
            .id(),
    )
}

fn fixture_fallible_expression_handling(handling: &FallibleHandling) -> FallibleExpressionHandling {
    match handling {
        FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
        FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
    }
}

pub(crate) fn collection_expr(
    builder: &mut AstExpressionBuilder,
    items: Vec<ExprId>,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    type_environment: &mut TypeEnvironment,
) -> ExprId {
    let element_type_id = builder
        .view(
            *items
                .first()
                .expect("empty fixture collection needs explicit type"),
        )
        .expect("fixture collection child")
        .value_type_id()
        .expect("single-valued child");
    builder
        .collection_with_type_id(
            builder.own_slice(&items),
            CollectionExpressionType {
                element_type_id,
                fixed_capacity: None,
                collection_type_id: None,
            },
            type_environment,
            span,
            value_mode,
        )
        .expect("fixture collection fits store")
        .id()
}

pub(crate) fn multi_bind_target(
    id: PathId,
    type_id: TypeId,
    value_mode: ValueMode,
    kind: MultiBindTargetKind,
    span: Option<SourceSpan>,
) -> MultiBindTarget {
    MultiBindTarget {
        id,
        type_id,
        value_mode,
        kind,
        span,
    }
}

pub(crate) fn choice_construct_expr(
    builder: &mut AstExpressionBuilder,
    _nominal_path: PathId,
    tag: usize,
    fields: Vec<ConstructorField>,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    builder
        .choice_construct(ChoiceConstructInput {
            tag,
            fields: builder.own_slice(&fields),
            type_id,
            span,
            value_mode,
        })
        .expect("fixture choice fits expression store")
        .id()
}

pub(crate) fn option_none_expr(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    inner_type_id: TypeId,
    type_environment: &mut TypeEnvironment,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .option_none_with_type_id(const_values, inner_type_id, type_environment, span)
        .expect("fixture optional constant fits store")
        .id()
}

pub(crate) fn choice_type_id(
    type_environment: &mut TypeEnvironment,
    path: PathId,
    variants: &[crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant],
) -> TypeId {
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
                            name: field.id,
                            type_id: field.type_id,
                            span: field.binding_span,
                        })
                        .collect::<Vec<_>>();
                    ChoiceVariantPayloadDefinition::Record {
                        fields: field_definitions.into_boxed_slice(),
                    }
                }
            },
            span: variant.span,
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

/// Freeze the fixture's issuing expression domain while retaining its canonical types.
pub(crate) fn build_ast_with_registered_types(
    nodes: Vec<AstNode>,
    entry_path: PathId,
    builder: AstExpressionBuilder,
    const_values: ConstValueStore,
    type_environment: TypeEnvironment,
) -> Ast {
    build_ast_with_choices(
        nodes,
        entry_path,
        vec![],
        builder,
        const_values,
        type_environment,
    )
}

pub(crate) fn build_ast_with_choices(
    mut nodes: Vec<AstNode>,
    entry_path: PathId,
    choice_definitions: Vec<HirTestChoiceDefinition>,
    builder: AstExpressionBuilder,
    const_values: ConstValueStore,
    mut type_environment: TypeEnvironment,
) -> Ast {
    // Register struct definitions from AST nodes so that HIR lowering can
    // resolve frontend TypeIds during declaration registration.
    for node in &nodes {
        if let NodeKind::StructDefinition(name, fields) = &node.kind {
            let field_definitions = fields
                .iter()
                .map(|field| FieldDefinition {
                    name: field.id,
                    type_id: field.type_id,
                    span: field.binding_span,
                })
                .collect::<Vec<_>>();

            let definition = StructTypeDefinition {
                id: NominalTypeId(0),
                path: *name,
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
        // Already interned via choice_type_id into this environment; registering again
        // would allocate a second nominal for the same path.
        if type_environment
            .nominal_id_for_path(&choice_def.nominal_path)
            .is_some()
        {
            continue;
        }
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
                                name: field.id,
                                type_id: field.type_id,
                                span: field.binding_span,
                            })
                            .collect::<Vec<_>>();
                        ChoiceVariantPayloadDefinition::Record {
                            fields: field_definitions.into_boxed_slice(),
                        }
                    }
                },
                span: variant.span,
            })
            .collect::<Vec<_>>();

        let definition = ChoiceTypeDefinition {
            id: NominalTypeId(0),
            path: choice_def.nominal_path,
            variants: variant_definitions.into_boxed_slice(),
            generic_parameters: None,
        };

        let _ = type_environment.register_nominal_choice(definition);
    }

    // Constructed expression types were selected in the issuing environment before append.
    let expressions = builder
        .freeze(&mut nodes)
        .expect("completed fixture expression store");

    Ast {
        root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        nodes,
        const_values,
        expressions,
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
        infallible_cast_evidence: Default::default(),
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
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    external_packages: &ExternalPackageRegistry,
) -> Result<
    (
        HirModule,
        crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    ),
    PreparedFailure,
> {
    let result = lower_ast_with_metadata(ast, string_table, path_fork, external_packages)?;
    Ok((result.hir_module, result.type_environment))
}

/// Lower a test `Ast` and return the full named lowering result, including extracted non-HIR
/// module metadata.
///
/// WHAT: a narrowly named helper for tests that genuinely need to assert extracted documentation
///       fragments or rendered-path metadata. It does not widen the common `lower_ast` contract.
pub(crate) fn lower_ast_with_metadata(
    mut ast: Ast,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    external_packages: &ExternalPackageRegistry,
) -> Result<crate::compiler_frontend::module_metadata::HirLoweringResult, PreparedFailure> {
    let type_environment = std::mem::take(&mut ast.type_environment);
    HirBuilder::new(
        string_table,
        path_fork,
        type_environment,
        crate::compiler_frontend::hir::functions::HirFunctionOriginLookup::default(),
        Default::default(),
    )
    .build_hir_module(ast, external_packages)
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
