use super::*;
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::builder_root_slots::{BuilderRootSlotName, ConstFragmentDestination};

use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use rustc_hash::FxHashMap;

#[test]
fn standalone_insert_helper_value_is_rejected_after_composition() {
    let source = r#"
value = [$in style: color: red;]
"#;

    let failure = parse_single_file_ast_diagnostic(source);
    crate::compiler_frontend::tests::parse_support::assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0022",
        Some("invalid_template_structure.helper_outside_wrapper_slot"),
    );
}

#[test]
fn finalized_module_constants_materialize_const_templates_before_hir() {
    let source = r#"
wrapper #= [:<div class="frame">[$slot]</div>]
content #= [wrapper: [:Hello]]
"#;

    let (ast, path_fork, string_table) = parse_single_file_ast(source);

    let wrapper_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| {
            path_fork
                .component(*row.path)
                .is_some_and(|name| string_table.resolve(name) == "wrapper")
        })
        .expect("wrapper constant should exist")
        .id;
    let content_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| {
            path_fork
                .component(*row.path)
                .is_some_and(|name| string_table.resolve(name) == "content")
        })
        .expect("content constant should exist")
        .id;

    let wrapper_value = ast
        .const_values
        .string_value(wrapper_id)
        .expect("wrapper template should already be materialized before HIR");
    let content_value = ast
        .const_values
        .string_value(content_id)
        .expect("const template application should already be materialized before HIR");

    assert_eq!(
        string_table.resolve(wrapper_value),
        "<div class=\"frame\"></div>"
    );
    assert_eq!(
        string_table.resolve(content_value),
        "<div class=\"frame\"> Hello</div>"
    );
}

#[test]
fn collects_const_top_level_fragments_from_tir_result_record() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let value = string_table.intern("const html");

    let mut results = FxHashMap::default();
    results.insert(
        path,
        FoldedConstTemplateResult::new(
            ConstStringValue::Text(value),
            ConstFragmentDestination::Body,
        ),
    );

    let fragments = vec![TopLevelConstFragment {
        runtime_insertion_index: 0,
        header_path: path,
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    }];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].runtime_insertion_index, 0);
    assert_eq!(collected[0].span, fragments[0].span);
    assert_eq!(collected[0].destination, ConstFragmentDestination::Body);
    assert_eq!(collected[0].value, ConstStringValue::Text(value));
}

#[test]
fn collects_const_top_level_fragments_from_folded_value() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let value = string_table.intern("folded html");

    let mut results = FxHashMap::default();
    results.insert(
        path,
        FoldedConstTemplateResult::new(
            ConstStringValue::Text(value),
            ConstFragmentDestination::Body,
        ),
    );

    let fragments = vec![TopLevelConstFragment {
        runtime_insertion_index: 2,
        header_path: path,
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    }];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].runtime_insertion_index, 2);
    assert_eq!(collected[0].value, ConstStringValue::Text(value));
}

#[test]
fn collects_mixed_const_top_level_fragments_in_source_order() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let first_path = path_fork
        .try_intern_portable_path("first.moth", &mut string_table)
        .expect("test path fits");
    let second_path = path_fork
        .try_intern_portable_path("second.moth", &mut string_table)
        .expect("test path fits");

    let first_value = string_table.intern("first");
    let second_value = string_table.intern("second");

    let mut results = FxHashMap::default();
    results.insert(
        first_path,
        FoldedConstTemplateResult::new(
            ConstStringValue::Text(first_value),
            ConstFragmentDestination::Body,
        ),
    );
    results.insert(
        second_path,
        FoldedConstTemplateResult::new(
            ConstStringValue::Text(second_value),
            ConstFragmentDestination::Named(BuilderRootSlotName::new("sidebar")),
        ),
    );

    let fragments = vec![
        TopLevelConstFragment {
            runtime_insertion_index: 1,
            header_path: first_path,
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        },
        TopLevelConstFragment {
            runtime_insertion_index: 3,
            header_path: second_path,
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        },
    ];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0].runtime_insertion_index, 1);
    assert_eq!(collected[0].destination, ConstFragmentDestination::Body);
    assert_eq!(collected[0].value, ConstStringValue::Text(first_value));
    assert_eq!(collected[1].runtime_insertion_index, 3);
    assert_eq!(
        collected[1].destination,
        ConstFragmentDestination::Named(BuilderRootSlotName::new("sidebar"))
    );
    assert_eq!(collected[1].span, fragments[1].span);
    assert_eq!(collected[1].value, ConstStringValue::Text(second_value));
}

