//! Generated sidecar preflight tests.
//!
//! WHAT: pins `validate_completed_generated_record` at the generated boundary: a sidecar HIR
//!       carrying a structural resource string publishes with its sidecar-local resource handle,
//!       while a text-only structural sidecar stays publishable and its `Text` piece handles
//!       re-bind through a non-identity string-table merge.
//! WHY: generic capture materialises file values through a fresh sidecar resource table, while
//!      stable text handles still need the ordinary string-table remap when the generated delta
//!      joins its owning compilation.

use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::ValueKind;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::fixture_resource;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, RegionId};
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::module_compilation::generated::test_fixtures::{
    generated_identity, summary, test_sidecar,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Builds one sidecar root statement carrying a structural string with the given pieces.
fn sidecar_structural_statement(pieces: Vec<ConstStringPiece>) -> HirStatement {
    HirStatement {
        id: HirNodeId(0),
        kind: HirStatementKind::Expr(HirExpression {
            id: HirValueId(0),
            kind: HirExpressionKind::StructuralString { pieces },
            ty: builtin_type_ids::STRING,
            value_kind: ValueKind::Const,
            region: RegionId(0),
        }),
        location: SourceLocation::default(),
    }
}

#[test]
fn a_resource_bearing_generated_sidecar_publishes_and_preserves_origin() {
    let identity = generated_identity("resource-bearing");
    let mut sidecar = test_sidecar(identity.clone(), summary());

    // Mint the handle through the sidecar-local resource table used by HIR lowering.
    let mut resources = ModuleResourceTable::new();
    let (logo, origin) = fixture_resource(&mut resources, "assets/logo.svg");

    let mut string_table = StringTable::new();
    let label = string_table.intern("assets/");

    sidecar.module.executable.hir.blocks.push(HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![sidecar_structural_statement(vec![
            ConstStringPiece::Text(label),
            ConstStringPiece::Resource(logo),
        ])],
        terminator: HirTerminator::Uninitialized,
    });
    sidecar.module.executable.resource_table = resources;

    let record = CompletedGeneratedFunction {
        identity,
        summary: summary(),
        sidecar,
    };

    validate_completed_generated_record(&record)
        .expect("a resource-bearing sidecar with a local resource table should publish");

    let statement = &record.sidecar.module.executable.hir.blocks[0].statements[0];
    let HirStatementKind::Expr(expression) = &statement.kind else {
        panic!("the sidecar root should still hold the structural expression statement");
    };
    let HirExpressionKind::StructuralString { pieces } = &expression.kind else {
        panic!(
            "expected the resource-bearing structural string to survive publication, got {:?}",
            expression.kind
        );
    };
    let [
        ConstStringPiece::Text(_),
        ConstStringPiece::Resource(published_resource),
    ] = pieces.as_slice()
    else {
        panic!("expected one text piece followed by one resource piece");
    };
    assert_eq!(
        *published_resource, logo,
        "published HIR must retain the handle issued by its sidecar-local table"
    );
    assert_eq!(
        record
            .sidecar
            .module
            .executable
            .resource_table
            .try_origin(*published_resource)
            .expect("published handle must resolve in the retained sidecar table")
            .origin,
        origin
    );
}

#[test]
fn a_text_only_generated_sidecar_publishes_and_remaps_piece_handles_through_a_merge() {
    let identity = generated_identity("text-only");
    let mut sidecar = test_sidecar(identity.clone(), summary());

    let mut string_table = StringTable::new();
    let head = string_table.intern("docs/");
    let tail = string_table.intern("guide");

    sidecar.module.executable.hir.blocks.push(HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![sidecar_structural_statement(vec![
            ConstStringPiece::Text(head),
            ConstStringPiece::Text(tail),
        ])],
        terminator: HirTerminator::Uninitialized,
    });

    let record = CompletedGeneratedFunction {
        identity,
        summary: summary(),
        sidecar,
    };

    validate_completed_generated_record(&record)
        .expect("a sidecar whose structural pieces carry only text must stay publishable");

    // Merge the module-local table into a boundary table whose indices differ, exactly as a
    // module result merges before publication, then remap through the generated delta lane.
    let mut merged = StringTable::new();
    merged.intern("a string an earlier module merged first");
    let remap = merged.merge_from(&string_table);
    assert!(
        !remap.is_identity(),
        "the fixture merge must move piece handles"
    );

    let mut delta = GeneratedFunctionDelta::from_records(vec![record]);
    delta.remap_string_ids(&remap);

    let statement = &delta.records()[0].sidecar.module.executable.hir.blocks[0].statements[0];
    let HirStatementKind::Expr(remapped) = &statement.kind else {
        panic!("the sidecar root should still hold the structural expression statement");
    };
    let HirExpressionKind::StructuralString { pieces } = &remapped.kind else {
        panic!(
            "expected the text-only structural string to survive the merge, got {:?}",
            remapped.kind
        );
    };

    match pieces.as_slice() {
        [
            ConstStringPiece::Text(remapped_head),
            ConstStringPiece::Text(remapped_tail),
        ] => {
            assert_ne!(remapped_head, &head, "every text piece handle must re-bind");
            assert_ne!(remapped_tail, &tail, "every text piece handle must re-bind");

            assert_eq!(merged.resolve(*remapped_head), "docs/");
            assert_eq!(merged.resolve(*remapped_tail), "guide");
        }

        other => panic!("expected two text pieces after the merge, got {other:?}"),
    }
}
