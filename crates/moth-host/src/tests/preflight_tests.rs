//! Tests for the pure-Rust preflight over declared host implementations.

use crate::{
    HostBindingIdentity, HostImplementation, HostImportRequirement, HostRequirements,
    HostSignature, HostType, PreflightError, preflight,
};

fn identity(package_path: &str, symbol: &str) -> HostBindingIdentity {
    HostBindingIdentity::new(package_path, [symbol])
}

fn import(symbol: &str, signature: HostSignature) -> HostImportRequirement {
    HostImportRequirement::new(identity("@platform", symbol), signature)
}

fn implementation(symbol: &str, signature: HostSignature) -> HostImplementation {
    HostImplementation::new(identity("@platform", symbol), signature)
}

fn requirements(imports: impl IntoIterator<Item = HostImportRequirement>) -> HostRequirements {
    HostRequirements::new(imports, std::iter::empty()).expect("test requirements should be valid")
}

fn accepted_signature() -> HostSignature {
    HostSignature::new()
        .parameter(HostType::String)
        .parameter(HostType::U8)
        .returns(HostType::U32)
        .fallible()
}

fn single_error(result: Result<(), crate::PreflightErrors>) -> PreflightError {
    let errors = result.expect_err("the mismatch should be reported");
    assert_eq!(errors.errors().len(), 1, "expected exactly one mismatch");
    errors.errors()[0].clone()
}

#[test]
fn an_import_with_a_matching_implementation_passes() {
    let requirements = requirements([import("has_prefix", accepted_signature())]);
    let implementations = [implementation("has_prefix", accepted_signature())];

    assert!(preflight(&requirements, &implementations).is_ok());
}

#[test]
fn exports_need_no_rust_implementation() {
    let requirements = HostRequirements::new(
        [import("has_prefix", accepted_signature())],
        [crate::HostExportRequirement::new(
            "update",
            "game_update",
            HostSignature::new(),
        )],
    )
    .expect("test requirements should be valid");
    let implementations = [implementation("has_prefix", accepted_signature())];

    assert!(preflight(&requirements, &implementations).is_ok());
}

#[test]
fn a_missing_implementation_is_reported() {
    let requirements = requirements([import("has_prefix", accepted_signature())]);

    let error = single_error(preflight(&requirements, &[]));

    assert_eq!(
        error,
        PreflightError::MissingImplementation {
            import: identity("@platform", "has_prefix"),
        }
    );
}

#[test]
fn a_duplicate_implementation_identity_is_reported() {
    let requirements = requirements([import("has_prefix", accepted_signature())]);
    let implementations = [
        implementation("has_prefix", accepted_signature()),
        implementation("has_prefix", accepted_signature()),
    ];

    let error = single_error(preflight(&requirements, &implementations));

    assert_eq!(
        error,
        PreflightError::DuplicateImplementation {
            import: identity("@platform", "has_prefix"),
            declared: 2,
        }
    );
}

#[test]
fn a_wrong_parameter_count_is_reported() {
    let requirements = requirements([import("has_prefix", accepted_signature())]);
    let declared = HostSignature::new()
        .parameter(HostType::String)
        .returns(HostType::U32)
        .fallible();
    let implementations = [implementation("has_prefix", declared)];

    let error = single_error(preflight(&requirements, &implementations));

    assert_eq!(
        error,
        PreflightError::ParameterCountMismatch {
            import: identity("@platform", "has_prefix"),
            required: 2,
            declared: 1,
        }
    );
}

#[test]
fn a_wrong_parameter_type_is_reported() {
    let requirements = requirements([import(
        "has_prefix",
        HostSignature::new().parameter(HostType::I64),
    )]);
    let implementations = [implementation(
        "has_prefix",
        HostSignature::new().parameter(HostType::I32),
    )];

    let error = single_error(preflight(&requirements, &implementations));

    assert_eq!(
        error,
        PreflightError::ParameterTypeMismatch {
            import: identity("@platform", "has_prefix"),
            index: 0,
            required: HostType::I64,
            declared: HostType::I32,
        }
    );
}