/// Interns one fixture resource origin for a piece-bearing collection test.
fn fixture_resource_id() -> ResourceId {
    let origin = StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("top-level-template-tests"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(std::path::Path::new("assets/logo.svg"))
            .expect("fixture resource path should be portable"),
    );

    let mut resources = ModuleResourceTable::new();
    resources.intern_origin(origin, None)
}

#[test]
fn collects_piece_bearing_and_plain_const_top_level_fragments_unchanged() {
    // WHAT: collection carries text, resource, and site-root pieces, alongside ordinary text
    //      and all-text structural fragments, without flattening either structural value.
    // WHY: structural const values must remain in the AST-local vocabulary until service conversion
    //      and the builder's final-text boundary.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let piece_path = path_fork
        .try_intern_portable_path("piece.moth", &mut string_table)
        .expect("test path fits");
    let all_text_path = path_fork
        .try_intern_portable_path("all-text.moth", &mut string_table)
        .expect("test path fits");
    let text_path = path_fork
        .try_intern_portable_path("text.moth", &mut string_table)
        .expect("test path fits");
    let prefix = string_table.intern("before");
    let suffix = string_table.intern("after");
    let all_text_prefix = string_table.intern("all-text-before");
    let all_text_suffix = string_table.intern("all-text-after");
    let plain_text = string_table.intern("plain");
    let resource = fixture_resource_id();
    let piece_value = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(resource),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::Text(suffix),
    ]);
    let all_text_value = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(all_text_prefix),
        ConstStringPiece::Text(all_text_suffix),
    ]);

    let mut results = FxHashMap::default();
    results.insert(
        piece_path,
        FoldedConstTemplateResult::new(piece_value.clone(), ConstFragmentDestination::Body),
    );
    results.insert(
        all_text_path,
        FoldedConstTemplateResult::new(all_text_value.clone(), ConstFragmentDestination::Body),
    );
    results.insert(
        text_path,
        FoldedConstTemplateResult::new(
            ConstStringValue::Text(plain_text),
            ConstFragmentDestination::Body,
        ),
    );

    let fragments = vec![
        TopLevelConstFragment {
            runtime_insertion_index: 1,
            header_path: piece_path,
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        },
        TopLevelConstFragment {
            runtime_insertion_index: 2,
            header_path: all_text_path,
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        },
        TopLevelConstFragment {
            runtime_insertion_index: 3,
            header_path: text_path,
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        },
    ];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 3);
    assert_eq!(collected[0].runtime_insertion_index, 1);
    assert_eq!(collected[0].value, piece_value);
    assert_eq!(collected[1].runtime_insertion_index, 2);
    assert_eq!(
        collected[1].value, all_text_value,
        "all-text structural fragments must remain Pieces with their exact ordered text runs"
    );
    assert_eq!(collected[2].runtime_insertion_index, 3);
    assert_eq!(
        collected[2].value,
        ConstStringValue::Text(plain_text),
        "ordinary text fragments must remain text values"
    );
}

#[test]
fn missing_const_top_level_fragment_result_returns_infrastructure_failure() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");

    let results = FxHashMap::<PathId, FoldedConstTemplateResult>::default();
    let fragments = vec![TopLevelConstFragment {
        runtime_insertion_index: 0,
        header_path: path,
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
    }];

    let error = collect_const_top_level_fragments(&fragments, &results)
        .expect_err("missing result should fail");

    assert!(
        format!("{:?}", error).contains("no corresponding folded template value"),
        "error should identify missing folded template value: {:?}",
        error
    );
}
