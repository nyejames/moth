use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation as MessageSourceLocation;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::reachability::{HirReachability, ReachableSiteRootUse};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext,
};
use std::path::{Path, PathBuf};

impl<'a> StructuralUrlRenderer<'a> {
    /// Render HIR structural pieces, resolving module-local resource IDs through their owner table.
    pub(crate) fn render_hir_pieces(
        &self,
        pieces: &[ConstStringPiece],
        resources: &ModuleResourceTable,
        string_table: &StringTable,
    ) -> Result<String, CompilerError> {
        let mut rendered = String::new();
        for piece in pieces {
            match piece {
                ConstStringPiece::Text(text) => rendered.push_str(string_table.resolve(*text)),
                ConstStringPiece::Resource(resource_id) => {
                    let origin = &resources.try_origin(*resource_id)?.origin;
                    rendered.push_str(&self.render_resource_origin(origin)?)
                }
                ConstStringPiece::SiteRoot => rendered.push_str(&self.render_site_root_url()?),
            }
        }
        Ok(rendered)
    }
}

fn origin(module_path: &str, resource_path: &str) -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("renderer-tests"),
            module_path.to_owned(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(Path::new(resource_path))
            .expect("fixture resource path should be portable"),
    )
}

fn renderer<'a>(
    plan: &'a HtmlResourceOutputPlan,
    context: &'a ResourceUrlContext,
    site_origin: Option<&'a str>,
) -> StructuralUrlRenderer<'a> {
    StructuralUrlRenderer::new(plan, context, site_origin)
}

fn plan_origin_for(
    plan: &mut HtmlResourceOutputPlan,
    resource_origin: StableResourceOriginId,
    context: ResourceUrlContext,
    string_table: &mut StringTable,
) {
    plan.plan_origin(
        resource_origin,
        MessageSourceLocation::default(),
        context,
        string_table,
        true,
    )
    .expect("resource should be planned");
}

#[test]
fn nested_route_resource_is_relative_to_context_parent() {
    let resource_origin = origin("docs/getting-started", "assets/logo.svg");
    let mut plan = HtmlResourceOutputPlan::new("renderer-tests");
    let context =
        ResourceUrlContext::PageDocument(PathBuf::from("docs/getting-started/index.html"));
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        context.clone(),
        &mut StringTable::new(),
    );

    let rendered = renderer(&plan, &context, Some("/moth"))
        .render_owned(&OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Resource(resource_origin),
        ]))
        .expect("resource URL should render");

    assert_eq!(rendered, "./assets/logo.svg");
}

#[test]
fn parent_relative_resource_retains_parent_segments() {
    let resource_origin = origin("docs", "assets/logo.svg");
    let mut plan = HtmlResourceOutputPlan::new("renderer-tests");
    let context =
        ResourceUrlContext::PageDocument(PathBuf::from("docs/getting-started/index.html"));
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        context.clone(),
        &mut StringTable::new(),
    );

    let rendered = renderer(&plan, &context, Some("/"))
        .render_owned(&OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Resource(resource_origin),
        ]))
        .expect("resource URL should render");

    assert_eq!(rendered, "../assets/logo.svg");
}

#[test]
fn resource_segments_are_percent_encoded_as_utf8() {
    let resource_origin = origin("docs/getting-started", "assets/my logo-é.svg");
    let mut plan = HtmlResourceOutputPlan::new("renderer-tests");
    let context =
        ResourceUrlContext::PageDocument(PathBuf::from("docs/getting-started/index.html"));
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        context.clone(),
        &mut StringTable::new(),
    );

    let rendered = renderer(&plan, &context, Some("/docs"))
        .render_owned(&OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Resource(resource_origin),
        ]))
        .expect("resource URL should render");

    assert_eq!(rendered, "./assets/my%20logo-%C3%A9.svg");
    assert!(!rendered.contains("/docs"));
}

#[test]
fn one_planned_origin_renders_relative_to_each_consuming_page() {
    let resource_origin = origin("assets", "logo.svg");
    let mut plan = HtmlResourceOutputPlan::new("renderer-tests");
    let first_context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let second_context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    let mut string_table = StringTable::new();
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        first_context.clone(),
        &mut string_table,
    );
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        second_context.clone(),
        &mut string_table,
    );
    let value = OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(resource_origin)]);

    assert_eq!(
        renderer(&plan, &first_context, Some("/docs"))
            .render_owned(&value)
            .unwrap(),
        "./assets/logo.svg"
    );
    assert_eq!(
        renderer(&plan, &second_context, Some("/docs"))
            .render_owned(&value)
            .unwrap(),
        "../assets/logo.svg"
    );
}

#[test]
fn stylesheet_and_page_contexts_use_their_own_parents() {
    let resource_origin = origin("assets", "site.css");
    let mut plan = HtmlResourceOutputPlan::new("renderer-tests");
    let page_context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    let stylesheet_context =
        ResourceUrlContext::Stylesheet(PathBuf::from("styles/nested/main.css"));
    let mut string_table = StringTable::new();
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        page_context.clone(),
        &mut string_table,
    );
    plan_origin_for(
        &mut plan,
        resource_origin.clone(),
        stylesheet_context.clone(),
        &mut string_table,
    );
    let value = OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(resource_origin)]);

    assert_eq!(
        renderer(&plan, &page_context, Some("/"))
            .render_owned(&value)
            .unwrap(),
        "../assets/site.css"
    );
    assert_eq!(
        renderer(&plan, &stylesheet_context, Some("/"))
            .render_owned(&value)
            .unwrap(),
        "../../assets/site.css"
    );
    assert_ne!(
        renderer(&plan, &page_context, Some("/"))
            .render_owned(&value)
            .unwrap(),
        renderer(&plan, &stylesheet_context, Some("/"))
            .render_owned(&value)
            .unwrap(),
        "page and stylesheet parents must produce different relatives"
    );
}

#[test]
fn site_root_uses_consuming_origin_policy() {
    let plan = HtmlResourceOutputPlan::new("renderer-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    let value = OwnedFoldedString::Pieces(vec![
        OwnedFoldedStringPiece::SiteRoot,
        OwnedFoldedStringPiece::Text(String::from("docs/")),
    ]);

    assert_eq!(
        renderer(&plan, &context, Some("/"))
            .render_owned(&value)
            .unwrap(),
        "/docs/"
    );
    assert_eq!(
        renderer(&plan, &context, Some("/moth"))
            .render_owned(&value)
            .unwrap(),
        "/moth/docs/"
    );
}

#[test]
fn no_origin_policy_rejects_reachable_site_root_use() {
    let mut reachability = HirReachability::default();
    reachability
        .reachable_site_root_uses
        .push(ReachableSiteRootUse {
            owner: FunctionId(0),
            location: SourceLocation::default(),
        });

    let result = validate_site_root_policy(&reachability, false, None);

    assert!(result.is_err());
}

#[test]
fn hir_resource_id_renders_through_its_module_table() {
    let resource_origin = origin("docs", "assets/logo.svg");
    let mut resources = ModuleResourceTable::new();
    let resource_id = resources.intern_origin(resource_origin.clone(), SourceLocation::default());
    let mut plan = HtmlResourceOutputPlan::new("renderer-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    let mut string_table = StringTable::new();
    plan_origin_for(
        &mut plan,
        resource_origin,
        context.clone(),
        &mut string_table,
    );

    let rendered = renderer(&plan, &context, Some("/"))
        .render_hir_pieces(
            &[ConstStringPiece::Resource(resource_id)],
            &resources,
            &string_table,
        )
        .expect("HIR resource piece should render");

    assert_eq!(rendered, "./assets/logo.svg");
}
