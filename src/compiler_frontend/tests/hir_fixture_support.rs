//! HIR fixture support for frontend and backend unit tests.
//!
//! WHAT: wraps synthetic AST-to-HIR lowering, and constructs HIR expression, statement and
//!       local nodes directly for tests that start below lowering.
//! WHY: these helpers sit at the HIR boundary and must not depend on borrow validation. HIR is
//!      the first backend-facing semantic IR, so node construction is language-owned rather than
//!      target-owned and both backends consume these constructors instead of copying them.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::hir_builder::lower_module;
use crate::compiler_frontend::hir::ids::{HirNodeId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn entry_and_start(string_table: &mut StringTable) -> (InternedPath, InternedPath) {
    let entry_path = InternedPath::from_single_str("main.moth", string_table);
    let start_name = entry_path.join_str(IMPLICIT_START_FUNC_NAME, string_table);
    (entry_path, start_name)
}

pub(crate) fn lower_hir(ast: Ast, string_table: &mut StringTable) -> HirModule {
    let lowering = lower_module(ast, string_table, HirFunctionOriginLookup::default(), None)
        .expect("HIR lowering should succeed");
    lowering.hir_module
}

/// Builds a malformed raw-template expression for HIR boundary invariant tests.
///
/// AST finalization should replace this shape with an owned runtime handoff. The
/// returned shared store keeps the deliberately unnormalized template's TIR identity valid.
pub(crate) fn raw_template_expression_for_hir_invariant(
    kind: TemplateType,
    location: SourceLocation,
    value_mode: ValueMode,
) -> (Expression, Rc<RefCell<TemplateIrStore>>) {
    let store_handle = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = store_handle.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_sequence_node(vec![], location.clone());
        builder.finish_template(
            root,
            Style::default(),
            kind.clone(),
            TemplateIrSummary::empty(),
            location.clone(),
        )
    };
    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
        location,
    };

    (Expression::template(template, value_mode), store_handle)
}

// ---------------------------------------------------------------------------
//  Direct HIR node construction
//
//  These build plain HIR nodes with no target policy in them. Both backends consume them so a
//  HIR node-shape change is a single edit. Target-specific fixture setup (each backend's
//  `build_type_environment` and `build_module`) stays with its backend.
// ---------------------------------------------------------------------------

pub(crate) fn expression(
    id: u32,
    kind: HirExpressionKind,
    ty: TypeId,
    region: RegionId,
    value_kind: ValueKind,
) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind,
        ty,
        value_kind,
        region,
    }
}

pub(crate) fn unit_expression(id: u32, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::TupleConstruct { elements: vec![] },
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn int_expression(id: u32, value: i32, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Int(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn bool_expression(id: u32, value: bool, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Bool(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn string_expression(
    id: u32,
    value: &str,
    ty: TypeId,
    region: RegionId,
) -> HirExpression {
    expression(
        id,
        HirExpressionKind::StringLiteral(value.to_owned()),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn statement(id: u32, kind: HirStatementKind, line: i32) -> HirStatement {
    HirStatement {
        id: HirNodeId(id),
        kind,
        location: test_source_location(line),
    }
}

pub(crate) fn local(local_id: u32, ty: TypeId, region: RegionId) -> HirLocal {
    HirLocal {
        id: LocalId(local_id),
        ty,
        mutable: true,
        region,
        source_info: Some(test_source_location(1)),
    }
}
