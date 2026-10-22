//! HIR validation regression tests.
//!
//! WHAT: exercises the post-lowering HIR validator against valid and intentionally broken modules.
//! WHY: validator coverage needs focused tests that isolate invariants from the rest of lowering.

use super::{
    append_hir_expression, function_node, minimal_lowered_hir_module, node, start_entry_block_index,
};
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::prepared_batch::AstDiagnosticFailure;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, FunctionTypeKey, GenericParameterId, NominalTypeId, TypeConstructor,
    TypeId, builtin_type_ids,
};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField,
    OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::{
    HirTestChoiceDefinition, build_ast_with_choices, build_ast_with_registered_types, lower_ast,
    validate_module_for_tests,
};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::{
    BlockId, ChoiceId, FieldId, HirNodeId, HirValueId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::{
    HirChoice, HirChoiceField, HirChoiceVariant, HirModule,
};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode, RangeStepFailureCause,
};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::{
    HirAssertionMessageEvaluation, HirFallibleOperation, HirJumpArgument, HirTerminator,
};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tests::ast_fixture_support::{
    make_test_variable, param_with_type_id,
};
use crate::compiler_tests::test_support::run_stack_budgeted_child;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;

use crate::compiler_frontend::value_mode::ValueMode;

// Shared builders for the validation regressions below.
fn generic_parameter_type_id(
    string_table: &mut StringTable,
    type_environment: &mut TypeEnvironment,
) -> TypeId {
    let parameter_name = string_table.intern("T");
    type_environment.intern_generic_parameter(GenericParameterId(0), parameter_name)
}

fn add_local(module: &mut HirModule, block_id: BlockId, id: LocalId, ty: TypeId) {
    let block = module
        .blocks
        .iter_mut()
        .find(|block| block.id == block_id)
        .expect("test block should exist");
    block.locals.push(HirLocal {
        id,
        ty,
        mutable: false,
        region: block.region,
        span: None,
    });
}

fn add_entry_parameter(
    module: &mut HirModule,
    local_id: LocalId,
    ty: TypeId,
    mutable: bool,
) -> BlockId {
    let function_index = module
        .start_function
        .expect("test module should have start")
        .0 as usize;
    let entry = module.functions[function_index].entry;
    add_local(module, entry, local_id, ty);
    module.blocks[entry.0 as usize]
        .locals
        .iter_mut()
        .find(|local| local.id == local_id)
        .expect("entry parameter local should be registered")
        .mutable = mutable;
    module.functions[function_index].params.push(local_id);
    entry
}

fn append_entry_statement(module: &mut HirModule, entry: BlockId, kind: HirStatementKind) {
    module.blocks[entry.0 as usize]
        .statements
        .push(HirStatement {
            id: HirNodeId(9000),
            kind,
            span: None,
        });
}

fn append_failure_block(
    module: &mut HirModule,
    region: RegionId,
    locals: Vec<HirLocal>,
) -> BlockId {
    let id = BlockId(
        module
            .blocks
            .iter()
            .map(|block| block.id.0)
            .max()
            .unwrap_or_default()
            + 1,
    );
    module.blocks.push(HirBlock {
        id,
        region,
        locals,
        statements: vec![],
        terminator: HirTerminator::RuntimeFailure {
            message: "test terminator".to_owned(),
            cause: None,
        },
    });
    id
}

fn install_entry_self_jump(module: &mut HirModule, args: Vec<HirJumpArgument>) -> BlockId {
    let entry = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    module.blocks[entry.0 as usize].terminator = HirTerminator::Jump {
        target: entry,
        args,
    };
    entry
}

fn validation_error_for_injected_local_type(
    build_type: impl FnOnce(&mut StringTable, &mut TypeEnvironment) -> TypeId,
) -> CompilerError {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let local_type_id = build_type(&mut string_table, &mut type_environment);

    let entry_block_index = start_entry_block_index(&module);
    let entry_block = &mut module.blocks[entry_block_index];
    entry_block.locals.push(HirLocal {
        id: LocalId(9000),
        ty: local_type_id,
        mutable: false,
        region: entry_block.region,
        span: None,
    });

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter inside TypeId")
}

fn inject_collection_expression_statement(
    module: &mut HirModule,
    collection_type_id: TypeId,
    span: Option<SourceSpan>,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let empty_elements = module
        .expressions
        .append_values(&[], span)
        .expect("empty collection elements should fit the expression store");
    let value_id = append_hir_expression(
        module,
        HirExpressionKind::Collection(empty_elements),
        collection_type_id,
        entry_region,
        ValueKind::RValue,
    );
    let statement_id = HirNodeId(9000);
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(value_id),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, value_id);
    module.blocks[entry_block_index].statements.push(statement);
}

/// Appends `expression_id` as an expression statement to the start function's entry block.
///
/// WHAT: records the statement and its value in the side table, then pushes the statement onto
/// the block.
/// WHY: module validation reaches expression rows only through block statements, so a bare row
/// would never be checked.
fn append_expression_statement(module: &mut HirModule, expression_id: HirValueId) {
    let entry_block_index = start_entry_block_index(module);
    let statement = HirStatement {
        id: HirNodeId(9004),
        kind: HirStatementKind::Expr(expression_id),
        span: None,
    };
    module.side_table.map_value(None, expression_id);
    module.side_table.map_statement(None, &statement);
    module.blocks[entry_block_index].statements.push(statement);
}

/// One exact `I32` value for HIR validation fixtures.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range")
}

/// One exact `F64` value for HIR validation fixtures.
fn f64_value(value: f64) -> FixedScalarValue {
    FixedScalarValue::binary_float(FixedScalar::F64, value)
        .expect("F64 literal is exactly representable")
}

fn int_expression(
    value: i64,
    type_id: TypeId,
    region: RegionId,
    span: &Option<SourceSpan>,
    module: &mut HirModule,
) -> HirValueId {
    let expression_id = append_hir_expression(
        module,
        HirExpressionKind::FixedScalar(i32_value(value)),
        type_id,
        region,
        ValueKind::RValue,
    );
    module.side_table.map_value(*span, expression_id);
    expression_id
}

fn float_expression(
    value: f64,
    type_id: TypeId,
    region: RegionId,
    span: &Option<SourceSpan>,
    module: &mut HirModule,
) -> HirValueId {
    let expression_id = append_hir_expression(
        module,
        HirExpressionKind::FixedScalar(f64_value(value)),
        type_id,
        region,
        ValueKind::RValue,
    );
    module.side_table.map_value(*span, expression_id);
    expression_id
}

fn add_validation_hir_struct(
    module: &mut HirModule,
    id: StructId,
    frontend_type_id: TypeId,
    fields: impl IntoIterator<Item = (FieldId, TypeId)>,
) {
    module.structs.push(HirStruct {
        id,
        frontend_type_id,
        fields: fields
            .into_iter()
            .map(|(id, ty)| HirField { id, ty })
            .collect(),
    });
}

fn append_place_load_for_validation(
    module: &mut HirModule,
    place: HirPlace,
    type_id: TypeId,
    span: Option<SourceSpan>,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_block = module.blocks[entry_block_index].id;
    let region = module.blocks[entry_block_index].region;
    let value = append_hir_expression(
        module,
        HirExpressionKind::Load(place),
        type_id,
        region,
        ValueKind::Place,
    );
    module.side_table.map_value(span, value);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(value),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block.0 as usize]
        .statements
        .push(statement);
}

