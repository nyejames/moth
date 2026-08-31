//! Expression lowering tests for JavaScript output.

use super::support::*;
use crate::backends::structural_string::StructuralStringUrlMap;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use std::sync::Arc;

#[test]
fn integer_division_binop_emits_zero_checked_truncation_path() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let assign_lhs = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 10, types.int, region),
        },
        1,
    );
    let assign_rhs = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(1)),
            value: int_expression(2, 3, types.int, region),
        },
        2,
    );
    let int_div_expr = expression(
        3,
        HirExpressionKind::BinOp {
            left: Box::new(expression(
                4,
                HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                types.int,
                region,
                ValueKind::Place,
            )),
            op: HirBinOp::IntDiv,
            right: Box::new(expression(
                5,
                HirExpressionKind::Load(HirPlace::Local(LocalId(1))),
                types.int,
                region,
                ValueKind::Place,
            )),
        },
        types.int,
        region,
        ValueKind::RValue,
    );
    let assign_result = statement(
        3,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(2)),
            value: int_div_expr,
        },
        3,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, types.int, region),
            local(1, types.int, region),
            local(2, types.int, region),
        ],
        statements: vec![assign_lhs, assign_rhs, assign_result],
        terminator: HirTerminator::Return(unit_expression(6, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[
            (LocalId(0), "lhs"),
            (LocalId(1), "rhs"),
            (LocalId(2), "result"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("Math.trunc("),
        "integer division should use truncation path"
    );
    assert!(
        output.source.contains("Integer division by zero"),
        "integer division path should include explicit zero trap"
    );
    assert!(
        output.source.contains("__rhs === 0"),
        "integer division path should branch on zero divisor"
    );
}

// Clone / explicit copy tests [clone]
// ---------------------------------------------------------------------------

/// Verifies that a HIR Copy expression emits __moth_clone_value(__moth_read(...)). [clone]
#[test]
fn explicit_copy_emits_clone_value_wrapped_read() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    // Assign a source local, then assign a copy of it to a target local.
    let assign_source = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(1, "hello", types.string, RegionId(0)),
        },
        1,
    );

    let copy_expr = expression(
        2,
        HirExpressionKind::Copy(HirPlace::Local(LocalId(0))),
        types.string,
        RegionId(0),
        ValueKind::RValue,
    );

    let assign_copy = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(1)),
            value: copy_expr,
        },
        2,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![
            local(0, types.string, RegionId(0)),
            local(1, types.string, RegionId(0)),
        ],
        statements: vec![assign_source, assign_copy],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "src"), (LocalId(1), "dst")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let src_name = expected_dev_local_name("src", 0);

    assert!(
        output
            .source
            .contains(&format!("__moth_clone_value(__moth_read({src_name}))")),
        "Copy expression must emit __moth_clone_value(__moth_read(src))"
    );
}

// ---------------------------------------------------------------------------
// Option carrier construct tests [option]
// ---------------------------------------------------------------------------

/// Verifies that VariantConstruct with Option carrier lowers to tagged JS objects. [option]
#[test]
fn lowers_option_construct_expression() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let value_name = string_table.intern("value");
    let option_value = expression(
        1,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 1,
            fields: vec![HirVariantField {
                name: Some(value_name),
                value: int_expression(2, 10, types.int, RegionId(0)),
            }],
        },
        types.option_int,
        RegionId(0),
        ValueKind::RValue,
    );

    let option_statement = statement(1, HirStatementKind::Expr(option_value), 1);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![option_statement],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("VariantConstruct(Option) lowering should succeed");

    assert!(
        output.source.contains("{ tag: \"some\", \"value\": 10 }"),
        "expected VariantConstruct(Option) to lower into a tagged JS object"
    );
}

// ---------------------------------------------------------------------------
// Fixed collection expression lowering tests [fixed-collection]
// ---------------------------------------------------------------------------

/// Verifies that a growable collection expression lowers to a plain JS array. [fixed-collection]
#[test]
fn growable_collection_expression_lowers_to_array() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();

    let growable_type = type_environment.intern_collection(types.int, None);

    let collection_expr = expression(
        1,
        HirExpressionKind::Collection(vec![
            int_expression(2, 1, types.int, RegionId(0)),
            int_expression(3, 2, types.int, RegionId(0)),
        ]),
        growable_type,
        RegionId(0),
        ValueKind::RValue,
    );

    let collection_statement = statement(1, HirStatementKind::Expr(collection_expr), 1);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![collection_statement],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("[1, 2]"),
        "growable collection must lower to a plain JS array literal"
    );
    // The runtime prelude includes the helper definition, so this checks only
    // for the expression-level wrapper call shape.
    assert!(
        !output.source.contains("__moth_fixed_collection([1, 2]"),
        "growable collection expression must not be wrapped in __moth_fixed_collection"
    );
}

