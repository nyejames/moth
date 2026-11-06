//! Tests for HTML builder root-slot interpretation.
//!
//! WHAT: exercises metadata concatenation, structural URL rendering, stylesheet classification
//!       and identity deduplication, and head contribution deduplication.
//! WHY: these policies shared by the JS and Wasm paths are builder-owned and must stay
//!      independent of the compiler's generic slot routing.

use super::*;
use crate::compiler_frontend::builder_root_slots::BuilderRootSlotName;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidHtmlSlotContributionView;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, InvalidHtmlSlotContributionReason,
};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::module_compilation::ResolvedConstFragment;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext, ResourceUseKind,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use std::path::{Path, PathBuf};

fn named_fragment(name: &str, value: OwnedFoldedString) -> ResolvedConstFragment {
    named_fragment_at(name, value, 0)
}

fn named_fragment_at(
    name: &str,
    value: OwnedFoldedString,
    runtime_insertion_index: usize,
) -> ResolvedConstFragment {
    ResolvedConstFragment {
        runtime_insertion_index,
        destination: ConstFragmentDestination::Named(BuilderRootSlotName::new(name)),
        span: None,
        value,
    }
}

fn body_fragment(value: OwnedFoldedString) -> ResolvedConstFragment {
    ResolvedConstFragment {
        runtime_insertion_index: 0,
        destination: ConstFragmentDestination::Body,
        span: None,
        value,
    }
}

/// Intern one fixture origin through the same stable identity machinery as production.
fn test_origin(relative_path: &str) -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("html-slot-plan-tests"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(Path::new(relative_path))
            .expect("fixture resource path should be portable"),
    )
}

fn page_context() -> ResourceUrlContext {
    ResourceUrlContext::PageDocument(PathBuf::from("index.html"))
}

fn nested_page_context() -> ResourceUrlContext {
    ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"))
}

fn plan_with_origins(
    origins: &[StableResourceOriginId],
    context: &ResourceUrlContext,
) -> HtmlResourceOutputPlan {
    let mut plan = HtmlResourceOutputPlan::new("html-slot-plan-tests");
    let mut string_table = StringTable::new();
    {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
        for origin in origins {
            plan.plan_origin(
                origin.clone(),
                None,
                context.clone(),
                &mut diagnostic_strings,
                ResourceUseKind::Metadata,
            )
            .expect("fixture resource origin should plan");
        }
    }
    plan
}

fn plan_text_slots(
    fragments: &[ResolvedConstFragment],
) -> Result<HtmlSlotPlan, crate::compiler_frontend::compiler_messages::PreparedFailure> {
    let context = page_context();
    let output_plan = HtmlResourceOutputPlan::new("html-slot-plan-tests");
    let renderer = StructuralUrlRenderer::new(&output_plan, &context, "/");
    let mut string_table = StringTable::new();
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
    plan_html_entry_slots(fragments, &renderer, None, &mut diagnostic_strings)
}

#[test]
fn metadata_concatenates_in_fragment_order_and_keeps_explicit_empty() {
    let fragments = vec![
        body_fragment(OwnedFoldedString::Text(String::from("ignored body"))),
        named_fragment_at("title", OwnedFoldedString::Text(String::from("First ")), 2),
        named_fragment_at("description", OwnedFoldedString::Text(String::new()), 0),
        named_fragment_at("title", OwnedFoldedString::Text(String::from("Second")), 1),
        named_fragment_at("lang", OwnedFoldedString::Text(String::new()), 0),
        named_fragment_at(
            "body_style",
            OwnedFoldedString::Text(String::from("padding:0;")),
            0,
        ),
    ];

    let plan = plan_text_slots(&fragments).expect("text slots should plan");

    assert_eq!(plan.metadata.title.as_deref(), Some("First Second"));
    assert_eq!(plan.metadata.description.as_deref(), Some(""));
    assert_eq!(plan.metadata.lang.as_deref(), Some(""));
    assert_eq!(plan.metadata.body_style.as_deref(), Some("padding:0;"));
    assert_eq!(plan.metadata.favicon, None);
}

#[test]
fn structural_metadata_renders_resources_in_page_url_context() {
    let favicon = test_origin("static/favicon.png");
    let context = page_context();
    let output_plan = plan_with_origins(std::slice::from_ref(&favicon), &context);
    let renderer = StructuralUrlRenderer::new(&output_plan, &context, "/");
    let fragments = vec![named_fragment(
        "favicon",
        OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(favicon)]),
    )];
    let mut string_table = StringTable::new();
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);

    let plan = plan_html_entry_slots(&fragments, &renderer, None, &mut diagnostic_strings)
        .expect("structural favicon should plan");

    assert_eq!(
        plan.metadata.favicon.as_deref(),
        Some("./static/favicon.png")
    );
}

#[test]
fn stylesheets_render_once_per_stable_origin_in_first_contribution_order() {
    let first = test_origin("styles/first.css");
    let second = test_origin("styles/second.css");
    let context = page_context();
    let output_plan = plan_with_origins(&[first.clone(), second.clone()], &context);
    let renderer = StructuralUrlRenderer::new(&output_plan, &context, "/");
    let fragments = vec![
        named_fragment(
            "stylesheets",
            OwnedFoldedString::Pieces(vec![
                OwnedFoldedStringPiece::Text(String::from("  ")),
                OwnedFoldedStringPiece::Resource(first.clone()),
                OwnedFoldedStringPiece::Text(String::from("\n")),
            ]),
        ),
        named_fragment(
            "stylesheets",
            OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(second.clone())]),
        ),
        named_fragment(
            "stylesheets",
            OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(first.clone())]),
        ),
    ];
    let mut string_table = StringTable::new();
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);

    let plan = plan_html_entry_slots(&fragments, &renderer, None, &mut diagnostic_strings)
        .expect("valid stylesheet contributions should plan");

    assert_eq!(
        plan.stylesheet_hrefs,
        vec![
            String::from("./styles/first.css"),
            String::from("./styles/second.css")
        ]
    );
}