#[test]
fn validator_accepts_field_projection_owned_by_current_nominal_type() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let mut path_fork = PathInternerFork::empty();
    let struct_path = super::symbol("Owner", &mut path_fork, &mut string_table);
    let field_path = super::symbol("value", &mut path_fork, &mut string_table);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let (_, owner_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path,
        fields: vec![FieldDefinition {
            name: field_path,
            type_id: int_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let field = FieldId(9000);
    add_validation_hir_struct(&mut module, StructId(9000), owner_type, [(field, int_type)]);

    let local = LocalId(9000);
    add_entry_parameter(&mut module, local, owner_type, false);
    let span = SourceSpan::new(SourceId::from_index(1), LocalSpan::source_start());
    let place = HirPlace::local(local)
        .with_field(field, &mut module.expressions, Some(span))
        .expect("the direct field projection should fit");
    append_place_load_for_validation(&mut module, place, int_type, Some(span));

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("a field owned by the current nominal type should pass HIR validation");
}

#[test]
fn validator_rejects_field_projection_owned_by_another_nominal_type() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let mut path_fork = PathInternerFork::empty();
    let owner_path = super::symbol("Owner", &mut path_fork, &mut string_table);
    let foreign_path = super::symbol("Foreign", &mut path_fork, &mut string_table);
    let owner_field_path = super::symbol("value", &mut path_fork, &mut string_table);
    let foreign_field_path = super::symbol("value", &mut path_fork, &mut string_table);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let (_, owner_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: owner_path,
        fields: vec![FieldDefinition {
            name: owner_field_path,
            type_id: int_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let (_, foreign_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: foreign_path,
        fields: vec![FieldDefinition {
            name: foreign_field_path,
            type_id: int_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let owner_field = FieldId(9000);
    let foreign_field = FieldId(9001);
    add_validation_hir_struct(
        &mut module,
        StructId(9000),
        owner_type,
        [(owner_field, int_type)],
    );
    add_validation_hir_struct(
        &mut module,
        StructId(9001),
        foreign_type,
        [(foreign_field, int_type)],
    );

    let local = LocalId(9000);
    add_entry_parameter(&mut module, local, owner_type, false);
    let span = SourceSpan::new(SourceId::from_index(1), LocalSpan::source_start());
    let place = HirPlace::local(local)
        .with_field(foreign_field, &mut module.expressions, Some(span))
        .expect("the foreign field projection should fit in malformed test HIR");
    append_place_load_for_validation(&mut module, place, int_type, Some(span));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("a same-typed field from another nominal type must be rejected");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert_eq!(error.source_span, Some(span));
}

#[test]
fn validator_rejects_field_projection_owned_by_another_generic_instance() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let mut path_fork = PathInternerFork::empty();
    let struct_path = super::symbol("Box", &mut path_fork, &mut string_table);
    let fixed_field_path = super::symbol("fixed", &mut path_fork, &mut string_table);
    let payload_field_path = super::symbol("payload", &mut path_fork, &mut string_table);
    let parameter_name = string_table.intern("T");
    let generic_parameters = type_environment
        .register_generic_parameter_list(
            [(TypeParameterId(0), parameter_name)].into_iter(),
            &Default::default(),
        )
        .list_id;
    let parameter_type =
        type_environment.intern_generic_parameter(GenericParameterId(0), parameter_name);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string_type = type_environment.builtins().string;
    let (nominal_id, _) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path,
        fields: vec![
            FieldDefinition {
                name: fixed_field_path,
                type_id: int_type,
                span: None,
            },
            FieldDefinition {
                name: payload_field_path,
                type_id: parameter_type,
                span: None,
            },
        ]
        .into_boxed_slice(),
        generic_parameters: Some(generic_parameters),
        const_record: false,
    });
    let int_instance = type_environment.intern_generic_instance(nominal_id, Box::new([int_type]));
    let string_instance =
        type_environment.intern_generic_instance(nominal_id, Box::new([string_type]));
    let int_fields = type_environment
        .fields_for(int_instance)
        .expect("the I32 instance should have substituted fields");
    let string_fields = type_environment
        .fields_for(string_instance)
        .expect("the String instance should have substituted fields");
    assert_eq!(int_fields[0].type_id, int_type);
    assert_eq!(string_fields[0].type_id, int_type);

    let int_fixed_field = FieldId(9000);
    let int_payload_field = FieldId(9001);
    let string_fixed_field = FieldId(9002);
    let string_payload_field = FieldId(9003);
    add_validation_hir_struct(
        &mut module,
        StructId(9000),
        int_instance,
        [
            (int_fixed_field, int_fields[0].type_id),
            (int_payload_field, int_fields[1].type_id),
        ],
    );
    add_validation_hir_struct(
        &mut module,
        StructId(9001),
        string_instance,
        [
            (string_fixed_field, string_fields[0].type_id),
            (string_payload_field, string_fields[1].type_id),
        ],
    );

    let local = LocalId(9000);
    add_entry_parameter(&mut module, local, int_instance, false);
    let span = SourceSpan::new(SourceId::from_index(1), LocalSpan::source_start());
    let place = HirPlace::local(local)
        .with_field(string_fixed_field, &mut module.expressions, Some(span))
        .expect("the foreign instance field projection should fit");
    append_place_load_for_validation(&mut module, place, int_type, Some(span));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("a field from Box<String> must not be accepted on Box<I32>");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert_eq!(error.source_span, Some(span));
}

#[test]
fn validator_resolves_field_and_index_projections_from_each_reached_type() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let mut path_fork = PathInternerFork::empty();
    let inner_path = super::symbol("Inner", &mut path_fork, &mut string_table);
    let inner_field_path = super::symbol("value", &mut path_fork, &mut string_table);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let (_, inner_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: inner_path,
        fields: vec![FieldDefinition {
            name: inner_field_path,
            type_id: int_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let item_collection_type = type_environment.intern_collection(inner_type, None);
    let outer_path = super::symbol("Outer", &mut path_fork, &mut string_table);
    let items_field_path = super::symbol("items", &mut path_fork, &mut string_table);
    let (_, outer_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: outer_path,
        fields: vec![FieldDefinition {
            name: items_field_path,
            type_id: item_collection_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let items_field = FieldId(9000);
    let value_field = FieldId(9001);
    add_validation_hir_struct(
        &mut module,
        StructId(9000),
        outer_type,
        [(items_field, item_collection_type)],
    );
    add_validation_hir_struct(
        &mut module,
        StructId(9001),
        inner_type,
        [(value_field, int_type)],
    );

    let local = LocalId(9000);
    let entry = add_entry_parameter(&mut module, local, outer_type, false);
    let region = module.blocks[entry.0 as usize].region;
    let index = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(0)),
        int_type,
        region,
        ValueKind::Const,
    );
    let place = HirPlace::local(local)
        .with_field(items_field, &mut module.expressions, None)
        .expect("the outer field projection should fit")
        .with_index(index, &mut module.expressions, None)
        .expect("the collection index projection should fit")
        .with_field(value_field, &mut module.expressions, None)
        .expect("the reached Inner field projection should fit");
    append_place_load_for_validation(&mut module, place, int_type, None);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("field and index projections should advance the base type at every step");
}

#[test]
fn validator_rejects_foreign_field_after_index_projection() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let mut path_fork = PathInternerFork::empty();
    let element_path = super::symbol("Element", &mut path_fork, &mut string_table);
    let foreign_path = super::symbol("Foreign", &mut path_fork, &mut string_table);
    let element_field_path = super::symbol("value", &mut path_fork, &mut string_table);
    let foreign_field_path = super::symbol("value", &mut path_fork, &mut string_table);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let (_, element_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: element_path,
        fields: vec![FieldDefinition {
            name: element_field_path,
            type_id: int_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let (_, foreign_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: foreign_path,
        fields: vec![FieldDefinition {
            name: foreign_field_path,
            type_id: int_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let collection_type = type_environment.intern_collection(element_type, None);
    let outer_path = super::symbol("Outer", &mut path_fork, &mut string_table);
    let items_field_path = super::symbol("items", &mut path_fork, &mut string_table);
    let (_, outer_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: outer_path,
        fields: vec![FieldDefinition {
            name: items_field_path,
            type_id: collection_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    let items_field = FieldId(9000);
    let element_field = FieldId(9001);
    let foreign_field = FieldId(9002);
    add_validation_hir_struct(
        &mut module,
        StructId(9000),
        outer_type,
        [(items_field, collection_type)],
    );
    add_validation_hir_struct(
        &mut module,
        StructId(9001),
        element_type,
        [(element_field, int_type)],
    );
    add_validation_hir_struct(
        &mut module,
        StructId(9002),
        foreign_type,
        [(foreign_field, int_type)],
    );

    let local = LocalId(9000);
    let entry = add_entry_parameter(&mut module, local, outer_type, false);
    let region = module.blocks[entry.0 as usize].region;
    let index = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(0)),
        int_type,
        region,
        ValueKind::Const,
    );
    let span = SourceSpan::new(SourceId::from_index(1), LocalSpan::source_start());
    let place = HirPlace::local(local)
        .with_field(items_field, &mut module.expressions, Some(span))
        .expect("the outer field projection should fit")
        .with_index(index, &mut module.expressions, Some(span))
        .expect("the collection index projection should fit")
        .with_field(foreign_field, &mut module.expressions, Some(span))
        .expect("the foreign field projection should fit");
    append_place_load_for_validation(&mut module, place, int_type, Some(span));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("an indexed Element place cannot use a field owned by Foreign");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert_eq!(error.source_span, Some(span));
}

#[test]
fn validator_rejects_place_classification_for_non_load_values() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let literal = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(1)),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        region,
        ValueKind::Place,
    );
    append_entry_statement(&mut module, entry, HirStatementKind::Expr(literal));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("a literal cannot carry Place value provenance");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("ValueKind::Place is only valid"));
}

#[test]
fn validator_accepts_rvalue_load_and_checks_load_copy_place_types() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    add_entry_parameter(
        &mut module,
        LocalId(9000),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        false,
    );
    let region = module.blocks[entry.0 as usize].region;
    let load = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(HirPlace::local(LocalId(9000))),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        region,
        ValueKind::RValue,
    );
    append_entry_statement(&mut module, entry, HirStatementKind::Expr(load));

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("an internal Load result can be classified as an RValue");

    for (kind, expected_message) in [
        (
            HirExpressionKind::Load(HirPlace::local(LocalId(9000))),
            "Load expression type does not match its place type",
        ),
        (
            HirExpressionKind::Copy(HirPlace::local(LocalId(9000))),
            "Copy expression type does not match its place type",
        ),
    ] {
        let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
        let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
        add_local(
            &mut module,
            entry,
            LocalId(9000),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
        );
        let region = module.blocks[entry.0 as usize].region;
        let expression = append_hir_expression(
            &mut module,
            kind,
            builtin_type_ids::BOOL,
            region,
            ValueKind::RValue,
        );
        append_entry_statement(&mut module, entry, HirStatementKind::Expr(expression));

        let error = validate_module_for_tests(
            &module,
            &string_table,
            &type_environment,
            &ExternalPackageRegistry::new(),
        )
        .expect_err("Load and Copy rows must retain their resolved place type");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
        assert!(error.msg.contains(expected_message));
    }
}

fn validate_numeric_op_for_test(
    op: HirNumericOp,
    operand_types: &[TypeId],
    result_type: TypeId,
) -> Result<(), CompilerError> {
    validate_numeric_op_for_test_with_types(op, |_| (operand_types.to_vec(), result_type))
}

fn validate_numeric_op_for_test_with_types(
    op: HirNumericOp,
    build_types: impl FnOnce(&mut TypeEnvironment) -> (Vec<TypeId>, TypeId),
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let (operand_types, result_type) = build_types(&mut type_environment);
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);
    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: result_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    let mut operands = Vec::with_capacity(operand_types.len());
    for (index, operand_type) in operand_types.iter().copied().enumerate() {
        let local_id = LocalId(9001 + index as u32);
        module.blocks[entry_block_index].locals.push(HirLocal {
            id: local_id,
            ty: operand_type,
            mutable: false,
            region: entry_region,
            span: None,
        });
        let function = module
            .start_function
            .expect("test module should have start");
        module.functions[function.0 as usize].params.push(local_id);

        let value_id = append_hir_expression(
            &mut module,
            HirExpressionKind::Load(HirPlace::local(local_id)),
            operand_type,
            entry_region,
            ValueKind::RValue,
        );
        module.side_table.map_value(span, value_id);
        operands.push(value_id);
    }

    let operands = match (op.operator.is_unary(), operands.as_slice()) {
        (true, [operand]) => HirNumericOperands::Unary { operand: *operand },
        (false, [left, right]) => HirNumericOperands::Binary {
            left: *left,
            right: *right,
        },
        _ => panic!("test NumericOp operand types must match operation arity"),
    };
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op,
            failure_mode: NumericFailureMode::Trap,
            operands,
            result: HirLocalDestination::Define(result_local),
        },
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
}

fn validate_number_numeric_op_for_test(
    operator: NumericOperator,
    scale: NumberScale,
    right_domain: Option<NumericScalar>,
) -> Result<(), CompilerError> {
    validate_numeric_op_for_test_with_types(
        HirNumericOp {
            operator,
            domain: NumericScalar::Number(scale),
        },
        |type_environment| {
            let number_type = type_environment.intern_number(scale);
            let right_type = right_domain.map_or(number_type, |domain| match domain {
                NumericScalar::Number(right_scale) => type_environment.intern_number(right_scale),
                _ => domain.type_id(type_environment),
            });
            let operand_types = if operator.is_unary() {
                vec![number_type]
            } else {
                vec![number_type, right_type]
            };
            (operand_types, number_type)
        },
    )
}

pub(super) fn builtin_error_type_id(type_environment: &mut TypeEnvironment) -> TypeId {
    let error_identity = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error);
    if let Some(error_type_id) = type_environment.type_id_for_canonical_identity(&error_identity) {
        return error_type_id;
    }

    let (_, error_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Vec::new().into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    type_environment
        .register_canonical_identity(error_identity, error_type_id)
        .expect("test builtin Error identity should register");
    error_type_id
}

fn validate_cast_op_for_test(
    policy: BuiltinCastPolicyId,
    source_type: TypeId,
    result_success_type: TypeId,
    result_error_override: Option<TypeId>,
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let builtin_error_type = builtin_error_type_id(&mut type_environment);
    install_cast_invoke_for_test(
        &mut module,
        policy,
        source_type,
        result_success_type,
        result_error_override.unwrap_or(builtin_error_type),
    );
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
}

fn install_cast_invoke_for_test(
    module: &mut HirModule,
    policy: BuiltinCastPolicyId,
    source_type: TypeId,
    success_type: TypeId,
    error_type: TypeId,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let source_local = LocalId(9001);
    let success_local = LocalId(9000);
    let error_local = LocalId(9002);
    add_entry_parameter(module, source_local, source_type, false);
    let source = append_hir_expression(
        module,
        HirExpressionKind::Load(HirPlace::local(source_local)),
        source_type,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(None, source);
    let success = append_failure_block(
        module,
        entry_region,
        vec![HirLocal {
            id: success_local,
            ty: success_type,
            mutable: false,
            region: entry_region,
            span: None,
        }],
    );
    let failure = append_failure_block(
        module,
        entry_region,
        vec![HirLocal {
            id: error_local,
            ty: error_type,
            mutable: false,
            region: entry_region,
            span: None,
        }],
    );
    module.blocks[entry_block_index].terminator = HirTerminator::Invoke {
        id: HirNodeId(9000),
        span: None,
        operation: HirFallibleOperation::CastOp { policy, source },
        success,
        success_results: vec![success_local],
        failure,
        error_result: error_local,
    };
}

fn numeric_scalar_type_for_test(
    type_environment: &mut TypeEnvironment,
    scalar: NumericScalar,
) -> TypeId {
    match scalar {
        NumericScalar::Number(scale) => type_environment.intern_number(scale),
        _ => scalar.type_id(type_environment),
    }
}

fn validate_number_cast_op_for_test(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let source_type = numeric_scalar_type_for_test(&mut type_environment, source_scalar);
    let target_type = numeric_scalar_type_for_test(&mut type_environment, target_scalar);
    let error_type = builtin_error_type_id(&mut type_environment);
    install_cast_invoke_for_test(
        &mut module,
        BuiltinCastPolicyId::NumericConversion {
            source: source_scalar,
            target: target_scalar,
        },
        source_type,
        target_type,
        error_type,
    );
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
}

fn fixed_type(scalar: FixedScalar) -> TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

fn validate_comparison_for_test(
    left_type: TypeId,
    right_type: TypeId,
    op: HirBinOp,
) -> Result<(), CompilerError> {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let left_local = LocalId(9000);
    let right_local = LocalId(9001);
    add_entry_parameter(&mut module, left_local, left_type, false);
    add_entry_parameter(&mut module, right_local, right_type, false);

    let left = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(HirPlace::local(left_local)),
        left_type,
        entry_region,
        ValueKind::RValue,
    );
    let right = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(HirPlace::local(right_local)),
        right_type,
        entry_region,
        ValueKind::RValue,
    );
    let expression = append_hir_expression(
        &mut module,
        HirExpressionKind::BinOp { op, left, right },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(span, left);
    module.side_table.map_value(span, right);
    module.side_table.map_value(span, expression);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
}

fn validate_number_comparison_for_test(
    integer_domain: NumericScalar,
    explicitly_converted: bool,
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let scale = NumberScale::new(2).expect("test Dec scale is valid");
    let number_type = type_environment.intern_number(scale);
    let integer_type = integer_domain.type_id(&type_environment);
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let number_local = LocalId(9000);
    let integer_local = LocalId(9001);
    add_entry_parameter(&mut module, number_local, number_type, false);
    add_entry_parameter(&mut module, integer_local, integer_type, false);

    let number = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(HirPlace::local(number_local)),
        number_type,
        entry_region,
        ValueKind::RValue,
    );
    let integer = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(HirPlace::local(integer_local)),
        integer_type,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(span, number);
    module.side_table.map_value(span, integer);
    let right = if explicitly_converted {
        let converted = append_hir_expression(
            &mut module,
            HirExpressionKind::Cast {
                source: integer,
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: integer_domain,
                    target: NumericScalar::Number(scale),
                },
            },
            number_type,
            entry_region,
            ValueKind::RValue,
        );
        module.side_table.map_value(span, converted);
        converted
    } else {
        integer
    };
    let expression = append_hir_expression(
        &mut module,
        HirExpressionKind::BinOp {
            op: HirBinOp::Lt,
            left: number,
            right,
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(span, expression);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
}

#[test]
fn validator_rejects_numeric_ops_with_invalid_fixed_domains() {
    let f32_type = fixed_type(FixedScalar::F32);
    let u32_type = fixed_type(FixedScalar::U32);
    let cases = [
        (
            HirNumericOp {
                operator: NumericOperator::IntegerDivide,
                domain: NumericScalar::Fixed(FixedScalar::F32),
            },
            vec![f32_type, f32_type],
            f32_type,
        ),
        (
            HirNumericOp {
                operator: NumericOperator::Divide,
                domain: NumericScalar::Fixed(FixedScalar::U32),
            },
            vec![u32_type, u32_type],
            u32_type,
        ),
        (
            HirNumericOp {
                operator: NumericOperator::Negate,
                domain: NumericScalar::Fixed(FixedScalar::U32),
            },
            vec![u32_type],
            u32_type,
        ),
    ];

    for (op, operand_types, result_type) in cases {
        let error = validate_numeric_op_for_test(op, &operand_types, result_type)
            .expect_err("validator should reject NumericOp domains unsupported by the operator");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
    }
}

#[test]
fn validator_rejects_fixed_numeric_op_operand_type_mismatch() {
    let u8_type = fixed_type(FixedScalar::U8);
    let u32_type = fixed_type(FixedScalar::U32);
    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[u8_type, u32_type],
        u32_type,
    )
    .expect_err("validator should reject operands not converted to the NumericOp domain");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_accepts_valid_fixed_numeric_ops() {
    let u32_type = fixed_type(FixedScalar::U32);
    let i32_type = fixed_type(FixedScalar::I32);

    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[u32_type, u32_type],
        u32_type,
    )
    .expect("valid fixed unsigned addition should pass HIR validation");
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I32),
        },
        &[i32_type],
        i32_type,
    )
    .expect("valid fixed signed negation should pass HIR validation");
    let f64_type = fixed_type(FixedScalar::F64);
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Fixed(FixedScalar::F64),
        },
        &[f64_type, f64_type],
        f64_type,
    )
    .expect("valid fixed binary-float division should pass HIR validation");
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::IntegerDivide,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[u32_type, u32_type],
        u32_type,
    )
    .expect("valid fixed integer division should pass HIR validation");
    let i8_type = fixed_type(FixedScalar::I8);
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I8),
        },
        &[i8_type],
        i8_type,
    )
    .expect("narrow signed negation keeps its own domain and should pass HIR validation");
}

