//! Proves a separate crate can describe a builder through public SDK names only.

use moth::builder::{
    BuilderDefinition, BuilderDefinitionDraft, Error, ErrorKind, HostPackage, SourcePackage,
};
use moth::host::{HostFunction, HostType};

#[test]
fn external_crate_defines_source_and_host_packages() {
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    let definition = BuilderDefinition::new("game")
        .source_package(
            SourcePackage::new("@ui")
                .from_dir(ui_dir.path())
                .available_in_templates(),
        )
        .host_package(
            HostPackage::new("@platform").function(
                HostFunction::new("has_prefix")
                    .parameter(HostType::String)
                    .parameter(HostType::String)
                    .returns(HostType::Bool),
            ),
        )
        .moth_templates()
        .markdown()
        .finish()
        .expect("public definition should finish");

    assert_eq!(definition.identity(), "game");
}

#[test]
fn external_crate_receives_typed_definition_errors() {
    let draft: BuilderDefinitionDraft =
        BuilderDefinition::new("game").host_package(HostPackage::new("@core/game"));
    let error: Error = draft
        .finish()
        .expect_err("builders cannot claim the Core namespace");

    assert_eq!(error.kind(), ErrorKind::InvalidDefinition);
    assert!(error.to_string().contains("@core/game"), "{error}");
}
