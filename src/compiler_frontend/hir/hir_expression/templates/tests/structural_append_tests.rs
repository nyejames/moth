//! Constant-side append composition tests for the runtime-template append seam.
//!
//! WHAT: pins the piece-list behaviour of `append_chunk_to_rendered_expression` when both
//!       operands are known constants: authored piece order survives, only adjacent `Text`
//!       runs fuse, a `Resource` or `SiteRoot` anchor stays a hard text-coalescing boundary,
//!       and an anchor-free result demotes back to the plain `StringLiteral` fast path.
//! WHY: the runtime handoff materializes structural strings as piece-bearing text, so these
//!       shapes are produced for every folded template. A regression that fused text through an
//!       anchor or kept an allocating piece vector for plain text would silently change every
//!       folded template string without any diagnostic.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, fixture_resource, setup_builder};
use crate::compiler_frontend::hir::ids::RegionId;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

fn structural_expression(
    builder: &mut HirBuilder<'_>,
    location: &SourceLocation,
    pieces: Vec<ConstStringPiece>,
) -> HirExpression {
    builder.make_expression(
        location,
        HirExpressionKind::StructuralString { pieces },
        builtin_type_ids::STRING,
        ValueKind::Const,
        RegionId(0),
    )
}

fn text_expression(
    builder: &mut HirBuilder<'_>,
    location: &SourceLocation,
    text: &str,
) -> HirExpression {
    builder.make_expression(
        location,
        HirExpressionKind::StringLiteral(text.to_owned()),
        builtin_type_ids::STRING,
        ValueKind::Const,
        RegionId(0),
    )
}

fn append_constant_chunk(
    builder: &mut HirBuilder<'_>,
    rendered: &mut HirExpression,
    chunk: HirExpression,
    location: &SourceLocation,
) {
    builder
        .append_chunk_to_rendered_expression(
            rendered,
            chunk,
            location,
            builtin_type_ids::STRING,
            RegionId(0),
        )
        .expect("constant string operands should compose without a diagnostic");
}

#[test]
fn appending_text_to_a_piece_bearing_string_keeps_the_resource_anchor_boundary() {
    let mut string_table = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let (logo, _origin) = fixture_resource(&mut resources, "assets/logo.svg");

    let authored = string_table.intern("assets/");
    let appended = string_table.intern("-thumbnail.svg");
    let location = SourceLocation::default();

    let mut builder = setup_builder(&mut string_table);
    let mut rendered = structural_expression(
        &mut builder,
        &location,
        vec![
            ConstStringPiece::Text(authored),
            ConstStringPiece::Resource(logo),
        ],
    );

    let chunk = text_expression(&mut builder, &location, "-thumbnail.svg");
    append_constant_chunk(&mut builder, &mut rendered, chunk, &location);

    let HirExpressionKind::StructuralString { pieces } = &rendered.kind else {
        panic!(
            "appending beside a resource anchor must stay structural, got {:?}",
            rendered.kind
        );
    };

    let expected = vec![
        ConstStringPiece::Text(authored),
        ConstStringPiece::Resource(logo),
        ConstStringPiece::Text(appended),
    ];
    assert_eq!(
        pieces, &expected,
        "appended text must join after the resource anchor as its own piece, never fused through it"
    );
}

#[test]
fn appending_two_piece_bearing_strings_fuses_the_join_but_never_across_an_anchor() {
    let mut string_table = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let (logo, _origin) = fixture_resource(&mut resources, "assets/logo.svg");

    let authored = string_table.intern("assets/");
    let left_run = string_table.intern("docs/guide");
    let right_run_head = string_table.intern("-v2");
    let right_run_tail = string_table.intern(".html");
    let after_anchor = string_table.intern("index");

    // A `Text` run ends the left operand and begins the right one, so the join itself fuses;
    // the two expected neighbours of that run sit behind anchors and must stay separate pieces.
    let fused_join = string_table.intern("docs/guide-v2.html");
    assert_eq!(string_table.resolve(fused_join), "docs/guide-v2.html");

    let location = SourceLocation::default();

    let mut builder = setup_builder(&mut string_table);
    let mut rendered = structural_expression(
        &mut builder,
        &location,
        vec![
            ConstStringPiece::Text(authored),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(left_run),
        ],
    );
    let chunk = structural_expression(
        &mut builder,
        &location,
        vec![
            ConstStringPiece::Text(right_run_head),
            ConstStringPiece::Text(right_run_tail),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(after_anchor),
        ],
    );

    append_constant_chunk(&mut builder, &mut rendered, chunk, &location);

    let HirExpressionKind::StructuralString { pieces } = &rendered.kind else {
        panic!(
            "appending two anchor-bearing strings must stay structural, got {:?}",
            rendered.kind
        );
    };

    let expected = vec![
        ConstStringPiece::Text(authored),
        ConstStringPiece::Resource(logo),
        ConstStringPiece::Text(fused_join),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::Text(after_anchor),
    ];
    assert_eq!(
        pieces, &expected,
        "both piece sequences survive in authored order; the join fuses into one run while \
         the resource and site-root anchors keep their neighbours separate"
    );
}

#[test]
fn anchor_free_append_demotes_to_a_plain_string_literal() {
    let mut string_table = StringTable::new();

    let head = string_table.intern("docs");
    let separator = string_table.intern("/");
    let tail = string_table.intern("index.html");
    let location = SourceLocation::default();

    let mut builder = setup_builder(&mut string_table);
    let mut rendered = structural_expression(
        &mut builder,
        &location,
        vec![
            ConstStringPiece::Text(head),
            ConstStringPiece::Text(separator),
        ],
    );
    let chunk = structural_expression(&mut builder, &location, vec![ConstStringPiece::Text(tail)]);

    append_constant_chunk(&mut builder, &mut rendered, chunk, &location);

    // The composed chunk carries no `Resource` or `SiteRoot` anchor, so it must come back on
    // the non-allocating plain-text path instead of a piece vector.
    let HirExpressionKind::StringLiteral(text) = &rendered.kind else {
        panic!(
            "an anchor-free append must demote to a plain StringLiteral, got {:?}",
            rendered.kind
        );
    };
    assert_eq!(text, "docs/index.html");
}