#[test]
fn validator_accepts_uint_numeric_ops_and_rejects_mixed_or_negated_pairs() {
    // U32 arithmetic validates in its own domain; a NumericOp whose operands are not
    // converted to its domain and U32 negation are never emitted by lowering, so both
    // are rejected here.
    let uint_type = builtin_type_ids::fixed_scalar(FixedScalar::U32);
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    for operator in [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::IntegerDivide,
        NumericOperator::Remainder,
        NumericOperator::Power,
    ] {
        validate_numeric_op_for_test(
            HirNumericOp {
                operator,
                domain: NumericScalar::Fixed(FixedScalar::U32),
            },
            &[uint_type, uint_type],
            uint_type,
        )
        .expect("valid U32 arithmetic should pass HIR validation");
    }

    // U32 `/` computes in F64, so a `/` NumericOp in the U32 domain itself
    // is rejected: lowering must convert the operands before emitting it.
    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[uint_type, uint_type],
        uint_type,
    )
    .expect_err("U32 division must leave the U32 domain before NumericOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[uint_type, int_type],
        uint_type,
    )
    .expect_err("mixed U32/I32 operands must be converted before NumericOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[uint_type],
        uint_type,
    )
    .expect_err("U32 negation has no operator domain");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_accepts_uint_cast_pairs_and_comparisons() {
    // U32 participates in fallible CastOp narrowing and in exact
    // U32/U32 and U32/I32 comparisons without any conversion failure.
    validate_number_cast_op_for_test(
        NumericScalar::Fixed(FixedScalar::U32),
        NumericScalar::Fixed(FixedScalar::I32),
    )
    .expect("U32-to-I32 narrowing should use a fallible CastOp");
    validate_number_cast_op_for_test(
        NumericScalar::Fixed(FixedScalar::I32),
        NumericScalar::Fixed(FixedScalar::U32),
    )
    .expect("I32-to-U32 narrowing should use a fallible CastOp");

    validate_comparison_for_test(
        builtin_type_ids::fixed_scalar(FixedScalar::U32),
        builtin_type_ids::fixed_scalar(FixedScalar::U32),
        HirBinOp::Lt,
    )
    .expect("exact U32 comparison should remain valid");
    validate_comparison_for_test(
        builtin_type_ids::fixed_scalar(FixedScalar::U32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        HirBinOp::Gt,
    )
    .expect("exact U32/I32 comparison should remain valid");
}

#[test]
fn validator_accepts_number_operator_scale_rules_and_int_power_exponent() {
    let scale_two = NumberScale::new(2).expect("test Dec scale is valid");

    for operator in [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::Divide,
        NumericOperator::Remainder,
    ] {
        validate_number_numeric_op_for_test(operator, scale_two, None)
            .expect("valid Dec2 arithmetic should satisfy HIR validation");
    }

    validate_number_numeric_op_for_test(NumericOperator::IntegerDivide, NumberScale::ZERO, None)
        .expect("Dec0 integer division should be valid");
    validate_number_numeric_op_for_test(NumericOperator::Negate, scale_two, None)
        .expect("Dec negation should be valid");
    validate_number_numeric_op_for_test(
        NumericOperator::Power,
        scale_two,
        Some(NumericScalar::Fixed(FixedScalar::I32)),
    )
    .expect("Dec power with an I32 exponent should be valid");
}

#[test]
fn validator_rejects_number_operator_scale_and_operand_mismatches() {
    let scale_two = NumberScale::new(2).expect("test Dec scale is valid");
    let scale_three = NumberScale::new(3).expect("test Dec scale is valid");

    for (operator, scale) in [
        (NumericOperator::Divide, NumberScale::ZERO),
        (NumericOperator::IntegerDivide, scale_two),
    ] {
        let error = validate_number_numeric_op_for_test(operator, scale, None)
            .expect_err("invalid Dec scale/operator pair should be rejected");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
    }

    let error = validate_number_numeric_op_for_test(
        NumericOperator::Power,
        scale_two,
        Some(NumericScalar::Number(scale_two)),
    )
    .expect_err("Dec power must not scale its exponent to Dec");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_number_numeric_op_for_test(
        NumericOperator::Add,
        scale_two,
        Some(NumericScalar::Fixed(FixedScalar::I32)),
    )
    .expect_err("mixed integer operands must be explicitly converted before NumericOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_number_numeric_op_for_test(
        NumericOperator::Add,
        scale_two,
        Some(NumericScalar::Number(scale_three)),
    )
    .expect_err("NumericOp operands must share the result scale");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_unregistered_number_domain_without_panicking() {
    let scale = NumberScale::new(7).expect("test Dec scale is valid");
    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Number(scale),
        },
        &[
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    )
    .expect_err("an unregistered Dec scale cannot form a valid HIR numeric domain");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_distinguishes_fallible_number_cast_pairs() {
    let scale_two = NumberScale::new(2).expect("test Dec scale is valid");
    let scale_four = NumberScale::new(4).expect("test Dec scale is valid");

    validate_number_cast_op_for_test(
        NumericScalar::Number(scale_four),
        NumericScalar::Number(scale_two),
    )
    .expect("Dec scale narrowing should use a fallible CastOp");
    validate_number_cast_op_for_test(
        NumericScalar::Number(scale_two),
        NumericScalar::Fixed(FixedScalar::I32),
    )
    .expect("Dec-to-I32 should use a fallible CastOp");

    let error = validate_number_cast_op_for_test(
        NumericScalar::Number(scale_two),
        NumericScalar::Number(scale_four),
    )
    .expect_err("an infallible Dec widening must not be represented by CastOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    for (source, target) in [
        (
            NumericScalar::Number(scale_two),
            NumericScalar::Number(scale_two),
        ),
        (
            NumericScalar::Number(scale_two),
            NumericScalar::Fixed(FixedScalar::F64),
        ),
        (
            NumericScalar::Number(scale_two),
            NumericScalar::Fixed(FixedScalar::Byte),
        ),
        (
            NumericScalar::Fixed(FixedScalar::F64),
            NumericScalar::Number(scale_two),
        ),
    ] {
        let error = validate_number_cast_op_for_test(source, target)
            .expect_err("unsupported Dec cast pair must have no HIR policy row");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
    }
}

#[test]
fn validator_rejects_numeric_conversion_cast_source_policy_mismatch() {
    let error = validate_cast_op_for_test(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::I32),
        },
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
    )
    .expect_err("validator should reject a CastOp source that disagrees with its policy");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_numeric_conversion_cast_success_slot_type_mismatch() {
    let error = validate_cast_op_for_test(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::I32),
            target: NumericScalar::Fixed(FixedScalar::U8),
        },
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        fixed_type(FixedScalar::I32),
        None,
    )
    .expect_err("validator should reject a CastOp success slot with the wrong type");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_numeric_conversion_cast_error_slot_type_mismatch() {
    let error = validate_cast_op_for_test(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::I32),
            target: NumericScalar::Fixed(FixedScalar::U8),
        },
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        Some(builtin_type_ids::STRING),
    )
    .expect_err("validator should reject a CastOp with a non-builtin error slot");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_uses_numeric_comparison_compatibility_and_keeps_same_type_other_values() {
    let i64_type = fixed_type(FixedScalar::I64);
    let u64_type = fixed_type(FixedScalar::U64);
    validate_comparison_for_test(i64_type, u64_type, HirBinOp::Lt)
        .expect("exact mixed signed/unsigned comparison should remain valid");

    let byte_type = fixed_type(FixedScalar::Byte);
    validate_comparison_for_test(byte_type, byte_type, HirBinOp::Eq)
        .expect("same-type Byte comparison should remain valid");

    let f64_type = fixed_type(FixedScalar::F64);
    let error = validate_comparison_for_test(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        f64_type,
        HirBinOp::Eq,
    )
    .expect_err("incompatible numeric comparison should be rejected");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_requires_explicit_integer_to_number_comparison_conversion() {
    validate_number_comparison_for_test(NumericScalar::Fixed(FixedScalar::I32), true)
        .expect("explicit I32-to-Dec comparison conversion should pass");
    validate_number_comparison_for_test(NumericScalar::Fixed(FixedScalar::U64), true)
        .expect("explicit fixed-integer-to-Dec comparison conversion should pass");

    let error = validate_number_comparison_for_test(NumericScalar::Fixed(FixedScalar::I32), false)
        .expect_err("mixed Dec and I32 comparison must be converted in HIR");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn valid_module_passes_explicit_validation() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("validator should accept a valid lowered module");
}

#[test]
fn validator_rejects_assertion_message_evaluation_fact_mismatch() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let message_span = None;
    let option_string = type_environment.intern_option(builtin_type_ids::STRING);
    let empty_fields = module
        .expressions
        .append_variant_fields(&[], message_span)
        .expect("empty assertion payload fields should fit the expression store");
    let message = append_hir_expression(
        &mut module,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 0,
            fields: empty_fields,
        },
        option_string,
        entry_region,
        ValueKind::RValue,
    );

    module.side_table.map_value(message_span, message);
    module.blocks[entry_block_index].terminator = HirTerminator::AssertFailure {
        message,
        message_evaluation: HirAssertionMessageEvaluation::Folded,
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a stale assertion message evaluation fact");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("evaluation fact"),
        "expected evaluation-fact mismatch, got: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_numeric_op_operand_shape_mismatch() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: int_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    let left = int_expression(1, int_type, entry_region, &span, &mut module);
    let right = int_expression(2, int_type, entry_region, &span, &mut module);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::Negate,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: HirLocalDestination::Define(result_local),
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject mismatched NumericOp arity");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("operand shape does not match the operation arity")
    );
}

#[test]
fn validator_rejects_numeric_op_operand_domain_mismatch() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let float_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: int_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    // I32-domain addition fed F64 operands: lowering must convert explicitly, so a
    // mismatched operand type is a builder bug the validator has to catch.
    let left = float_expression(1.0, float_type, entry_region, &span, &mut module);
    let right = float_expression(2.0, float_type, entry_region, &span, &mut module);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::Add,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: HirLocalDestination::Define(result_local),
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject NumericOp operands outside the domain type");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_integer_divide_on_float_domain() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let float_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: float_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    let left = float_expression(1.0, float_type, entry_region, &span, &mut module);
    let right = float_expression(2.0, float_type, entry_region, &span, &mut module);

    // Truncating `//` needs an integer domain; real `/` is the only division on F64.
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::IntegerDivide,
                domain: NumericScalar::Fixed(FixedScalar::F64),
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: HirLocalDestination::Define(result_local),
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject IntegerDivide on an F64 domain");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("operator does not match the domain"));
}

fn append_expression_for_validation(
    module: &mut HirModule,
    span: &Option<SourceSpan>,
    left_type: TypeId,
    result_type: TypeId,
    int_type: TypeId,
) -> HirValueId {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let left = append_hir_expression(
        module,
        HirExpressionKind::StringLiteral("prefix".to_owned()),
        left_type,
        entry_region,
        ValueKind::Const,
    );
    module.side_table.map_value(*span, left);

    let right = int_expression(7, int_type, entry_region, span, module);
    let expression = append_hir_expression(
        module,
        HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            left,
            right,
        },
        result_type,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(*span, expression);
    expression
}

#[test]
fn validator_accepts_internal_string_append_with_scalar_chunk() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;
    let expression = append_expression_for_validation(
        &mut module,
        &span,
        string_type,
        string_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let statement = HirStatement {
        id: HirNodeId(9013),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("valid StringAppend should pass HIR validation");
}

#[test]
fn validator_rejects_string_append_with_non_string_result() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;
    let expression = append_expression_for_validation(
        &mut module,
        &span,
        string_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let statement = HirStatement {
        id: HirNodeId(9014),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("StringAppend with a non-String result should fail validation");
    assert!(
        error
            .msg
            .contains("StringAppend must produce a String from a String accumulator")
    );
}

#[test]
fn validator_rejects_string_append_with_non_string_accumulator() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let expression = append_expression_for_validation(
        &mut module,
        &span,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        type_environment.builtins().string,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let statement = HirStatement {
        id: HirNodeId(9015),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("StringAppend with a non-String accumulator should fail validation");
    assert!(
        error
            .msg
            .contains("StringAppend must produce a String from a String accumulator")
    );
}

#[test]
fn validator_rejects_plain_numeric_unary_op() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let operand = int_expression(1, int_type, entry_region, &span, &mut module);

    let expression = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Neg,
            operand,
        },
        int_type,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(span, expression);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject plain numeric UnaryOp");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Plain HirUnaryOp::Neg must be lowered through HirStatementKind::NumericOp")
    );
}

