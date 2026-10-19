//! Same-instance retry tests for the iterative HIR expression walk.
//!
//! WHAT: validates one expression twice through a single `HirValidator`, so the second walk starts
//! from whatever state the first walk left behind.
//! WHY: a failed walk must clear every active row and drain its pending frames. A row left marked
//! would make the retry report a false cycle, so the retry has to reproduce the first diagnostic.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirVariantCarrier, ValueKind};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::tests::{
    append_hir_expression, minimal_lowered_hir_module, start_entry_block_index,
};

#[test]
fn validator_clears_consecutive_not_rows_after_dangling_child_error() {
    let (_string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let mut expression_id = HirValueId(u32::MAX);
    for _ in 0..3 {
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

    let external_packages = ExternalPackageRegistry::new();
    let mut validator = HirValidator::new(&module, &type_environment, &external_packages);
    validator
        .collect_definition_ids()
        .expect("test module definition IDs should validate");
    let first_error = validator
        .validate_expression(expression_id, None)
        .expect_err("validator should reject the dangling child after entering three Not rows");
    let second_error = validator
        .validate_expression(expression_id, None)
        .expect_err("retry should reach the dangling child after clearing every entered Not row");

    assert_eq!(first_error.error_type, ErrorType::HirTransformation);
    assert!(first_error.msg.contains("Invalid expression id"));
    assert_eq!(second_error.error_type, ErrorType::HirTransformation);
    assert!(second_error.msg.contains("Invalid expression id"));
    assert_eq!(second_error.msg, first_error.msg);
}

#[test]
fn validator_rejects_self_cycle_through_range_row() {
    let (_string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let leaf = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::Const,
    );
    // The Range's start points at the row it is about to occupy, so walking the start re-enters
    // the Range itself. The cycle is only detected if the first walk's active mark survives until
    // the retry starts.
    let range_expression_id = u32::try_from(module.expressions.measurements().rows.len)
        .expect("test expression store should fit its dense ID domain");
    let range_id = append_hir_expression(
        &mut module,
        HirExpressionKind::Range {
            start: HirValueId(range_expression_id),
            end: leaf,
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );

    let external_packages = ExternalPackageRegistry::new();
    let mut validator = HirValidator::new(&module, &type_environment, &external_packages);
    validator
        .collect_definition_ids()
        .expect("test module definition IDs should validate");
    let first_error = validator
        .validate_expression(range_id, None)
        .expect_err("a Range row that reaches itself should be rejected");
    let second_error = validator
        .validate_expression(range_id, None)
        .expect_err("retry should report the same cycle");

    assert_eq!(first_error.error_type, ErrorType::HirTransformation);
    assert!(
        first_error
            .msg
            .contains("HIR expression graph contains a cycle")
    );
    assert_eq!(second_error.error_type, ErrorType::HirTransformation);
    assert!(
        second_error
            .msg
            .contains("HIR expression graph contains a cycle")
    );
}

#[test]
fn validator_clears_expression_marks_after_error_in_deep_walk() {
    let (_string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let leaf = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::Const,
    );
    // The dangling start sits below a deep chain, so the first walk must unwind every active row
    // before the retry can reach the same error instead of a stale cycle.
    let mut value = append_hir_expression(
        &mut module,
        HirExpressionKind::Range {
            start: HirValueId(u32::MAX),
            end: leaf,
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    for _ in 0..1_000 {
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

    let external_packages = ExternalPackageRegistry::new();
    let mut validator = HirValidator::new(&module, &type_environment, &external_packages);
    validator
        .collect_definition_ids()
        .expect("test module definition IDs should validate");
    let first_error = validator
        .validate_expression(value, None)
        .expect_err("validator should reject the dangling Range start");
    let second_error = validator
        .validate_expression(value, None)
        .expect_err("retry should reach the same dangling Range start");

    assert_eq!(first_error.error_type, ErrorType::HirTransformation);
    assert!(first_error.msg.contains("Invalid expression id"));
    assert_eq!(second_error.msg, first_error.msg);
}

#[test]
fn validator_retry_reports_same_post_check_error_after_error_exit() {
    let (_string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let source = append_hir_expression(
        &mut module,
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::Const,
    );
    // Variant index 5 is outside Option's 0..=1 range. The error comes from the post-check that
    // runs on Exit, after `source` has been walked and its mark cleared.
    let mut value = append_hir_expression(
        &mut module,
        HirExpressionKind::VariantPayloadGet {
            carrier: HirVariantCarrier::Option,
            source,
            variant_index: 5,
            field_index: 0,
        },
        builtin_type_ids::BOOL,
        entry_region,
        ValueKind::RValue,
    );
    // The Not parents stay on the frame stack when the error returns. The retry only reproduces
    // the same diagnostic if those pending Exit frames are drained along with their marks.
    for _ in 0..3 {
        value = append_hir_expression(
            &mut module,
            HirExpressionKind::UnaryOp {
                op: HirUnaryOp::Not,
                operand: value,
            },
            builtin_type_ids::BOOL,
            entry_region,
            ValueKind::RValue,
        );
    }

    let external_packages = ExternalPackageRegistry::new();
    let mut validator = HirValidator::new(&module, &type_environment, &external_packages);
    validator
        .collect_definition_ids()
        .expect("test module definition IDs should validate");
    let first_error = validator
        .validate_expression(value, None)
        .expect_err("validator should reject an out-of-range Option payload index");
    let second_error = validator
        .validate_expression(value, None)
        .expect_err("retry should reject the same Option payload index");

    assert_eq!(first_error.error_type, ErrorType::HirTransformation);
    assert!(
        first_error
            .msg
            .contains("VariantPayloadGet variant index 5 out of range for Option")
    );
    assert_eq!(second_error.msg, first_error.msg);
}