#[test]
fn an_optional_payload_difference_is_a_mismatch() {
    let required = requirements([import(
        "lookup",
        HostSignature::new().parameter(HostType::optional(HostType::U32)),
    )]);
    let implementations = [implementation(
        "lookup",
        HostSignature::new().parameter(HostType::optional(HostType::I32)),
    )];

    let error = single_error(preflight(&required, &implementations));

    assert_eq!(
        error,
        PreflightError::ParameterTypeMismatch {
            import: identity("@platform", "lookup"),
            index: 0,
            required: HostType::optional(HostType::U32),
            declared: HostType::optional(HostType::I32),
        }
    );

    // Nested optionals stay structurally distinct, so a differing inner payload is a mismatch
    // rather than a comparison that ignores the nesting.
    let required = requirements([import(
        "lookup",
        HostSignature::new().parameter(HostType::optional(HostType::optional(HostType::U32))),
    )]);
    let implementations = [implementation(
        "lookup",
        HostSignature::new().parameter(HostType::optional(HostType::optional(HostType::U64))),
    )];

    let error = single_error(preflight(&required, &implementations));

    assert_eq!(
        error,
        PreflightError::ParameterTypeMismatch {
            import: identity("@platform", "lookup"),
            index: 0,
            required: HostType::optional(HostType::optional(HostType::U32)),
            declared: HostType::optional(HostType::optional(HostType::U64)),
        }
    );
}

#[test]
fn byte_and_u8_are_distinct_identities() {
    let requirements = requirements([import(
        "first_byte",
        HostSignature::new().parameter(HostType::Byte),
    )]);
    let implementations = [implementation(
        "first_byte",
        HostSignature::new().parameter(HostType::U8),
    )];

    let error = single_error(preflight(&requirements, &implementations));

    assert_eq!(
        error,
        PreflightError::ParameterTypeMismatch {
            import: identity("@platform", "first_byte"),
            index: 0,
            required: HostType::Byte,
            declared: HostType::U8,
        }
    );
}

#[test]
fn a_wrong_success_value_count_is_reported() {
    let requirements = requirements([import("count", HostSignature::new().returns(HostType::U32))]);
    let implementations = [implementation("count", HostSignature::new())];

    let error = single_error(preflight(&requirements, &implementations));

    assert_eq!(
        error,
        PreflightError::SuccessValueCountMismatch {
            import: identity("@platform", "count"),
            required: 1,
            declared: 0,
        }
    );
}

#[test]
fn a_wrong_success_value_type_is_reported() {
    let requirements = requirements([import("count", HostSignature::new().returns(HostType::U32))]);
    let implementations = [implementation(
        "count",
        HostSignature::new().returns(HostType::I64),
    )];

    let error = single_error(preflight(&requirements, &implementations));

    assert_eq!(
        error,
        PreflightError::SuccessValueTypeMismatch {
            import: identity("@platform", "count"),
            index: 0,
            required: HostType::U32,
            declared: HostType::I64,
        }
    );
}

#[test]
fn a_wrong_error_channel_is_reported() {
    let required = requirements([import(
        "read",
        HostSignature::new().returns(HostType::String).fallible(),
    )]);
    let implementations = [implementation(
        "read",
        HostSignature::new().returns(HostType::String),
    )];

    let error = single_error(preflight(&required, &implementations));

    assert_eq!(
        error,
        PreflightError::ErrorChannelMismatch {
            import: identity("@platform", "read"),
            required: true,
            declared: false,
        }
    );

    let required = requirements([import(
        "read",
        HostSignature::new().returns(HostType::String),
    )]);
    let implementations = [implementation(
        "read",
        HostSignature::new().returns(HostType::String).fallible(),
    )];

    let error = single_error(preflight(&required, &implementations));

    assert_eq!(
        error,
        PreflightError::ErrorChannelMismatch {
            import: identity("@platform", "read"),
            required: false,
            declared: true,
        }
    );
}