fn inject_float_statement(
    module: &mut HirModule,
    span: &Option<SourceSpan>,
    kind: HirStatementKind,
    source_type: TypeId,
    result_type: TypeId,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let source = float_expression(1.5, source_type, entry_region, span, module);
    inject_float_statement_with_source(module, span, kind, source, result_type);
}

/// Emits a `FormatFloat`/`ValidateFloat` statement over an already-built source expression.
///
/// WHY: negative fixtures need a source literal that is valid for its own non-float type, so the
///      statement's own type guard rejects it instead of an ill-typed literal.
fn inject_float_statement_with_source(
    module: &mut HirModule,
    span: &Option<SourceSpan>,
    kind: HirStatementKind,
    source: HirValueId,
    result_type: TypeId,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);

    {
        let entry_block = &mut module.blocks[entry_block_index];
        entry_block.locals.push(HirLocal {
            id: result_local,
            ty: result_type,
            mutable: false,
            region: entry_region,
            span: None,
        });
    }

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: match kind {
            HirStatementKind::FormatFloat { failure_mode, .. } => HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result: HirLocalDestination::Define(result_local),
            },
            HirStatementKind::ValidateFloat { failure_mode, .. } => {
                HirStatementKind::ValidateFloat {
                    source,
                    failure_mode,
                    result: HirLocalDestination::Define(result_local),
                }
            }
            _ => panic!("inject_float_statement only supports FormatFloat and ValidateFloat"),
        },
        span: *span,
    };

    module.side_table.map_statement(*span, &statement);
    module.blocks[entry_block_index].statements.push(statement);
}

/// Builds a mapped fixed binary-float literal row for `scalar`/`type_id`.
fn fixed_float_source(
    module: &mut HirModule,
    span: &Option<SourceSpan>,
    scalar: FixedScalar,
    type_id: TypeId,
) -> HirValueId {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let value = FixedScalarValue::binary_float(scalar, 1.5)
        .expect("1.5 is exactly representable at every supported binary-float precision");
    let expression_id = append_hir_expression(
        module,
        HirExpressionKind::FixedScalar(value),
        type_id,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(*span, expression_id);
    expression_id
}

#[test]
fn validator_accepts_format_float_trap() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;

    inject_float_statement(
        &mut module,
        &span,
        HirStatementKind::FormatFloat {
            source: HirValueId(0),
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        string_type,
    );

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("validator should accept FormatFloat with Trap and String result local");
}

#[test]
fn validator_accepts_validate_float_trap() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let float_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);

    inject_float_statement(
        &mut module,
        &span,
        HirStatementKind::ValidateFloat {
            source: HirValueId(0),
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        float_type,
        float_type,
    );

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("validator should accept ValidateFloat with Trap and F64 result local");
}

#[test]
fn validator_accepts_validate_float_with_exact_fixed_f64_type() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let f64_type = fixed_type(FixedScalar::F64);
    let source = fixed_float_source(&mut module, &span, FixedScalar::F64, f64_type);

    inject_float_statement_with_source(
        &mut module,
        &span,
        HirStatementKind::ValidateFloat {
            source: HirValueId(0),
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        source,
        f64_type,
    );

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("validator should accept ValidateFloat with an exact fixed F64 source and result");
}

#[test]
fn validator_rejects_validate_float_result_type_mismatch() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let f64_type = fixed_type(FixedScalar::F64);
    let source = fixed_float_source(&mut module, &span, FixedScalar::F64, f64_type);

    inject_float_statement_with_source(
        &mut module,
        &span,
        HirStatementKind::ValidateFloat {
            source: HirValueId(0),
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        source,
        // A narrower binary-float identity must not erode the validated exact type.
        fixed_type(FixedScalar::F32),
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a result local that erodes the validated type");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_validate_float_with_non_float_source() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let i32_type = fixed_type(FixedScalar::I32);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let i32_value = FixedScalarValue::signed(FixedScalar::I32, 41).expect("41 fits in I32");
    let source = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value),
        i32_type,
        entry_region,
        ValueKind::RValue,
    );
    module.side_table.map_value(span, source);

    inject_float_statement_with_source(
        &mut module,
        &span,
        HirStatementKind::ValidateFloat {
            source: HirValueId(0),
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        source,
        i32_type,
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a non-float ValidateFloat source");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_format_float_trap_with_non_string_result() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let float_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);

    inject_float_statement(
        &mut module,
        &span,
        HirStatementKind::FormatFloat {
            source: HirValueId(0),
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        float_type,
        float_type,
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject FormatFloat Trap with non-String result local");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("FormatFloat Trap result local has the wrong success type")
    );
}

#[test]
fn validator_rejects_discharged_mode_on_float_and_range_step_statements() {
    let dummy_source = || HirValueId(0);
    // (statement kind under test, expected rejection): the `inject_float_statement`
    // fixture reads the failure mode off the passed kind, so each case reuses it;
    // `RangeStepFailure` needs its own injection below.
    let float_cases = [
        (
            HirStatementKind::FormatFloat {
                source: dummy_source(),
                failure_mode: NumericFailureMode::Infallible,
                result: HirLocalDestination::Define(LocalId(0)),
            },
            "FormatFloat cannot use the discharged numeric mode",
        ),
        (
            HirStatementKind::ValidateFloat {
                source: dummy_source(),
                failure_mode: NumericFailureMode::Infallible,
                result: HirLocalDestination::Define(LocalId(0)),
            },
            "ValidateFloat cannot use the discharged numeric mode",
        ),
    ];

    for (kind, expected_message) in float_cases {
        let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
        let span = None;
        let float_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);
        let success_type = match &kind {
            HirStatementKind::FormatFloat { .. } => type_environment.builtins().string,
            _ => float_type,
        };
        inject_float_statement(&mut module, &span, kind, float_type, success_type);

        let error = validate_module_for_tests(
            &module,
            &string_table,
            &type_environment,
            &ExternalPackageRegistry::new(),
        )
        .expect_err("validator should reject Infallible on float statements");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
        assert!(
            error.msg.contains(expected_message),
            "unexpected rejection message: {}",
            error.msg
        );
    }

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);
    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: type_environment.builtins().bool,
        mutable: false,
        region: entry_region,
        span: None,
    });
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::RangeStepFailure {
            cause: RangeStepFailureCause::ZeroStep,
            failure_mode: NumericFailureMode::Infallible,
            result: HirLocalDestination::Define(result_local),
        },
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject Infallible on RangeStepFailure");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("RangeStepFailure cannot use the discharged numeric mode"),
        "unexpected rejection message: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_invalid_jump_target() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let entry_block = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    module.blocks[entry_block.0 as usize].terminator = HirTerminator::Jump {
        target: crate::compiler_frontend::hir::ids::BlockId(999),
        args: vec![],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject invalid jump target");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unknown HIR block id"));
}

#[test]
fn validator_accepts_typed_jump_definition_from_local_source() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    add_entry_parameter(
        &mut module,
        LocalId(9000),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        false,
    );
    add_local(
        &mut module,
        entry,
        LocalId(9001),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    install_entry_self_jump(
        &mut module,
        vec![HirJumpArgument {
            source: LocalId(9000),
            destination: LocalId(9001),
        }],
    );

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("edge should define its target local from a same-typed source");
}

#[test]
fn validator_rejects_jump_with_unknown_source_or_destination_local() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    add_local(
        &mut module,
        entry,
        LocalId(9001),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    install_entry_self_jump(
        &mut module,
        vec![HirJumpArgument {
            source: LocalId(9990),
            destination: LocalId(9001),
        }],
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("edge source must name a registered local");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unknown HIR local id"));

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    add_local(
        &mut module,
        entry,
        LocalId(9000),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    install_entry_self_jump(
        &mut module,
        vec![HirJumpArgument {
            source: LocalId(9000),
            destination: LocalId(9991),
        }],
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("edge destination must name a registered local");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unknown HIR local id"));
}

#[test]
fn validator_rejects_mismatched_and_duplicate_jump_destinations() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    add_local(
        &mut module,
        entry,
        LocalId(9000),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    add_local(
        &mut module,
        entry,
        LocalId(9001),
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
    );
    install_entry_self_jump(
        &mut module,
        vec![HirJumpArgument {
            source: LocalId(9000),
            destination: LocalId(9001),
        }],
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("edge source and destination types must match");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("have different types"));

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    for id in [LocalId(9000), LocalId(9001), LocalId(9002)] {
        add_local(
            &mut module,
            entry,
            id,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
        );
    }
    install_entry_self_jump(
        &mut module,
        vec![
            HirJumpArgument {
                source: LocalId(9000),
                destination: LocalId(9002),
            },
            HirJumpArgument {
                source: LocalId(9001),
                destination: LocalId(9002),
            },
        ],
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("one edge cannot define its target local more than once");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("defines destination"));
}

#[test]
fn validator_requires_jump_destinations_to_belong_to_the_target_block() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    add_local(
        &mut module,
        entry,
        LocalId(9000),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    add_local(
        &mut module,
        entry,
        LocalId(9001),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let target = append_failure_block(&mut module, region, vec![]);
    module.blocks[entry.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![HirJumpArgument {
            source: LocalId(9000),
            destination: LocalId(9001),
        }],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("edge destination must be declared in the target block");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("not a local owned by target block"));
}

#[test]
fn validator_requires_all_incoming_edges_to_agree_on_destination_set_and_arity() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let source = LocalId(9000);
    let destination = LocalId(9001);
    add_local(
        &mut module,
        entry,
        source,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let target = append_failure_block(
        &mut module,
        region,
        vec![HirLocal {
            id: destination,
            ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            mutable: false,
            region,
            span: None,
        }],
    );
    let jump_source = append_failure_block(&mut module, region, vec![]);
    let condition = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        region,
        ValueKind::Const,
    );
    module.blocks[entry.0 as usize].terminator = HirTerminator::If {
        condition,
        then_block: target,
        else_block: jump_source,
    };
    module.blocks[jump_source.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![HirJumpArgument {
            source,
            destination,
        }],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("an If edge with no transfers must agree with its Jump predecessor");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("disagree on explicit destination set or arity")
    );

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let left_source = LocalId(9000);
    let right_source = LocalId(9001);
    let left_destination = LocalId(9002);
    let right_destination = LocalId(9003);
    add_local(
        &mut module,
        entry,
        left_source,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    add_local(
        &mut module,
        entry,
        right_source,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let target = append_failure_block(
        &mut module,
        region,
        vec![
            HirLocal {
                id: left_destination,
                ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                mutable: false,
                region,
                span: None,
            },
            HirLocal {
                id: right_destination,
                ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                mutable: false,
                region,
                span: None,
            },
        ],
    );
    let left_predecessor = append_failure_block(&mut module, region, vec![]);
    let right_predecessor = append_failure_block(&mut module, region, vec![]);
    let condition = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        region,
        ValueKind::Const,
    );
    module.blocks[entry.0 as usize].terminator = HirTerminator::If {
        condition,
        then_block: left_predecessor,
        else_block: right_predecessor,
    };
    module.blocks[left_predecessor.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![HirJumpArgument {
            source: left_source,
            destination: left_destination,
        }],
    };
    module.blocks[right_predecessor.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![HirJumpArgument {
            source: right_source,
            destination: right_destination,
        }],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("equal-arity predecessors must define the same destination locals");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("disagree on explicit destination set or arity")
    );
}

#[test]
fn validator_accepts_reordered_incoming_jump_destination_pairs() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let first_source = LocalId(9000);
    let second_source = LocalId(9001);
    let first_destination = LocalId(9002);
    let second_destination = LocalId(9003);
    add_entry_parameter(
        &mut module,
        first_source,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        false,
    );
    add_entry_parameter(
        &mut module,
        second_source,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        false,
    );
    let target = append_failure_block(
        &mut module,
        region,
        vec![
            HirLocal {
                id: first_destination,
                ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                mutable: false,
                region,
                span: None,
            },
            HirLocal {
                id: second_destination,
                ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                mutable: false,
                region,
                span: None,
            },
        ],
    );
    let first_predecessor = append_failure_block(&mut module, region, vec![]);
    let second_predecessor = append_failure_block(&mut module, region, vec![]);
    let condition = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        region,
        ValueKind::Const,
    );
    module.blocks[entry.0 as usize].terminator = HirTerminator::If {
        condition,
        then_block: first_predecessor,
        else_block: second_predecessor,
    };
    module.blocks[first_predecessor.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![
            HirJumpArgument {
                source: first_source,
                destination: first_destination,
            },
            HirJumpArgument {
                source: second_source,
                destination: second_destination,
            },
        ],
    };
    module.blocks[second_predecessor.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![
            HirJumpArgument {
                source: second_source,
                destination: second_destination,
            },
            HirJumpArgument {
                source: first_source,
                destination: first_destination,
            },
        ],
    };

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("explicit source/destination pairs may be reordered across predecessors");
}

#[test]
fn validator_rejects_jump_sources_owned_by_another_function() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let helper_name = super::symbol("helper", &mut path_fork, &mut string_table);
    let helper = function_node(
        helper_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );
    let ast = build_ast_with_registered_types(
        vec![helper, start],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let start_entry = module.functions[module.start_function.unwrap().0 as usize].entry;
    let helper_entry = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .expect("helper function should exist")
        .entry;
    add_local(
        &mut module,
        start_entry,
        LocalId(9000),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    add_local(
        &mut module,
        helper_entry,
        LocalId(9001),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    module.blocks[start_entry.0 as usize].terminator = HirTerminator::Jump {
        target: start_entry,
        args: vec![HirJumpArgument {
            source: LocalId(9001),
            destination: LocalId(9000),
        }],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("edge source must belong to the current function");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("belongs to function"));
}

#[test]
fn validator_requires_parameters_to_be_defined_in_the_entry_block() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let start_index = module.start_function.unwrap().0 as usize;
    let entry = module.functions[start_index].entry;
    let region = module.blocks[entry.0 as usize].region;
    let non_entry_local = HirLocal {
        id: LocalId(9000),
        ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        mutable: false,
        region,
        span: None,
    };
    let target = append_failure_block(&mut module, region, vec![non_entry_local]);
    module.blocks[entry.0 as usize].terminator = HirTerminator::Jump {
        target,
        args: vec![],
    };
    module.functions[start_index].params.push(LocalId(9000));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("function parameters must be defined in the function entry block");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("not defined in its entry block"));
}

#[test]
fn validator_rejects_parameter_definition_destinations_but_accepts_updates() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let parameter = LocalId(9000);
    let entry = add_entry_parameter(
        &mut module,
        parameter,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        false,
    );
    let region = module.blocks[entry.0 as usize].region;
    let value = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(1)),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        region,
        ValueKind::Const,
    );
    append_entry_statement(
        &mut module,
        entry,
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(parameter),
            value,
        },
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("ordinary Write cannot redefine a function ABI parameter");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("ABI parameter local"));

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let parameter = LocalId(9000);
    let entry = add_entry_parameter(&mut module, parameter, builtin_type_ids::BOOL, false);
    append_entry_statement(
        &mut module,
        entry,
        HirStatementKind::RangeStepFailure {
            cause: RangeStepFailureCause::ZeroStep,
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(parameter),
        },
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("dedicated operation cannot define over a function ABI parameter");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("ABI parameter local"));

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let parameter = LocalId(9000);
    let entry = add_entry_parameter(&mut module, parameter, builtin_type_ids::BOOL, true);
    append_entry_statement(
        &mut module,
        entry,
        HirStatementKind::RangeStepFailure {
            cause: RangeStepFailureCause::ZeroStep,
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Update(parameter),
        },
    );

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("a dedicated operation may update a parameter result local");

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let parameter = LocalId(9000);
    add_entry_parameter(
        &mut module,
        parameter,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        false,
    );
    install_entry_self_jump(
        &mut module,
        vec![HirJumpArgument {
            source: parameter,
            destination: parameter,
        }],
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("a CFG edge cannot define a function ABI parameter");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("ABI parameter local"));

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let parameter = LocalId(9000);
    let entry = add_entry_parameter(
        &mut module,
        parameter,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        true,
    );
    let region = module.blocks[entry.0 as usize].region;
    let value = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(1)),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        region,
        ValueKind::Const,
    );
    append_entry_statement(
        &mut module,
        entry,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(HirPlace::local(parameter)),
            value,
        },
    );

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("ordinary place assignment may update an ABI parameter");
}

