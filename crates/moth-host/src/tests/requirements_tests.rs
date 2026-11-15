//! Tests for the semantic import and export requirements the compiler hands to a runtime.

use crate::{
    HostBindingIdentity, HostExportRequirement, HostImportRequirement, HostRequirements,
    HostSignature, RequirementsError,
};

fn identity(package_path: &str, symbol: &str) -> HostBindingIdentity {
    HostBindingIdentity::new(package_path, [symbol])
}

fn import(package_path: &str, symbol: &str) -> HostImportRequirement {
    HostImportRequirement::new(identity(package_path, symbol), HostSignature::new())
}

fn export(facade_name: &str, export_name: &str) -> HostExportRequirement {
    HostExportRequirement::new(facade_name, export_name, HostSignature::new())
}

#[test]
fn binding_identities_are_structural_and_name_themselves() {
    let from_components = HostBindingIdentity::new("@core/io", ["input", "new"]);
    let from_owned_components =
        HostBindingIdentity::new("@core/io", vec!["input".to_owned(), "new".to_owned()]);

    assert_eq!(from_components, from_owned_components);
    assert_eq!(from_components.to_string(), "@core/io#input.new");

    // The display spelling joins components, so a single component containing '.' spells the
    // same name as two components while remaining a structurally distinct identity.
    let dotted_component = HostBindingIdentity::new("@core/io", ["input.new"]);
    assert_eq!(dotted_component.to_string(), from_components.to_string());
    assert_ne!(dotted_component, from_components);

    let requirements = HostRequirements::new(
        [
            HostImportRequirement::new(dotted_component, HostSignature::new()),
            HostImportRequirement::new(from_components, HostSignature::new()),
        ],
        std::iter::empty(),
    )
    .expect("structurally distinct identities are not duplicates");
    assert_eq!(requirements.imports().len(), 2);

    assert_eq!(
        identity("@platform", "has_prefix").to_string(),
        "@platform#has_prefix"
    );
}

#[test]
fn requirements_have_a_canonical_order_independent_of_input_order() {
    let forward = HostRequirements::new(
        [import("@platform", "has_prefix"), import("@audio", "play")],
        [
            export("update", "game_update"),
            export("start", "game_start"),
        ],
    )
    .expect("distinct identities and export names should be accepted");
    let reversed = HostRequirements::new(
        [import("@audio", "play"), import("@platform", "has_prefix")],
        [
            export("start", "game_start"),
            export("update", "game_update"),
        ],
    )
    .expect("distinct identities and export names should be accepted");

    assert_eq!(forward, reversed);

    let identities = forward
        .imports()
        .iter()
        .map(|requirement| requirement.identity().to_string())
        .collect::<Vec<_>>();
    assert_eq!(identities, ["@audio#play", "@platform#has_prefix"]);

    let export_names = forward
        .exports()
        .iter()
        .map(|requirement| requirement.export_name())
        .collect::<Vec<_>>();
    assert_eq!(export_names, ["game_start", "game_update"]);
}

#[test]
fn requirements_reject_a_duplicate_import_identity() {
    let duplicate = import("@platform", "has_prefix");

    let error = HostRequirements::new([duplicate.clone(), duplicate], std::iter::empty())
        .expect_err("a repeated import identity should be rejected");

    assert_eq!(
        error,
        RequirementsError::DuplicateImportIdentity {
            identity: identity("@platform", "has_prefix"),
        }
    );
}

#[test]
fn requirements_reject_a_duplicate_export_name() {
    let error = HostRequirements::new(
        std::iter::empty(),
        [
            export("update", "game_update"),
            export("tick", "game_update"),
        ],
    )
    .expect_err("a repeated Wasm export name should be rejected");

    assert_eq!(
        error,
        RequirementsError::DuplicateExportName {
            export_name: "game_update".to_owned(),
        }
    );
}
