//! Guaranteed-output classification tests for the runtime slot-application walker.
//!
//! WHAT: pins what `owned_runtime_template_node_guarantees_output` reports for piece-bearing
//!       owned text nodes alongside the plain-text classification it must not change.
//! WHY: the runtime handoff materializes resource and site-root pieces inside ordinary `Text`
//!       nodes, so a regression classifying such a node as producing no output would silence
//!       wrapper output flags without any other test noticing.

use crate::compiler_frontend::ast::templates::OwnedRuntimeTemplateNode;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::path::Path;

fn fixture_resource_origin(relative_path: &str) -> StableResourceOriginId {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("hir-slot-guarantee-tests"),
        String::new(),
        ModuleRootRole::Normal,
    );

    let resource_path = PortableResourcePath::from_relative_logical_path(Path::new(relative_path))
        .expect("fixture resource path should be portable");

    StableResourceOriginId::module_owned(module_origin, resource_path)
}

fn piece_bearing_text_node(pieces: Vec<OwnedFoldedStringPiece>) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::Text {
        text: OwnedFoldedString::Pieces(pieces),
        reactive_subscription: None,
        location: SourceLocation::default(),
    }
}

fn plain_text_node(text: &str) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::Text {
        text: OwnedFoldedString::Text(text.to_owned()),
        reactive_subscription: None,
        location: SourceLocation::default(),
    }
}

#[test]
fn text_node_with_a_resource_piece_guarantees_output() {
    let string_table = StringTable::new();

    let node = piece_bearing_text_node(vec![OwnedFoldedStringPiece::Resource(
        fixture_resource_origin("assets/logo.svg"),
    )]);

    assert!(
        super::owned_runtime_template_node_guarantees_output(&node, &string_table),
        "a resource piece renders a URL once the build assigns contexts, so the node must keep the wrapper's unconditional-output guarantee"
    );
}

#[test]
fn text_node_with_a_site_root_piece_guarantees_output() {
    let string_table = StringTable::new();
    let node = piece_bearing_text_node(vec![
        OwnedFoldedStringPiece::SiteRoot,
        OwnedFoldedStringPiece::Text("   ".to_owned()),
    ]);

    assert!(
        super::owned_runtime_template_node_guarantees_output(&node, &string_table),
        "a site-root piece guarantees output even when every literal piece is blank"
    );
}

#[test]
fn plain_text_and_all_text_pieces_keep_the_trimming_classification() {
    let string_table = StringTable::new();

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &plain_text_node("hello"),
            &string_table
        ),
        "nonempty plain text keeps its guarantee"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(&plain_text_node(""), &string_table),
        "empty plain text still produces no output"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &plain_text_node("   "),
            &string_table
        ),
        "blank plain text still produces no output"
    );

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![OwnedFoldedStringPiece::Text("hello".to_owned())]),
            &string_table
        ),
        "an all-text piece list with literal runs keeps the same guarantee as plain text"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![
                OwnedFoldedStringPiece::Text(" ".to_owned()),
                OwnedFoldedStringPiece::Text(String::new()),
            ]),
            &string_table
        ),
        "an all-text piece list of blank runs still produces no output"
    );

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![
                OwnedFoldedStringPiece::Text("a".to_owned()),
                OwnedFoldedStringPiece::Text(" ".to_owned()),
            ]),
            &string_table
        ),
        "an all-text piece list with one nonempty run keeps its guarantee"
    );
}