#[test]
fn validator_rejects_non_literal_match_pattern() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block_id = start.entry;
    let local_id = start.params[0];
    let local_ty = module.blocks[entry_block_id.0 as usize]
        .locals
        .iter()
        .find(|local| local.id == local_id)
        .expect("parameter local should be present in the entry block")
        .ty;
    let region = module.blocks[entry_block_id.0 as usize].region;
    let value_span = None;
    let scrutinee = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(1)),
        local_ty,
        region,
        ValueKind::Const,
    );
    let pattern_value = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(HirPlace::local(local_id)),
        local_ty,
        region,
        ValueKind::Place,
    );
    module.side_table.map_value(value_span, scrutinee);
    module.side_table.map_value(value_span, pattern_value);

    module.blocks[entry_block_id.0 as usize].terminator = HirTerminator::Match {
        scrutinee,
        arms: vec![HirMatchArm {
            pattern: HirPattern::Literal(pattern_value),
            guard: None,
            body: entry_block_id,
        }],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject non-literal match pattern");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Match literal pattern"));
}

#[test]
fn validator_rechecks_shared_expression_in_match_pattern_context() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_block_id = module.blocks[entry_block_index].id;
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let bool_type = type_environment.builtins().bool;
    let value_span = SourceSpan::new(SourceId::from_index(1), LocalSpan::source_start());
    let match_span = SourceSpan::new(SourceId::from_index(2), LocalSpan::source_start());
    assert_ne!(value_span, match_span);

    let shared_value = module
        .expressions
        .append_expression(HirExpression {
            kind: HirExpressionKind::FixedScalar(i32_value(1)),
            ty: int_type,
            value_kind: ValueKind::Const,
            region: entry_region,
            span: Some(value_span),
        })
        .expect("shared literal expression should fit");
    module.side_table.map_value(Some(value_span), shared_value);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(shared_value),
        span: None,
    };
    module.blocks[entry_block_index].statements.push(statement);
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("the shared literal should validate as an ordinary expression");

    let scrutinee = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        bool_type,
        entry_region,
        ValueKind::Const,
    );
    module.side_table.map_value(None, scrutinee);
    module.blocks[entry_block_index].terminator = HirTerminator::Match {
        scrutinee,
        arms: vec![HirMatchArm {
            pattern: HirPattern::Literal(shared_value),
            guard: None,
            body: entry_block_id,
        }],
    };
    module
        .side_table
        .map_terminator(Some(match_span), entry_block_id);
    module
        .side_table
        .map_terminator_span(entry_block_id, match_span);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("the shared literal must be checked against its match subject type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Match pattern value type does not match the direct scrutinee type")
    );
    assert_eq!(error.source_span, Some(match_span));
}

fn inject_fixed_scalar_match_pattern(
    module: &mut HirModule,
    scrutinee_type_id: TypeId,
    pattern_type_id: TypeId,
    pattern_value: FixedScalarValue,
    build_pattern: impl FnOnce(HirValueId) -> HirPattern,
) {
    let start_function = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block_id = start_function.entry;
    let entry_block_index = entry_block_id.0 as usize;
    let region = module.blocks[entry_block_index].region;
    let local_id = LocalId(9000);
    module.blocks[entry_block_index].locals.push(HirLocal {
        id: local_id,
        ty: scrutinee_type_id,
        mutable: false,
        region,
        span: None,
    });

    let scrutinee = append_hir_expression(
        module,
        HirExpressionKind::Load(HirPlace::local(local_id)),
        scrutinee_type_id,
        region,
        ValueKind::Place,
    );
    let pattern_value = append_hir_expression(
        module,
        HirExpressionKind::FixedScalar(pattern_value),
        pattern_type_id,
        region,
        ValueKind::Const,
    );
    module.side_table.map_value(None, scrutinee);
    module.side_table.map_value(None, pattern_value);

    module.blocks[entry_block_index].terminator = HirTerminator::Match {
        scrutinee,
        arms: vec![HirMatchArm {
            pattern: build_pattern(pattern_value),
            guard: None,
            body: entry_block_id,
        }],
    };
}

#[test]
fn validator_rejects_fixed_match_literal_type_different_from_scrutinee() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    inject_fixed_scalar_match_pattern(
        &mut module,
        fixed_type(FixedScalar::U8),
        fixed_type(FixedScalar::U16),
        FixedScalarValue::unsigned(FixedScalar::U16, 200).expect("200 fits in U16"),
        HirPattern::Literal,
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a pattern with a different type from its scrutinee");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("direct scrutinee type"));
}

#[test]
fn validator_rejects_fixed_match_relational_type_different_from_scrutinee() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    inject_fixed_scalar_match_pattern(
        &mut module,
        fixed_type(FixedScalar::U8),
        fixed_type(FixedScalar::U16),
        FixedScalarValue::unsigned(FixedScalar::U16, 10).expect("10 fits in U16"),
        |value| HirPattern::Relational {
            op: HirRelationalPatternOp::LessThan,
            value,
        },
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a relational pattern with the wrong scalar type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("direct scrutinee type"));
}

#[test]
fn validator_rejects_option_labelled_fixed_scalar_match_pattern() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let u8_type = fixed_type(FixedScalar::U8);
    let option_u8_type = type_environment.intern_option(u8_type);
    inject_fixed_scalar_match_pattern(
        &mut module,
        option_u8_type,
        option_u8_type,
        FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits in U8"),
        |value| HirPattern::OptionValue { value },
    );

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a fixed payload labelled with the option type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("direct scrutinee type"));
}

#[test]
fn validator_rejects_missing_side_table_mappings() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let authored_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::source_start(),
    ));

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let value = expressions
        .fixed_scalar(
            &mut const_values,
            i32_value(1),
            authored_span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal must be valid")
        .id();
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(x, value, &expressions)),
                authored_span,
            ),
            node(NodeKind::Return(vec![]), authored_span),
        ],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        expressions,
        const_values,
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    module.side_table.clear();

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject missing side-table mappings");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("side-table mapping"));
}

#[test]
fn validator_rejects_unresolved_generic_parameter_types() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, mut type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let parameter_name = string_table.intern("T");
    let generic_type_id =
        type_environment.intern_generic_parameter(GenericParameterId(0), parameter_name);

    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    entry_block.locals.push(HirLocal {
        id: LocalId(9000),
        ty: generic_type_id,
        mutable: false,
        region: entry_block.region,
        span: None,
    });

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter TypeIds");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_collection_containing_generic_parameter() {
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        type_environment.intern_constructed(
            TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
                fixed_capacity: None,
            }),
            Box::new([generic_type_id]),
        )
    });

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_option_containing_generic_parameter() {
    let option_error =
        validation_error_for_injected_local_type(|string_table, type_environment| {
            let generic_type_id = generic_parameter_type_id(string_table, type_environment);
            type_environment.intern_constructed(
                TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
                Box::new([generic_type_id]),
            )
        });
    assert_eq!(option_error.error_type, ErrorType::HirTransformation);
    assert!(option_error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_generic_nominal_instance_containing_generic_parameter() {
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        let box_path = PathId::ROOT;
        let (nominal_id, _) = type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: box_path,
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });

        type_environment.intern_generic_instance(nominal_id, Box::new([generic_type_id]))
    });

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_type_containing_generic_parameter() {
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        type_environment.intern_function(FunctionTypeKey {
            parameters: Box::new([generic_type_id]),
            returns: Box::new([builtin_type_ids::fixed_scalar(FixedScalar::I32)]),
            error_return: None,
        })
    });

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_struct_field_type_containing_generic_parameter() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, mut type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);
    let collection_type_id = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([generic_type_id]),
    );

    module.structs.push(HirStruct {
        id: StructId(9000),
        frontend_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        fields: vec![HirField {
            id: FieldId(9000),
            ty: collection_type_id,
        }],
    });

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter in HIR field types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_return_type_containing_generic_parameter() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let start_index = module
        .start_function
        .expect("normal test module should have start")
        .0 as usize;
    module.functions[start_index].returns = vec![generic_type_id].into_boxed_slice();

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter in return types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_error_type_containing_generic_parameter() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);
    let start_index = module
        .start_function
        .expect("normal test module should have start")
        .0 as usize;
    module.functions[start_index].error_return = Some(generic_type_id);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter in error types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_parameter_type_containing_generic_parameter() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let value_name = super::symbol("value", &mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                value_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, mut type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    entry_block.locals[0].ty = generic_type_id;

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter in parameter locals");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_choice_payload_type_containing_generic_parameter() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);
    let field_name = string_table.intern("value");

    module.choices.push(HirChoice {
        id: ChoiceId(9000),
        frontend_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        variants: vec![HirChoiceVariant {
            name: string_table.intern("Some"),
            fields: vec![HirChoiceField {
                name: field_name,
                ty: generic_type_id,
            }],
        }],
    });

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter in choice payloads");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_expression_type_containing_generic_parameter() {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let entry_block_index = start_entry_block_index(&module);
    let region = module.blocks[entry_block_index].region;
    let statement_id = HirNodeId(9000);
    let span = None;
    let value_id = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(1)),
        generic_type_id,
        region,
        ValueKind::Const,
    );
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(value_id),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, value_id);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject unresolved generic parameter in expression types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_anonymous_const_record_marker_on_local() {
    let error = validation_error_for_injected_local_type(|_, type_environment| {
        type_environment.anonymous_const_record_type()
    });
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_anonymous_const_record_marker_on_expression() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let marker = type_environment.anonymous_const_record_type();
    let entry_block_index = start_entry_block_index(&module);
    let region = module.blocks[entry_block_index].region;
    let statement_id = HirNodeId(9000);
    let span = None;
    let value_id = append_hir_expression(
        &mut module,
        HirExpressionKind::FixedScalar(i32_value(1)),
        marker,
        region,
        ValueKind::Const,
    );
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(value_id),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, value_id);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject the compile-time marker on an expression");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker")
    );
}

#[test]
fn validator_rejects_anonymous_const_record_marker_on_function_return() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let marker = type_environment.anonymous_const_record_type();
    let start_function = module
        .start_function
        .expect("normal test module should have start");
    module.functions[start_function.0 as usize].returns = vec![marker].into_boxed_slice();

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject the compile-time marker on a function return");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker")
    );
}

