//! Structured external symbol path tests.
//!
//! WHAT: exercises `ExternalSymbolPath` construction, validation, and display.
//! WHY: these invariants live in the external package surface and cannot be inspected from
//!      rendered output.

use crate::compiler_frontend::external_packages::symbol_path::{
    ExternalSymbolPath, ExternalSymbolPathError, InvalidExternalSymbolComponent,
};

#[test]
fn from_single_creates_one_component_path() {
    let path = ExternalSymbolPath::from_single("foo");
    assert_eq!(path.components(), &["foo"]);
    assert_eq!(path.leaf(), "foo");
    assert_eq!(path.component_count(), 1);
    assert!(path.is_single());
}

#[test]
fn from_components_preserves_all_components() {
    let path = ExternalSymbolPath::from_components(vec![
        "io".to_owned(),
        "input".to_owned(),
        "new".to_owned(),
    ]);
    assert_eq!(path.components(), &["io", "input", "new"]);
    assert_eq!(path.leaf(), "new");
    assert_eq!(path.component_count(), 3);
    assert!(!path.is_single());
}

#[test]
fn try_from_components_rejects_empty_path() {
    let result = ExternalSymbolPath::try_from_components(Vec::new());
    assert_eq!(result, Err(ExternalSymbolPathError::EmptyPath));
}

#[test]
fn try_from_components_rejects_empty_component() {
    let result = ExternalSymbolPath::try_from_components(vec!["io".to_owned(), "".to_owned()]);
    assert!(
        matches!(
            result,
            Err(ExternalSymbolPathError::InvalidComponent {
                index: 1,
                ref component,
                reason: InvalidExternalSymbolComponent::Empty,
            }) if component.is_empty()
        ),
        "expected empty-component error, got {result:?}"
    );
}

#[test]
fn try_from_components_rejects_separators() {
    let slash = ExternalSymbolPath::try_from_components(vec!["io/input".to_owned()]);
    assert!(
        matches!(
            slash,
            Err(ExternalSymbolPathError::InvalidComponent {
                reason: InvalidExternalSymbolComponent::ContainsPathSeparator,
                ..
            })
        ),
        "expected path-separator error, got {slash:?}"
    );

    let dot = ExternalSymbolPath::try_from_components(vec!["io.input".to_owned()]);
    assert!(
        matches!(
            dot,
            Err(ExternalSymbolPathError::InvalidComponent {
                reason: InvalidExternalSymbolComponent::ContainsNamespaceSeparator,
                ..
            })
        ),
        "expected namespace-separator error, got {dot:?}"
    );
}

#[test]
fn child_appends_component() {
    let parent = ExternalSymbolPath::from_single("input");
    let child = parent.child("new");
    assert_eq!(child.components(), &["input", "new"]);
    assert_eq!(parent.components(), &["input"]);
}

#[test]
fn push_appends_in_place() {
    let mut path = ExternalSymbolPath::from_single("input");
    path.push("new");
    assert_eq!(path.components(), &["input", "new"]);
}

#[test]
fn display_text_joins_with_dots() {
    let path = ExternalSymbolPath::from_components(vec![
        "io".to_owned(),
        "input".to_owned(),
        "new".to_owned(),
    ]);
    assert_eq!(path.display_text(), "io.input.new");
    assert_eq!(path.to_string(), "io.input.new");
}
