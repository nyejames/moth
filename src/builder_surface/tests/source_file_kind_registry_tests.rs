//! Unit tests for the source file kind registry.

use crate::builder_surface::source_file_kind_registry::{
    OptionalSourceKind, SourceFileKind, SourceFileKindRegistry,
};

#[test]
fn empty_registry_supports_compiler_owned_kinds_only() {
    let registry = SourceFileKindRegistry::new();

    assert_eq!(
        registry.kind_for_extension("moth"),
        Some(SourceFileKind::Moth)
    );
    assert_eq!(
        registry.kind_for_extension("mtf"),
        Some(SourceFileKind::MothTemplate)
    );
    assert_eq!(registry.kind_for_extension("md"), None);
}

#[test]
fn register_plain_markdown_makes_it_supported_without_reassigning_templates() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register(OptionalSourceKind::PlainMarkdown);

    assert_eq!(
        registry.kind_for_extension("md"),
        Some(SourceFileKind::PlainMarkdown)
    );
    assert_eq!(
        registry.kind_for_extension("mtf"),
        Some(SourceFileKind::MothTemplate),
        "selecting an optional kind cannot reassign a compiler-owned extension"
    );
}

#[test]
fn unsupported_extension_returns_none() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register(OptionalSourceKind::PlainMarkdown);

    assert_eq!(registry.kind_for_extension("css"), None);
    assert_eq!(registry.kind_for_extension("json"), None);
    assert_eq!(registry.kind_for_extension("html"), None);
}

#[test]
fn compiler_owned_kinds_are_never_optional() {
    assert_eq!(SourceFileKind::Moth.optional(), None);
    assert_eq!(SourceFileKind::MothTemplate.optional(), None);
    assert_eq!(
        SourceFileKind::PlainMarkdown.optional(),
        Some(OptionalSourceKind::PlainMarkdown)
    );
}

#[test]
fn plain_markdown_extension_round_trips() {
    assert_eq!(
        SourceFileKind::from_extension("md"),
        Some(SourceFileKind::PlainMarkdown)
    );
    assert_eq!(SourceFileKind::PlainMarkdown.extension(), "md");
    assert_eq!(SourceFileKind::PlainMarkdown.extension_suffix(), ".md");
}

#[test]
fn registry_is_cloneable() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register(OptionalSourceKind::PlainMarkdown);

    let cloned = registry.clone();
    assert_eq!(
        cloned.kind_for_extension("md"),
        Some(SourceFileKind::PlainMarkdown)
    );
}