#[test]
fn classify_hir_type_rejects_anonymous_const_record_marker() {
    let type_environment = TypeEnvironment::new();
    let error = classify_hir_type(
        type_environment.anonymous_const_record_type(),
        &type_environment,
    )
    .expect_err("the marker must not classify as a runtime ABI type");
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker")
    );
}

#[test]
fn validator_rejects_placeholder_terminator() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let entry = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    module.blocks[entry.0 as usize].terminator = HirTerminator::Uninitialized;

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject placeholder terminators");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("placeholder terminator"));
}

#[test]
fn validator_rejects_region_cycle() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let region_id = module.regions[0].id();
    module.regions[0] = HirRegion::lexical(region_id, Some(region_id));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject cyclic region parents");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("cycle"));
}

#[test]
fn validator_accepts_deep_runtime_not_chain_on_small_stack() {
    const STACK_BUDGET_CHILD_ENV: &str = "MOTH_HIR_VALIDATION_STACK_BUDGET_CHILD";
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_none() {
        run_stack_budgeted_child(
            module_path!(),
            "validator_accepts_deep_runtime_not_chain_on_small_stack",
            STACK_BUDGET_CHILD_ENV,
            "deep Not validation must fit a 1 MiB stack",
        );
        return;
    }

    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
            let entry =
                add_entry_parameter(&mut module, LocalId(9000), builtin_type_ids::BOOL, false);
            let region = module.blocks[entry.0 as usize].region;
            let mut value = append_hir_expression(
                &mut module,
                HirExpressionKind::Load(HirPlace::local(LocalId(9000))),
                builtin_type_ids::BOOL,
                region,
                ValueKind::Place,
            );
            for _ in 0..2048 {
                value = append_hir_expression(
                    &mut module,
                    HirExpressionKind::UnaryOp {
                        op: HirUnaryOp::Not,
                        operand: value,
                    },
                    builtin_type_ids::BOOL,
                    region,
                    ValueKind::RValue,
                );
            }

            // Both branches revisit the same chain after its active path has unwound.
            let comparison = append_hir_expression(
                &mut module,
                HirExpressionKind::BinOp {
                    op: HirBinOp::Eq,
                    left: value,
                    right: value,
                },
                builtin_type_ids::BOOL,
                region,
                ValueKind::RValue,
            );
            append_entry_statement(&mut module, entry, HirStatementKind::Expr(comparison));

            validate_module_for_tests(
                &module,
                &string_table,
                &type_environment,
                &ExternalPackageRegistry::new(),
            )
            .expect("a deep runtime Not chain with shared uses should validate");
        })
        .expect("the bounded-stack validation thread should spawn")
        .join()
        .expect("deep Not validation should finish on a 1 MiB stack");
}

#[test]
fn validator_rejects_cycle_in_expression_id_graph() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let next_expression_id = u32::try_from(module.expressions.measurements().rows.len)
        .expect("test expression store should fit its dense ID domain");
    let cycle_id = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand: HirValueId(next_expression_id),
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    let statement = HirStatement {
        id: HirNodeId(9001),
        kind: HirStatementKind::Expr(cycle_id),
        span: None,
    };
    module.side_table.map_value(None, cycle_id);
    module.side_table.map_statement(None, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject cyclic expression IDs");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("HIR expression graph contains a cycle"));
}

#[test]
fn validator_rejects_cycle_after_consecutive_not_rows() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry = module.blocks[entry_block_index].id;
    let entry_region = module.blocks[entry_block_index].region;
    let next_expression_id = u32::try_from(module.expressions.measurements().rows.len)
        .expect("test expression store should fit its dense ID domain");
    let second_expression_id = next_expression_id
        .checked_add(1)
        .expect("the second test expression ID should fit");
    let mut expression_id = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand: HirValueId(second_expression_id),
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    // The root enters a second Not row, then a third, before revisiting the second.
    for _ in 0..2 {
        expression_id = append_hir_expression(
            &mut module,
            HirExpressionKind::UnaryOp {
                op: HirUnaryOp::Not,
                operand: expression_id,
            },
            builtin_type_ids::BOOL,
            entry_region,
            ValueKind::RValue,
        );
    }
    append_entry_statement(&mut module, entry, HirStatementKind::Expr(expression_id));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a cycle after entering multiple Not rows");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("HIR expression graph contains a cycle"));
}

#[test]
fn validator_rejects_dangling_expression_child_id() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let expression_id = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand: HirValueId(u32::MAX),
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    let statement = HirStatement {
        id: HirNodeId(9002),
        kind: HirStatementKind::Expr(expression_id),
        span: None,
    };
    module.side_table.map_value(None, expression_id);
    module.side_table.map_statement(None, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a child ID outside the dense expression store");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Invalid expression id"));
}

#[test]
fn validator_rejects_dangling_index_expression_id() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let element_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let collection_type = type_environment.intern_collection(element_type, None);
    let local_id = LocalId(9003);
    let entry_block_id = module.blocks[entry_block_index].id;
    add_local(&mut module, entry_block_id, local_id, collection_type);
    let place = HirPlace::local(local_id)
        .with_index(HirValueId(u32::MAX), &mut module.expressions, None)
        .expect("the malformed index edge can still be represented in a test row");
    let expression_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(place),
        element_type,
        entry_region,
        ValueKind::Place,
    );
    let statement = HirStatement {
        id: HirNodeId(9004),
        kind: HirStatementKind::Expr(expression_id),
        span: None,
    };
    module.side_table.map_value(None, expression_id);
    module.side_table.map_statement(None, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject an index edge outside the dense expression store");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Invalid expression id"));
}

#[test]
fn validator_rejects_cycle_closing_deep_binop_chain() {
    const DEPTH: u32 = 10_000;
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry = module.blocks[entry_block_index].id;
    let entry_region = module.blocks[entry_block_index].region;
    let leaf = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::Const,
    );
    let first_chain_id = u32::try_from(module.expressions.measurements().rows.len)
        .expect("test expression store should fit its dense ID domain");
    // The bottom row names the top row, which is appended last, so the chain closes into a loop.
    let mut value = HirValueId(first_chain_id + DEPTH - 1);
    for _ in 0..DEPTH {
        value = append_hir_expression(
            &mut module,
            HirExpressionKind::BinOp {
                op: HirBinOp::And,
                left: value,
                right: leaf,
            },
            builtin_type_ids::BOOL,
            entry_region,
            ValueKind::RValue,
        );
    }
    append_entry_statement(&mut module, entry, HirStatementKind::Expr(value));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a deep BinOp chain that loops back to its top");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("HIR expression graph contains a cycle"));
}

#[test]
fn validator_accepts_deep_binop_chain_on_small_stack() {
    const STACK_BUDGET_CHILD_ENV: &str = "MOTH_HIR_VALIDATION_BINOP_STACK_BUDGET_CHILD";
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_none() {
        run_stack_budgeted_child(
            module_path!(),
            "validator_accepts_deep_binop_chain_on_small_stack",
            STACK_BUDGET_CHILD_ENV,
            "deep BinOp validation must fit a 1 MiB stack",
        );
        return;
    }

    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
            let entry_block_index = start_entry_block_index(&module);
            let entry = module.blocks[entry_block_index].id;
            let region = module.blocks[entry_block_index].region;
            let leaf = append_hir_expression(
                &mut module,
                HirExpressionKind::Bool(true),
                builtin_type_ids::BOOL,
                region,
                ValueKind::Const,
            );
            let mut value = leaf;
            for _ in 0..20_000 {
                value = append_hir_expression(
                    &mut module,
                    HirExpressionKind::BinOp {
                        op: HirBinOp::And,
                        left: value,
                        right: leaf,
                    },
                    builtin_type_ids::BOOL,
                    region,
                    ValueKind::RValue,
                );
            }
            // The comparison visits the same chain twice, so the second visit starts after the
            // first one has unwound its active rows.
            let comparison = append_hir_expression(
                &mut module,
                HirExpressionKind::BinOp {
                    op: HirBinOp::Eq,
                    left: value,
                    right: value,
                },
                builtin_type_ids::BOOL,
                region,
                ValueKind::RValue,
            );
            append_entry_statement(&mut module, entry, HirStatementKind::Expr(comparison));

            validate_module_for_tests(
                &module,
                &string_table,
                &type_environment,
                &ExternalPackageRegistry::new(),
            )
            .expect("a deep BinOp chain with a shared revisit should validate");
        })
        .expect("the bounded-stack validation thread should spawn")
        .join()
        .expect("deep BinOp validation should finish on a 1 MiB stack");
}

#[test]
fn validator_visits_range_start_before_end() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let start = HirValueId(u32::MAX - 1);
    let end = HirValueId(u32::MAX);
    let range_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Range { start, end },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    append_expression_statement(&mut module, range_id);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a Range with dangling children");

    // Both children are dangling, so the reported id shows which child the walk reached first.
    assert!(
        error
            .msg
            .contains(&format!("Invalid expression id {start:?}"))
    );
    assert!(
        !error
            .msg
            .contains(&format!("Invalid expression id {end:?}"))
    );
}

#[test]
fn validator_visits_outermost_load_index_first() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_block_id = module.blocks[entry_block_index].id;
    let entry_region = module.blocks[entry_block_index].region;
    let element_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let collection_type = type_environment.intern_collection(element_type, None);
    let local_id = LocalId(9003);
    add_local(&mut module, entry_block_id, local_id, collection_type);
    // Projections run root-outward, so `outer` is the outermost index and must be reported first.
    let inner = HirValueId(u32::MAX - 1);
    let outer = HirValueId(u32::MAX);
    let place = HirPlace::local(local_id)
        .with_index(inner, &mut module.expressions, None)
        .expect("the malformed index edge can still be represented in a test row")
        .with_index(outer, &mut module.expressions, None)
        .expect("the malformed index edge can still be represented in a test row");
    let load_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(place),
        element_type,
        entry_region,
        ValueKind::Place,
    );
    append_expression_statement(&mut module, load_id);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a Load with dangling indexes");

    assert!(
        error
            .msg
            .contains(&format!("Invalid expression id {outer:?}"))
    );
    assert!(
        !error
            .msg
            .contains(&format!("Invalid expression id {inner:?}"))
    );
}

#[test]
fn validator_reports_left_chain_error_before_right_chain_error() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let leaf = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::Const,
    );

    // The left chain bottoms out in a dangling operand. The right chain bottoms out in an
    // out-of-range payload index, which only the post-check on Exit can reject.
    let mut left = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand: HirValueId(u32::MAX),
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    for _ in 0..1_000 {
        left = append_hir_expression(
            &mut module,
            HirExpressionKind::UnaryOp {
                op: HirUnaryOp::Not,
                operand: left,
            },
            builtin_type_ids::BOOL,
            entry_region,
            ValueKind::RValue,
        );
    }

    let mut right = append_hir_expression(
        &mut module,
        HirExpressionKind::VariantPayloadGet {
            carrier: HirVariantCarrier::Option,
            source: leaf,
            variant_index: 5,
            field_index: 0,
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    for _ in 0..1_000 {
        right = append_hir_expression(
            &mut module,
            HirExpressionKind::UnaryOp {
                op: HirUnaryOp::Not,
                operand: right,
            },
            builtin_type_ids::BOOL,
            entry_region,
            ValueKind::RValue,
        );
    }

    let root = append_hir_expression(
        &mut module,
        HirExpressionKind::BinOp {
            op: HirBinOp::And,
            left,
            right,
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    append_expression_statement(&mut module, root);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject the malformed left chain");

    // Children are walked left to right, so the left chain's dangling operand is reported before
    // the right chain's payload index is ever checked.
    assert!(error.msg.contains("Invalid expression id"));
    assert!(!error.msg.contains("VariantPayloadGet variant index"));
}

#[test]
fn validator_checks_comparison_result_type_before_walking_operands() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let left = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand: HirValueId(u32::MAX),
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    let right = append_hir_expression(
        &mut module,
        HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Not,
            operand: HirValueId(u32::MAX),
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    // An Eq row must produce Bool. Its operands are readable rows, so the result-type check fires
    // before the walk descends into the dangling operands below them.
    let comparison = append_hir_expression(
        &mut module,
        HirExpressionKind::BinOp {
            op: HirBinOp::Eq,
            left,
            right,
        },
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        entry_region,
        ValueKind::RValue,
    );
    append_expression_statement(&mut module, comparison);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a comparison that does not produce Bool");

    assert!(error.msg.contains("HirBinOp comparison must produce Bool"));
    assert!(!error.msg.contains("Invalid expression id"));
}

#[test]
fn validator_reports_invalid_load_index_before_load_type_mismatch() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_block_id = module.blocks[entry_block_index].id;
    let entry_region = module.blocks[entry_block_index].region;
    let element_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let collection_type = type_environment.intern_collection(element_type, None);
    let local_id = LocalId(9003);
    add_local(&mut module, entry_block_id, local_id, collection_type);
    let place = HirPlace::local(local_id)
        .with_index(HirValueId(u32::MAX), &mut module.expressions, None)
        .expect("the malformed index edge can still be represented in a test row");
    // The element is I32, so declaring the Load as BOOL would fail the type check. That check runs
    // after the walk has validated the index, so the dangling index must be reported instead.
    let load_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Load(place),
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::Place,
    );
    append_expression_statement(&mut module, load_id);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a Load with a dangling index");

    assert!(error.msg.contains("Invalid expression id"));
    assert!(
        !error
            .msg
            .contains("Load expression type does not match its place type")
    );
}

#[test]
fn validator_accepts_deep_load_index_chain_on_small_stack() {
    const STACK_BUDGET_CHILD_ENV: &str = "MOTH_HIR_VALIDATION_LOAD_CHAIN_STACK_BUDGET_CHILD";
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_none() {
        run_stack_budgeted_child(
            module_path!(),
            "validator_accepts_deep_load_index_chain_on_small_stack",
            STACK_BUDGET_CHILD_ENV,
            "deep Load index validation must fit a 1 MiB stack",
        );
        return;
    }

    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
            let entry_block_index = start_entry_block_index(&module);
            let entry_region = module.blocks[entry_block_index].region;
            let element_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            let collection_type = type_environment.intern_collection(element_type, None);
            let local_id = LocalId(9003);
            // Parameters are definitely defined at entry, so the Load root satisfies the
            // definite-definition check without needing an assignment statement.
            add_entry_parameter(&mut module, local_id, collection_type, false);

            // Each Load indexes the previous one, so the walk descends one index edge per row.
            let mut index = int_expression(0, element_type, entry_region, &None, &mut module);
            for _ in 0..2_000 {
                let place = HirPlace::local(local_id)
                    .with_index(index, &mut module.expressions, None)
                    .expect("the index row is a valid dense expression id");
                index = append_hir_expression(
                    &mut module,
                    HirExpressionKind::Load(place),
                    element_type,
                    entry_region,
                    ValueKind::Place,
                );
            }
            append_expression_statement(&mut module, index);

            validate_module_for_tests(
                &module,
                &string_table,
                &type_environment,
                &ExternalPackageRegistry::new(),
            )
            .expect("a deep chain of Load index rows should validate");
        })
        .expect("the bounded-stack validation thread should spawn")
        .join()
        .expect("deep Load index validation should finish on a 1 MiB stack");
}