/// Verifies that a fixed collection expression lowers to a __moth_fixed_collection call. [fixed-collection]
#[test]
fn fixed_collection_expression_lowers_to_wrapper() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();

    let fixed_type = type_environment.intern_collection(types.int, Some(4));

    let collection_expr = expression(
        1,
        HirExpressionKind::Collection(vec![
            int_expression(2, 10, types.int, RegionId(0)),
            int_expression(3, 20, types.int, RegionId(0)),
        ]),
        fixed_type,
        RegionId(0),
        ValueKind::RValue,
    );

    let collection_statement = statement(1, HirStatementKind::Expr(collection_expr), 1);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![collection_statement],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_fixed_collection([10, 20], 4)"),
        "fixed collection must lower to __moth_fixed_collection(items, capacity)"
    );
}

/// Verifies that a fixed collection expression with zero elements emits the wrapper. [fixed-collection]
#[test]
fn fixed_collection_empty_expression_lowers_to_wrapper() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();

    let fixed_type = type_environment.intern_collection(types.int, Some(2));

    let collection_expr = expression(
        1,
        HirExpressionKind::Collection(vec![]),
        fixed_type,
        RegionId(0),
        ValueKind::RValue,
    );

    let collection_statement = statement(1, HirStatementKind::Expr(collection_expr), 1);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![collection_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("__moth_fixed_collection([], 2)"),
        "empty fixed collection must lower to __moth_fixed_collection([], capacity)"
    );
}

// Map literal expression lowering tests [map]
// ---------------------------------------------------------------------------

/// Verifies that a map literal with entries lowers to `__moth_map_new([[key, value], ...])`. [map]
#[test]
fn map_literal_with_entries_lowers_to_map_new() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let entry1 = HirMapEntry {
        key: string_expression(1, "Priya", types.string, RegionId(0)),
        value: int_expression(2, 10, types.int, RegionId(0)),
    };
    let entry2 = HirMapEntry {
        key: string_expression(3, "Grace", types.string, RegionId(0)),
        value: int_expression(4, 12, types.int, RegionId(0)),
    };

    let map_expr = expression(
        5,
        HirExpressionKind::MapLiteral(vec![entry1, entry2]),
        types.map_string_int,
        RegionId(0),
        ValueKind::RValue,
    );

    let map_statement = statement(1, HirStatementKind::Expr(map_expr), 1);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![map_statement],
        terminator: HirTerminator::Return(unit_expression(6, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_map_new([[\"Priya\", 10], [\"Grace\", 12]])"),
        "map literal must lower to __moth_map_new with ordered key-value pairs"
    );
}

/// Verifies that an empty map literal lowers to `__moth_map_new([])`. [map]
#[test]
fn empty_map_literal_lowers_to_map_new_with_empty_array() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let map_expr = expression(
        1,
        HirExpressionKind::MapLiteral(vec![]),
        types.map_string_int,
        RegionId(0),
        ValueKind::RValue,
    );

    let map_statement = statement(1, HirStatementKind::Expr(map_expr), 1);

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![map_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("__moth_map_new([])"),
        "empty map literal must lower to __moth_map_new([])"
    );
}

/// Verifies that builder-rendered structural strings become escaped JS string literals.
#[test]
fn lowers_structural_string_with_url_map_as_escaped_literal() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let prefix = string_table.intern("before");
    let suffix = string_table.intern("after");
    let structural_expression = expression(
        1,
        HirExpressionKind::StructuralString {
            pieces: vec![
                ConstStringPiece::Text(prefix),
                ConstStringPiece::SiteRoot,
                ConstStringPiece::Text(suffix),
            ],
        },
        types.string,
        region,
        ValueKind::RValue,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(structural_expression),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.string,
    };
    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let url_map = StructuralStringUrlMap {
        site_root_url: Some("quote\"line\n".to_owned()),
        ..Default::default()
    };
    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config().with_structural_string_urls(Arc::new(url_map)),
        &type_environment,
    )
    .expect("JS lowering should render structural strings");

    assert!(
        output
            .source
            .contains("return \"beforequote\\\"line\\nafter\";"),
        "structural string should be rendered and escaped as one JS literal:\n{}",
        output.source
    );
}