#[test]
fn an_unused_implementation_is_not_an_error() {
    let requirements = requirements([import("has_prefix", accepted_signature())]);
    let implementations = [
        implementation("has_prefix", accepted_signature()),
        implementation("unused", accepted_signature()),
        // A duplicate declaration is only ambiguous for an identity the artefact imports.
        implementation("unused", accepted_signature()),
    ];

    assert!(preflight(&requirements, &implementations).is_ok());
}

#[test]
fn structurally_distinct_symbol_paths_are_matched_independently() {
    let dotted = HostBindingIdentity::new("@core/io", ["input.new"]);
    let components = HostBindingIdentity::new("@core/io", ["input", "new"]);
    assert_eq!(dotted.to_string(), components.to_string());

    let required = HostRequirements::new(
        [
            HostImportRequirement::new(dotted.clone(), accepted_signature()),
            HostImportRequirement::new(components.clone(), accepted_signature()),
        ],
        std::iter::empty(),
    )
    .expect("structurally distinct identities are not duplicates");

    let only_components = [HostImplementation::new(
        components.clone(),
        accepted_signature(),
    )];
    let errors = preflight(&required, &only_components)
        .expect_err("the dotted component identity has no implementation");
    assert_eq!(
        errors.errors(),
        [PreflightError::MissingImplementation {
            import: dotted.clone()
        }]
    );

    let both = [
        HostImplementation::new(dotted, accepted_signature()),
        HostImplementation::new(components, accepted_signature()),
    ];
    assert!(preflight(&required, &both).is_ok());
}

#[test]
fn several_differences_in_one_signature_are_all_reported() {
    let required = requirements([import("read", accepted_signature())]);
    let declared = HostSignature::new()
        .parameter(HostType::I64)
        .parameter(HostType::U8)
        .returns(HostType::I64);
    let implementations = [implementation("read", declared)];

    let errors =
        preflight(&required, &implementations).expect_err("every difference should be reported");

    assert_eq!(
        errors.errors(),
        [
            PreflightError::ParameterTypeMismatch {
                import: identity("@platform", "read"),
                index: 0,
                required: HostType::String,
                declared: HostType::I64,
            },
            PreflightError::SuccessValueTypeMismatch {
                import: identity("@platform", "read"),
                index: 0,
                required: HostType::U32,
                declared: HostType::I64,
            },
            PreflightError::ErrorChannelMismatch {
                import: identity("@platform", "read"),
                required: true,
                declared: false,
            },
        ]
    );
}

#[test]
fn every_mismatch_is_reported_in_canonical_order() {
    let requirements = requirements([
        import("open", HostSignature::new().returns(HostType::U32)),
        HostImportRequirement::new(
            identity("@audio", "play"),
            HostSignature::new().parameter(HostType::String),
        ),
        import("has_prefix", HostSignature::new()),
    ]);
    let implementations = [
        implementation("open", HostSignature::new().returns(HostType::I64)),
        HostImplementation::new(
            identity("@audio", "play"),
            HostSignature::new().parameter(HostType::U64),
        ),
    ];

    let errors =
        preflight(&requirements, &implementations).expect_err("every mismatch should be reported");

    assert_eq!(
        errors.errors(),
        [
            PreflightError::ParameterTypeMismatch {
                import: identity("@audio", "play"),
                index: 0,
                required: HostType::String,
                declared: HostType::U64,
            },
            PreflightError::MissingImplementation {
                import: identity("@platform", "has_prefix"),
            },
            PreflightError::SuccessValueTypeMismatch {
                import: identity("@platform", "open"),
                index: 0,
                required: HostType::U32,
                declared: HostType::I64,
            },
        ]
    );

    let message = errors.to_string();
    assert!(message.contains("@audio#play"), "message was: {message}");
    assert!(
        message.contains("@platform#has_prefix"),
        "message was: {message}"
    );
    assert!(message.contains("@platform#open"), "message was: {message}");
}