/// Appends a `VariantConstruct` row with one named payload field.
///
/// WHAT: the carrier selects a choice variant or the `Option` `some` form.
/// WHY: every link of the deep chain builds this row shape, so one constructor owns it.
fn append_variant_row(
    module: &mut HirModule,
    carrier: HirVariantCarrier,
    variant_index: usize,
    field_name: StringId,
    value: HirValueId,
    type_id: TypeId,
    region: RegionId,
) -> HirValueId {
    let fields = module
        .expressions
        .append_variant_fields(
            &[HirVariantField {
                name: Some(field_name),
                value,
            }],
            None,
        )
        .expect("one variant field should fit the expression store");
    append_hir_expression(
        module,
        HirExpressionKind::VariantConstruct {
            carrier,
            variant_index,
            fields,
        },
        type_id,
        region,
        ValueKind::RValue,
    )
}

/// Appends `VariantPayloadGet` field 0. Each chain link reads the row built just before it.
fn append_payload_get_row(
    module: &mut HirModule,
    carrier: HirVariantCarrier,
    source: HirValueId,
    variant_index: usize,
    type_id: TypeId,
    region: RegionId,
) -> HirValueId {
    append_hir_expression(
        module,
        HirExpressionKind::VariantPayloadGet {
            carrier,
            source,
            variant_index,
            field_index: 0,
        },
        type_id,
        region,
        ValueKind::RValue,
    )
}

#[test]
fn validator_accepts_deep_mixed_shape_chain_on_small_stack() {
    // Rows per cycle: numeric 2, Bool 7, Tree 9. The phases total about 25,000 rows, past the
    // 20,000-row depth that overflowed a recursive walk on the 1 MiB child stack.
    const NUMERIC_CYCLES: usize = 1_000;
    const BOOL_CYCLES: usize = 2_000;
    const TREE_CYCLES: usize = 1_000;
    const LEAF_VARIANT: usize = 0;
    const HOLDER_VARIANT: usize = 1;
    const DICT_VARIANT: usize = 2;

    const STACK_BUDGET_CHILD_ENV: &str = "MOTH_HIR_VALIDATION_MIXED_SHAPE_STACK_BUDGET_CHILD";
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_none() {
        run_stack_budgeted_child(
            module_path!(),
            "validator_accepts_deep_mixed_shape_chain_on_small_stack",
            STACK_BUDGET_CHILD_ENV,
            "deep mixed-shape HIR validation must fit a 1 MiB stack",
        );
        return;
    }

    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
            let mut path_fork = PathInternerFork::empty();
            let entry_block_index = start_entry_block_index(&module);
            let region = module.blocks[entry_block_index].region;

            // Tree and Holder reference each other through Collection and Map, so each nominal is
            // registered empty and its members are filled once every TypeId they name exists.
            let tree_path = super::symbol("Tree", &mut path_fork, &mut string_table);
            let holder_path = super::symbol("Holder", &mut path_fork, &mut string_table);
            let (_, tree_type) = type_environment.register_nominal_choice(ChoiceTypeDefinition {
                id: NominalTypeId(0),
                path: tree_path,
                variants: Box::new([]),
                generic_parameters: None,
            });
            let (_, holder_type) = type_environment.register_nominal_struct(StructTypeDefinition {
                id: NominalTypeId(0),
                path: holder_path,
                fields: Box::new([]),
                generic_parameters: None,
                const_record: false,
            });

            let bool_type = builtin_type_ids::BOOL;
            let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
            let byte_type = builtin_type_ids::fixed_scalar(FixedScalar::Byte);
            let collection_type = type_environment.intern_collection(tree_type, None);
            let map_type = type_environment.intern_map(bool_type, tree_type);
            let option_bool_type = type_environment.intern_option(bool_type);

            let value_name = string_table.intern("value");
            let holder_name = string_table.intern("holder");
            let entries_name = string_table.intern("entries");
            let items_path = super::symbol("items", &mut path_fork, &mut string_table);
            let value_path = super::symbol("value", &mut path_fork, &mut string_table);
            let holder_field_path = super::symbol("holder", &mut path_fork, &mut string_table);
            let entries_path = super::symbol("entries", &mut path_fork, &mut string_table);
            type_environment.update_struct_fields(
                holder_type,
                vec![FieldDefinition {
                    name: items_path,
                    type_id: collection_type,
                    span: None,
                }]
                .into_boxed_slice(),
            );
            type_environment.update_choice_variants(
                tree_type,
                vec![
                    ChoiceVariantDefinition {
                        name: string_table.intern("Leaf"),
                        tag: LEAF_VARIANT,
                        payload: ChoiceVariantPayloadDefinition::Record {
                            fields: vec![FieldDefinition {
                                name: value_path,
                                type_id: bool_type,
                                span: None,
                            }]
                            .into_boxed_slice(),
                        },
                        span: None,
                    },
                    ChoiceVariantDefinition {
                        name: string_table.intern("Holder"),
                        tag: HOLDER_VARIANT,
                        payload: ChoiceVariantPayloadDefinition::Record {
                            fields: vec![FieldDefinition {
                                name: holder_field_path,
                                type_id: holder_type,
                                span: None,
                            }]
                            .into_boxed_slice(),
                        },
                        span: None,
                    },
                    ChoiceVariantDefinition {
                        name: string_table.intern("Dict"),
                        tag: DICT_VARIANT,
                        payload: ChoiceVariantPayloadDefinition::Record {
                            fields: vec![FieldDefinition {
                                name: entries_path,
                                type_id: map_type,
                                span: None,
                            }]
                            .into_boxed_slice(),
                        },
                        span: None,
                    },
                ]
                .into_boxed_slice(),
            );

            let tree_choice = ChoiceId(0);
            let holder_struct = StructId(0);
            let items_field = FieldId(0);
            add_validation_hir_struct(
                &mut module,
                holder_struct,
                holder_type,
                [(items_field, collection_type)],
            );
            module.choices.push(HirChoice {
                id: tree_choice,
                frontend_type_id: tree_type,
                variants: vec![
                    HirChoiceVariant {
                        name: string_table.intern("Leaf"),
                        fields: vec![HirChoiceField {
                            name: value_name,
                            ty: bool_type,
                        }],
                    },
                    HirChoiceVariant {
                        name: string_table.intern("Holder"),
                        fields: vec![HirChoiceField {
                            name: holder_name,
                            ty: holder_type,
                        }],
                    },
                    HirChoiceVariant {
                        name: string_table.intern("Dict"),
                        fields: vec![HirChoiceField {
                            name: entries_name,
                            ty: map_type,
                        }],
                    },
                ],
            });
            let tree_carrier = || HirVariantCarrier::Choice {
                choice_id: tree_choice,
            };

            // Shared leaves. Later links reuse them, so the validator's active-row marks see repeated
            // visits to rows that are already finished.
            let leaf_u8 = append_hir_expression(
                &mut module,
                HirExpressionKind::FixedScalar(
                    FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("1 fits in U8"),
                ),
                u8_type,
                region,
                ValueKind::Const,
            );
            let leaf_bool = append_hir_expression(
                &mut module,
                HirExpressionKind::Bool(true),
                bool_type,
                region,
                ValueKind::Const,
            );
            let leaf_tree = append_variant_row(
                &mut module,
                tree_carrier(),
                LEAF_VARIANT,
                value_name,
                leaf_bool,
                tree_type,
                region,
            );

            // Numeric phase: U8 -> Byte -> U8. Each Cast reads the row before it, so the post-children
            // policy and source checks run on every link.
            let mut numeric = leaf_u8;
            for _ in 0..NUMERIC_CYCLES {
                let byte = append_hir_expression(
                    &mut module,
                    HirExpressionKind::Cast {
                        source: numeric,
                        policy: BuiltinCastPolicyId::U8ToByte,
                    },
                    byte_type,
                    region,
                    ValueKind::RValue,
                );
                numeric = append_hir_expression(
                    &mut module,
                    HirExpressionKind::Cast {
                        source: byte,
                        policy: BuiltinCastPolicyId::ByteToU8,
                    },
                    u8_type,
                    region,
                    ValueKind::RValue,
                );
            }

            // Bridge: a U8 comparison produces the first Bool row of the spine.
            let mut flag = append_hir_expression(
                &mut module,
                HirExpressionKind::BinOp {
                    left: numeric,
                    op: HirBinOp::Eq,
                    right: leaf_u8,
                },
                bool_type,
                region,
                ValueKind::RValue,
            );

            // Bool phase: Not, And, Eq, then Option wrap and unwrap, then Tree.Leaf wrap and unwrap.
            for _ in 0..BOOL_CYCLES {
                let negated = append_hir_expression(
                    &mut module,
                    HirExpressionKind::UnaryOp {
                        op: HirUnaryOp::Not,
                        operand: flag,
                    },
                    bool_type,
                    region,
                    ValueKind::RValue,
                );
                let conjoined = append_hir_expression(
                    &mut module,
                    HirExpressionKind::BinOp {
                        left: negated,
                        op: HirBinOp::And,
                        right: leaf_bool,
                    },
                    bool_type,
                    region,
                    ValueKind::RValue,
                );
                let compared = append_hir_expression(
                    &mut module,
                    HirExpressionKind::BinOp {
                        left: conjoined,
                        op: HirBinOp::Eq,
                        right: leaf_bool,
                    },
                    bool_type,
                    region,
                    ValueKind::RValue,
                );
                let some = append_variant_row(
                    &mut module,
                    HirVariantCarrier::Option,
                    OPTION_SOME_VARIANT_INDEX,
                    value_name,
                    compared,
                    option_bool_type,
                    region,
                );
                let unwrapped = append_payload_get_row(
                    &mut module,
                    HirVariantCarrier::Option,
                    some,
                    OPTION_SOME_VARIANT_INDEX,
                    bool_type,
                    region,
                );
                let leafed = append_variant_row(
                    &mut module,
                    tree_carrier(),
                    LEAF_VARIANT,
                    value_name,
                    unwrapped,
                    tree_type,
                    region,
                );
                flag = append_payload_get_row(
                    &mut module,
                    tree_carrier(),
                    leafed,
                    LEAF_VARIANT,
                    bool_type,
                    region,
                );
            }

            // Bridge into the Tree family. The map key is the Bool spine; the map value is the shared
            // Tree leaf.
            let entries = module
                .expressions
                .append_map_entries(
                    &[HirMapEntry {
                        key: flag,
                        value: leaf_tree,
                    }],
                    None,
                )
                .expect("one map entry should fit the expression store");
            let entries_map = append_hir_expression(
                &mut module,
                HirExpressionKind::MapLiteral(entries),
                map_type,
                region,
                ValueKind::RValue,
            );
            let mut tree = append_variant_row(
                &mut module,
                tree_carrier(),
                DICT_VARIANT,
                entries_name,
                entries_map,
                tree_type,
                region,
            );

            // Tree phase: Collection, StructConstruct, Holder wrap and unwrap, a MapLiteral whose value
            // child is the spine, then Dict wrap and unwrap.
            for _ in 0..TREE_CYCLES {
                let items = module
                    .expressions
                    .append_values(&[tree], None)
                    .expect("one collection element should fit the expression store");
                let collection = append_hir_expression(
                    &mut module,
                    HirExpressionKind::Collection(items),
                    collection_type,
                    region,
                    ValueKind::RValue,
                );
                let struct_fields = module
                    .expressions
                    .append_struct_fields(&[(items_field, collection)], None)
                    .expect("one struct field should fit the expression store");
                let holder = append_hir_expression(
                    &mut module,
                    HirExpressionKind::StructConstruct {
                        struct_id: holder_struct,
                        fields: struct_fields,
                    },
                    holder_type,
                    region,
                    ValueKind::RValue,
                );
                let wrapped = append_variant_row(
                    &mut module,
                    tree_carrier(),
                    HOLDER_VARIANT,
                    holder_name,
                    holder,
                    tree_type,
                    region,
                );
                let held = append_payload_get_row(
                    &mut module,
                    tree_carrier(),
                    wrapped,
                    HOLDER_VARIANT,
                    holder_type,
                    region,
                );
                let rewrapped = append_variant_row(
                    &mut module,
                    tree_carrier(),
                    HOLDER_VARIANT,
                    holder_name,
                    held,
                    tree_type,
                    region,
                );
                let map_entries = module
                    .expressions
                    .append_map_entries(
                        &[HirMapEntry {
                            key: leaf_bool,
                            value: rewrapped,
                        }],
                        None,
                    )
                    .expect("one map entry should fit the expression store");
                let map = append_hir_expression(
                    &mut module,
                    HirExpressionKind::MapLiteral(map_entries),
                    map_type,
                    region,
                    ValueKind::RValue,
                );
                let dict = append_variant_row(
                    &mut module,
                    tree_carrier(),
                    DICT_VARIANT,
                    entries_name,
                    map,
                    tree_type,
                    region,
                );
                let mapped = append_payload_get_row(
                    &mut module,
                    tree_carrier(),
                    dict,
                    DICT_VARIANT,
                    map_type,
                    region,
                );
                tree = append_variant_row(
                    &mut module,
                    tree_carrier(),
                    DICT_VARIANT,
                    entries_name,
                    mapped,
                    tree_type,
                    region,
                );
            }

            // The root reuses the deepest Tree row and the shared leaf. The second visit to `tree`
            // starts only after the first visit has finished, so it is not a cycle.
            let elements = module
                .expressions
                .append_values(&[tree, leaf_tree, tree], None)
                .expect("collection elements should fit the expression store");
            let root = append_hir_expression(
                &mut module,
                HirExpressionKind::Collection(elements),
                collection_type,
                region,
                ValueKind::RValue,
            );
            append_expression_statement(&mut module, root);

            validate_module_for_tests(
                &module,
                &string_table,
                &type_environment,
                &ExternalPackageRegistry::new(),
            )
            .expect("the deep mixed-shape chain should validate");
        })
        .expect("the bounded-stack validation thread should spawn")
        .join()
        .expect("deep mixed-shape validation should finish on a 1 MiB stack");
}