#[test]
fn stylesheet_classification_rejects_every_invalid_contribution_shape() {
    let css = test_origin("styles/site.css");
    let other_css = test_origin("styles/other.css");
    let image = test_origin("static/icon.png");

    let plain_path = OwnedFoldedString::Text(String::from("styles/site.css"));
    assert_eq!(
        classify_stylesheet_contribution(&plain_path),
        Err(InvalidHtmlSlotContributionReason::NotAStaticResource)
    );

    let whitespace_only = OwnedFoldedString::Text(String::from("  \n"));
    assert_eq!(
        classify_stylesheet_contribution(&whitespace_only),
        Err(InvalidHtmlSlotContributionReason::MissingResource)
    );

    let site_root = OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::SiteRoot]);
    assert_eq!(
        classify_stylesheet_contribution(&site_root),
        Err(InvalidHtmlSlotContributionReason::SiteRoot)
    );

    let multiple = OwnedFoldedString::Pieces(vec![
        OwnedFoldedStringPiece::Resource(css.clone()),
        OwnedFoldedStringPiece::Resource(other_css),
    ]);
    assert_eq!(
        classify_stylesheet_contribution(&multiple),
        Err(InvalidHtmlSlotContributionReason::MultipleResources)
    );

    let combined = OwnedFoldedString::Pieces(vec![
        OwnedFoldedStringPiece::Text(String::from("prefix")),
        OwnedFoldedStringPiece::Resource(css.clone()),
    ]);
    assert_eq!(
        classify_stylesheet_contribution(&combined),
        Err(InvalidHtmlSlotContributionReason::NotAStaticResource)
    );

    let non_css = OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(image)]);
    assert_eq!(
        classify_stylesheet_contribution(&non_css),
        Err(InvalidHtmlSlotContributionReason::NonCssResource)
    );

    let padded = OwnedFoldedString::Pieces(vec![
        OwnedFoldedStringPiece::Text(String::from(" \n")),
        OwnedFoldedStringPiece::Resource(css),
        OwnedFoldedStringPiece::Text(String::from("\t")),
    ]);
    assert_eq!(
        classify_stylesheet_contribution(&padded)
            .expect("formatting whitespace around one CSS origin is valid"),
        &test_origin("styles/site.css")
    );
}

#[test]
fn head_keeps_first_identical_complete_contribution_in_order() {
    let fragments = vec![
        named_fragment(
            "head",
            OwnedFoldedString::Text(String::from("<meta name=\"a\">")),
        ),
        named_fragment(
            "head",
            OwnedFoldedString::Text(String::from("<meta name=\"a\">")),
        ),
        named_fragment(
            "head",
            OwnedFoldedString::Text(String::from("<meta name=\"a\"><meta name=\"a\">")),
        ),
    ];

    let plan = plan_text_slots(&fragments).expect("head contributions should plan");

    assert_eq!(
        plan.extra_head_html,
        "<meta name=\"a\"><meta name=\"a\"><meta name=\"a\">"
    );
}

#[test]
fn head_deduplicates_after_structural_url_resolution_in_nested_route() {
    let stylesheet = test_origin("styles/docs.css");
    let context = nested_page_context();
    let output_plan = plan_with_origins(std::slice::from_ref(&stylesheet), &context);
    let renderer = StructuralUrlRenderer::new(&output_plan, &context, "/");
    let fragments = vec![
        named_fragment(
            "head",
            OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(stylesheet)]),
        ),
        named_fragment(
            "head",
            OwnedFoldedString::Text(String::from("../styles/docs.css")),
        ),
    ];
    let mut string_table = StringTable::new();
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);

    let plan = plan_html_entry_slots(&fragments, &renderer, None, &mut diagnostic_strings)
        .expect("head contributions should plan");

    assert_eq!(plan.extra_head_html, "../styles/docs.css");
}

#[test]
fn invalid_stylesheet_contribution_diagnostic_uses_stable_family_and_reason() {
    let mut string_table = StringTable::new();
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
    let slot = diagnostic_strings.intern("stylesheets");
    let mut span_builder = ExtendedSpanBuilder::default();
    let span = SourceSpan::new(
        SourceId::from_index(0),
        LocalSpan::exact(3, 12, &mut span_builder).expect("fixture span should fit"),
    );
    let draft = project_config::invalid_html_slot_contribution(
        slot,
        InvalidHtmlSlotContributionReason::MultipleResources,
        Some(span),
    );

    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = compact
        .first()
        .expect("slot-contribution diagnostic should compact");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0061");
    let facts = diagnostic
        .family_view::<InvalidHtmlSlotContributionView>()
        .expect("slot-contribution diagnostic should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidHtmlSlotContributionReason::MultipleResources
    );

    let message =
        terminal::format_guidance_view(diagnostic, DiagnosticRenderContext::new(&string_table))
            .join("\n");
    assert!(
        message.contains("`stylesheets`") && message.contains("more than one"),
        "unexpected message: {message}"
    );
}

#[test]
fn invalid_stylesheet_planning_reports_a_diagnosed_failure() {
    let fragments = vec![named_fragment(
        "stylesheets",
        OwnedFoldedString::Text(String::from("https://example.com/site.css")),
    )];

    let error = plan_text_slots(&fragments).expect_err("external URL must reject");

    assert!(matches!(
        error,
        crate::compiler_frontend::compiler_messages::PreparedFailure::Diagnosed(_)
    ));
}
