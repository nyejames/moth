//! Host function metadata regression tests.
//!
//! WHAT: exercises host return-slot derivation and registry uniqueness rules.
//! WHY: host metadata feeds both AST lowering and borrow-check call summaries, so small
//! regressions here can break multiple frontend stages at once.

use super::*;
use crate::compiler_frontend::external_packages::{
    external_success_returns, ExternalAbiType, ExternalJsLowering, ExternalReturnAlias,
    ExternalSignatureType,
};

#[test]
fn return_slots_preserve_alias_metadata() {
    let host_function = ExternalFunctionDef {
        name: "concat_like".to_owned(),
        parameters: vec![
            ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                access_kind: ExternalAccessKind::Shared,
            },
            ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                access_kind: ExternalAccessKind::Shared,
            },
        ],
        returns: vec![ExternalReturnSlot::alias_args(
            ExternalAbiType::Utf8Str,
            vec![1],
        )],
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    };

    let returns = &host_function.returns;
    assert_eq!(returns.len(), 1);
    assert_eq!(
        returns[0].value_type,
        ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)
    );
    assert!(matches!(
        &returns[0].alias,
        ExternalReturnAlias::AliasArgs(parameter_indices) if parameter_indices == &[1usize]
    ));
}

#[test]
fn register_function_rejects_duplicates() {
    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_function(ExternalFunctionDef {
            name: "test_func".to_owned(),
            parameters: Vec::new(),
            returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
            error_return_type: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .unwrap();

    let result = registry.register_function(ExternalFunctionDef {
        name: "test_func".to_owned(),
        parameters: Vec::new(),
        returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    });

    assert!(result.is_err());
}

#[test]
fn collection_helpers_keep_receiver_parameter_access_modes() {
    let registry = ExternalPackageRegistry::new();

    let growable_push = registry
        .get_function_by_id(ExternalFunctionId::CollectionPushGrowable)
        .unwrap();
    let fixed_push = registry
        .get_function_by_id(ExternalFunctionId::CollectionPushFixed)
        .unwrap();

    for push in [growable_push, fixed_push] {
        assert_eq!(push.parameters.len(), 2);
        assert_eq!(push.parameters[0].access_kind, ExternalAccessKind::Mutable);
        assert_eq!(
            push.parameters[0].language_type,
            ExternalSignatureType::Abi(ExternalAbiType::Inferred)
        );
        assert_eq!(push.parameters[1].access_kind, ExternalAccessKind::Shared);
        assert_eq!(
            push.parameters[1].language_type,
            ExternalSignatureType::Abi(ExternalAbiType::Inferred)
        );
        assert_eq!(
            push.returns,
            external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh)
        );
        assert!(push.error_return_type.is_none());
        assert!(push.lowerings.wasm.is_none());
    }

    assert!(matches!(
        &growable_push.lowerings.js,
        Some(ExternalJsLowering::RuntimeFunction(name))
            if name.as_str() == "__moth_collection_push_growable"
    ));
    assert!(matches!(
        &fixed_push.lowerings.js,
        Some(ExternalJsLowering::RuntimeFunction(name))
            if name.as_str() == "__moth_collection_push_fixed"
    ));
    assert_ne!(growable_push.name, fixed_push.name);

    let set = registry
        .get_function_by_id(ExternalFunctionId::CollectionSet)
        .unwrap();
    assert_eq!(set.parameters[0].access_kind, ExternalAccessKind::Mutable);

    let length = registry
        .get_function_by_id(ExternalFunctionId::CollectionLength)
        .unwrap();
    assert_eq!(length.parameters[0].access_kind, ExternalAccessKind::Shared);
}

#[test]
fn same_symbol_name_across_packages_is_allowed() {
    let registry = ExternalPackageRegistry::new().with_test_packages_for_integration();

    // Both @test/pkg-a and @test/pkg-b expose a function named "open".
    let a_result = registry.resolve_package_function("@test/pkg-a", "open");
    assert!(a_result.is_some(), "@test/pkg-a/open should resolve");

    let b_result = registry.resolve_package_function("@test/pkg-b", "open");
    assert!(b_result.is_some(), "@test/pkg-b/open should resolve");

    // They must map to distinct IDs.
    let (a_id, _) = a_result.unwrap();
    let (b_id, _) = b_result.unwrap();
    assert_ne!(
        a_id, b_id,
        "same symbol in different packages must have distinct IDs"
    );
}

#[test]
fn resolve_package_function_selects_correct_package() {
    let registry = ExternalPackageRegistry::new().with_test_packages_for_integration();

    let (a_id, a_def) = registry
        .resolve_package_function("@test/pkg-a", "open")
        .unwrap();
    let (b_id, b_def) = registry
        .resolve_package_function("@test/pkg-b", "open")
        .unwrap();

    assert_eq!(a_def.name, "open");
    assert_eq!(b_def.name, "open");
    assert_ne!(a_id, b_id);
}