#[test]
fn validator_rejects_missing_region_parent() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let region_id = module.regions[0].id();
    module.regions[0] = HirRegion::lexical(region_id, Some(RegionId(9999)));

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject missing region parents");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("missing parent"));
}

#[test]
fn validator_rejects_cross_function_cfg_edges() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let helper_name = super::symbol("helper", &mut path_fork, &mut string_table);

    let helper = function_node(
        helper_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![helper, start],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let start_entry = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    let helper_entry = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .map(|function| function.entry)
        .expect("helper function should exist");

    module.blocks[start_entry.0 as usize].terminator = HirTerminator::Jump {
        target: helper_entry,
        args: vec![],
    };

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject cross-function CFG edges");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("multiple functions") || error.msg.contains("crosses function boundary")
    );
}

#[test]
fn lowering_errors_preserve_string_table_context() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let missing_function = super::symbol("missing_fn", &mut path_fork, &mut string_table);

    let call_span = None;

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let call = expressions
        .function_call_with_typed_arguments(missing_function, Vec::new(), Vec::new(), call_span)
        .expect("fixture call must have valid expression edges")
        .id();
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(NodeKind::ExpressionStatement(call), call_span),
            node(NodeKind::Return(vec![]), None),
        ],
        None,
    );

    let messages = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            ConstValueStore::default(),
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("unknown function call should fail HIR lowering");

    assert!(matches!(messages, AstDiagnosticFailure::Infrastructure(_)));
}
// ---------------------------------------------------------------------------
// VariantConstruct validation
// ---------------------------------------------------------------------------

#[test]
fn hir_variant_construct_option_invalid_index_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let entry_block_index = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize;
    let region = module.blocks[entry_block_index].region;

    let mut type_env = type_environment.clone();
    let int_ty = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let option_ty = type_env.intern_constructed(
        crate::compiler_frontend::datatypes::ids::TypeConstructor::Builtin(
            crate::compiler_frontend::datatypes::ids::BuiltinTypeConstructor::Option,
        ),
        Box::new([int_ty]),
    );

    let stmt_id = HirNodeId(9000);
    let span = None;
    let fields = module
        .expressions
        .append_variant_fields(&[], span)
        .expect("empty option fields should fit the expression store");
    let expr_id = append_hir_expression(
        &mut module,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 99,
            fields,
        },
        option_ty,
        region,
        ValueKind::Const,
    );

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expr_id),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_env,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject out-of-range Option variant index");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("out of range"),
        "expected 'out of range' in error, got: {}",
        error.msg
    );
}

#[test]
fn hir_variant_construct_choice_wrong_field_name_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let response_param = super::symbol("response", &mut path_fork, &mut string_table);
    let ok_name = string_table.intern("Ok");
    let err_name = string_table.intern("Err");
    let wrong_name = string_table.intern("content");

    let choice_variants = vec![
        ChoiceVariant {
            id: ok_name,
            payload: ChoiceVariantPayload::Record {
                fields: vec![Declaration {
                    id: path_fork
                        .try_intern_portable_path("message", &mut string_table)
                        .expect("test path fits"),
                    type_id: builtin_type_ids::STRING,
                    type_syntax: None,
                    value_mode: ValueMode::ImmutableOwned,
                    value: DeclarationValue::Absent,
                    binding_span: None,
                    config_qualifier: None,
                }],
            },
            span: None,
        },
        ChoiceVariant {
            id: err_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
    ];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                response_param,
                builtin_type_ids::NONE,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: path_fork
                .try_intern_portable_path("Response", &mut string_table)
                .expect("test path fits"),
            variants: choice_variants,
        }],
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let entry_block_index = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize;
    let region = module.blocks[entry_block_index].region;

    let string_ty = builtin_type_ids::STRING;

    let stmt_id = HirNodeId(9000);
    let span = None;
    let value_id = append_hir_expression(
        &mut module,
        HirExpressionKind::StringLiteral("hello".to_owned()),
        string_ty,
        region,
        ValueKind::Const,
    );
    let fields = module
        .expressions
        .append_variant_fields(
            &[HirVariantField {
                name: Some(wrong_name),
                value: value_id,
            }],
            span,
        )
        .expect("choice field should fit the expression store");
    let expr_id = append_hir_expression(
        &mut module,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Choice {
                choice_id: ChoiceId(0),
            },
            variant_index: 0,
            fields,
        },
        string_ty,
        region,
        ValueKind::Const,
    );

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expr_id),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id);
    module.side_table.map_value(span, value_id);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject wrong field name in choice VariantConstruct");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("field name"),
        "expected 'field name' in error, got: {}",
        error.msg
    );
}

#[test]
fn hir_variant_construct_choice_wrong_field_type_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let response_param = super::symbol("response", &mut path_fork, &mut string_table);
    let ok_name = string_table.intern("Ok");
    let err_name = string_table.intern("Err");
    let message_name = string_table.intern("message");

    let choice_variants = vec![
        ChoiceVariant {
            id: ok_name,
            payload: ChoiceVariantPayload::Record {
                fields: vec![Declaration {
                    id: path_fork
                        .try_intern_portable_path("message", &mut string_table)
                        .expect("test path fits"),
                    type_id: builtin_type_ids::STRING,
                    type_syntax: None,
                    value_mode: ValueMode::ImmutableOwned,
                    value: DeclarationValue::Absent,
                    binding_span: None,
                    config_qualifier: None,
                }],
            },
            span: None,
        },
        ChoiceVariant {
            id: err_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
    ];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                response_param,
                builtin_type_ids::NONE,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: path_fork
                .try_intern_portable_path("Response", &mut string_table)
                .expect("test path fits"),
            variants: choice_variants,
        }],
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let entry_block_index = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize;
    let region = module.blocks[entry_block_index].region;

    let string_ty = builtin_type_ids::STRING;
    let bool_ty = builtin_type_ids::BOOL;

    let stmt_id = HirNodeId(9000);
    let span = None;
    let value_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        bool_ty,
        region,
        ValueKind::Const,
    );
    let fields = module
        .expressions
        .append_variant_fields(
            &[HirVariantField {
                name: Some(message_name),
                value: value_id,
            }],
            span,
        )
        .expect("choice field should fit the expression store");
    let expr_id = append_hir_expression(
        &mut module,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Choice {
                choice_id: ChoiceId(0),
            },
            variant_index: 0,
            fields,
        },
        string_ty,
        region,
        ValueKind::Const,
    );

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expr_id),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id);
    module.side_table.map_value(span, value_id);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject wrong field type in choice VariantConstruct");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("field type mismatch"),
        "expected 'field type mismatch' in error, got: {}",
        error.msg
    );
}

#[test]
fn hir_route_construct_with_input_missing_leading_capture_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let rename_name = string_table.intern("Rename");
    let record_field = |name: &str,
                        type_id: TypeId,
                        path_fork: &mut super::PathInternerFork,
                        string_table: &mut StringTable| Declaration {
        id: path_fork
            .try_intern_portable_path(name, string_table)
            .expect("test path fits"),
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };
    let choice_variants = vec![ChoiceVariant {
        id: rename_name,
        payload: ChoiceVariantPayload::Record {
            fields: vec![
                record_field(
                    "id",
                    builtin_type_ids::STRING,
                    &mut path_fork,
                    &mut string_table,
                ),
                record_field(
                    "value",
                    builtin_type_ids::STRING,
                    &mut path_fork,
                    &mut string_table,
                ),
            ],
        },
        span: None,
    }];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );
    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: path_fork
                .try_intern_portable_path("TodoMessage", &mut string_table)
                .expect("test path fits"),
            variants: choice_variants,
        }],
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");
    let entry_block_index = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize;
    let region = module.blocks[entry_block_index].region;
    let choice_type = module.choices[0].frontend_type_id;

    // One future input supplies only the trailing `value`, so `id` must be captured.
    let span = None;
    let fields = module
        .expressions
        .append_variant_fields(&[], span)
        .expect("empty capture range should fit the expression store");
    let expr_id = append_hir_expression(
        &mut module,
        HirExpressionKind::RouteConstruct {
            choice_id: ChoiceId(0),
            variant_index: 0,
            fields,
            future_input: true,
        },
        choice_type,
        region,
        ValueKind::Const,
    );
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expr_id),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject a one-input route missing a leading capture");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("captures must fill"),
        "expected the capture-count error, got: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_collection_expression_with_non_collection_type() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    inject_collection_expression_statement(&mut module, int_type, None);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject Collection expression with non-collection type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("not a collection type"),
        "expected 'not a collection type' in error, got: {}",
        error.msg
    );
}

#[test]
fn validator_accepts_collection_expression_with_growable_collection_type() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let growable_collection = type_environment.intern_collection(int_type, None);
    inject_collection_expression_statement(&mut module, growable_collection, None);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("validator should accept Collection expression with growable collection type");
}

#[test]
fn validator_accepts_collection_expression_with_fixed_collection_type() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let fixed_collection = type_environment.intern_collection(int_type, Some(64));
    inject_collection_expression_statement(&mut module, fixed_collection, None);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("validator should accept Collection expression with fixed collection type");
}

#[test]
fn validator_rejects_number_literal_scale_that_mismatches_hir_type() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let value_scale = NumberScale::new(2).expect("test scale is valid");
    let value = NumberValue::from_normalized("1.25", NumericLiteralSign::Positive, value_scale)
        .expect("Dec value is exactly representable at its declared scale");
    let number_type = type_environment.intern_number(value_scale);
    let mismatched_type = type_environment.intern_number(NumberScale::ZERO);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let span = None;
    let value_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Number(value),
        number_type,
        entry_region,
        ValueKind::Const,
    );
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(value_id),
        span,
    };

    module.side_table.map_value(span, value_id);
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("matching Dec value and TypeId scales should pass validation");

    let expression = match module.blocks[entry_block_index]
        .statements
        .last()
        .map(|statement| &statement.kind)
    {
        Some(HirStatementKind::Expr(expression)) => *expression,
        _ => panic!("injected Dec expression statement should remain in the HIR block"),
    };
    let mismatched_expression = module
        .expressions
        .copy_expression_with_metadata(expression, span, mismatched_type, entry_region)
        .expect("the mismatched test row should append before publication");
    let statement = module.blocks[entry_block_index]
        .statements
        .last_mut()
        .expect("injected Dec expression statement should remain in the HIR block");
    statement.kind = HirStatementKind::Expr(mismatched_expression);
    module.side_table.map_value(span, mismatched_expression);

    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("validator should reject Dec value/type scale disagreement");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_owns_pending_catch_handler_edges_and_rejects_dangling_or_cross_function_records() {
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

    let source = "double |value I32| -> I32:\nreturn value + 2147483647\n;\n\
                  recover |value I32| -> I32:\nreturn double(value) catch then 0\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("the direct inferred-failure catch must lower");
    assert_eq!(module.catch_protected_calls.len(), 1);
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("the pending handler edge must belong to its caller before installation");
    let record = module.catch_protected_calls[0];

    module.catch_protected_calls[0].statement = HirNodeId(u32::MAX);
    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("a dangling pending edge cannot validate");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    module.catch_protected_calls[0] = record;

    let foreign_function = module
        .functions
        .iter()
        .find(|function| function.id != record.owner)
        .expect("the fixture must have a distinct callee");
    module.catch_protected_calls[0].handler.block = foreign_function.entry;
    let error = validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect_err("a pending handler cannot enter another function's CFG");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}
