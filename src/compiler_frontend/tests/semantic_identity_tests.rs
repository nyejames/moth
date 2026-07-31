//! Focused unit tests for the compiler-semantic identity vocabulary.
//!
//! WHAT: exercises the structural invariants of the stable exported-declaration origin IDs that
//!      integration output cannot inspect: identity depends only on module origin, defining
//!      name, declaration category and receiver type identity, never on source file, source
//!      location, declaration order or export alias.
//! WHY: these are pure value invariants owned by `compiler_frontend::semantic_identity`, so they
//!      own a focused test beside the module rather than an end-to-end case.

use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::semantic_identity::{
    FunctionOriginKind, ModuleRootRole, OriginConstantId, OriginDeclarationId, OriginFunctionId,
    OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StableOwnedSourceIdentity, StablePackageIdentity,
};

use std::collections::HashSet;
use std::path::Path;

fn module_origin(logical_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("my-project"),
        logical_path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn other_module_origin(logical_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("other-project"),
        logical_path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn struct_type(module: StableModuleOriginIdentity, name: &str) -> OriginTypeId {
    OriginTypeId::new(module, name.to_owned(), OriginTypeCategory::Struct)
}

#[test]
fn type_origin_is_equal_for_equal_construction_independent_of_source_file_or_order() {
    // Identity carries no source file, source location or declaration-order field, so two
    // independently constructed IDs with the same module origin, defining name and category are
    // equal regardless of construction order. Equal values hash equal.
    let module = module_origin("ui/button");

    let first = struct_type(module.clone(), "Button");
    let second = struct_type(module, "Button");

    assert_eq!(
        first, second,
        "equal construction must yield equal identity"
    );
    let mut set = HashSet::new();
    set.insert(first.clone());
    assert!(
        set.contains(&second),
        "equal identity must hash to the same slot as an equal construction"
    );
}

#[test]
fn rename_changes_type_identity() {
    let module = module_origin("ui/button");

    let button = struct_type(module.clone(), "Button");
    let label = struct_type(module, "Label");

    assert_ne!(button, label, "renaming a declaration must change identity");
}

#[test]
fn declaration_category_alters_type_identity() {
    let module = module_origin("shapes");
    let name = "Shape";

    let structure = OriginTypeId::new(module.clone(), name.to_owned(), OriginTypeCategory::Struct);
    let choice = OriginTypeId::new(module.clone(), name.to_owned(), OriginTypeCategory::Choice);
    let alias = OriginTypeId::new(
        module.clone(),
        name.to_owned(),
        OriginTypeCategory::TransparentAlias,
    );

    assert_ne!(structure, choice, "struct and choice must differ");
    assert_ne!(structure, alias, "struct and transparent alias must differ");
    assert_ne!(choice, alias, "choice and transparent alias must differ");
}

#[test]
fn module_change_alters_type_identity() {
    let first_module = module_origin("ui/button");
    let second_module = module_origin("ui/card");

    let button_in_button = struct_type(first_module, "Button");
    let button_in_card = struct_type(second_module, "Button");

    assert_ne!(
        button_in_button, button_in_card,
        "moving a declaration between modules must change identity"
    );
}

#[test]
fn free_function_and_receiver_method_are_distinct() {
    let module = module_origin("runtime");

    let free = OriginFunctionId::new_free(module.clone(), "run".to_owned());
    let worker = struct_type(module.clone(), "Worker");
    let method = OriginFunctionId::new_receiver(module.clone(), "run".to_owned(), worker.clone());

    assert_ne!(
        free, method,
        "a free function and a receiver method of the same name must differ"
    );
    assert!(
        matches!(free.kind(), FunctionOriginKind::Free),
        "free function kind must be Free"
    );
    assert!(
        matches!(method.kind(), FunctionOriginKind::Receiver(_)),
        "receiver method kind must carry its receiver"
    );
    assert!(free.receiver().is_none(), "free function has no receiver");
    assert_eq!(
        method.receiver(),
        Some(&worker),
        "receiver method carries its receiver type identity"
    );
}

#[test]
fn receiver_type_identity_is_part_of_method_identity() {
    let module = module_origin("runtime");

    let worker = struct_type(module.clone(), "Worker");
    let runner = struct_type(module.clone(), "Runner");

    let run_on_worker = OriginFunctionId::new_receiver(module.clone(), "run".to_owned(), worker);
    let run_on_runner = OriginFunctionId::new_receiver(module, "run".to_owned(), runner);

    assert_ne!(
        run_on_worker, run_on_runner,
        "methods of the same name on distinct receiver types must differ"
    );

    let run_on_worker_again = OriginFunctionId::new_receiver(
        module_origin("runtime"),
        "run".to_owned(),
        struct_type(module_origin("runtime"), "Worker"),
    );
    assert_eq!(
        run_on_worker, run_on_worker_again,
        "method identity must be stable across equal construction"
    );
}

#[test]
fn export_alias_cannot_alter_origin_identity() {
    // Export alias is not a constructor input, so renaming a declaration at export time cannot
    // alter its origin. Both IDs are built from the same defining name and module; any export
    // alias lives in a later phase and never reaches this value.
    let module = module_origin("ui/button");
    let declared = struct_type(module.clone(), "Button");

    let re_exported_under_alias = struct_type(module, "Button");

    assert_eq!(
        declared, re_exported_under_alias,
        "an export alias must not alter origin identity"
    );
}

#[test]
fn unified_declaration_id_preserves_typed_category() {
    let module = module_origin("shared");

    let type_id = OriginDeclarationId::Type(struct_type(module.clone(), "Shared"));
    let function = OriginDeclarationId::Function(OriginFunctionId::new_free(
        module.clone(),
        "Shared".to_owned(),
    ));
    let constant =
        OriginDeclarationId::Constant(OriginConstantId::new(module.clone(), "Shared".to_owned()));
    let trait_id = OriginDeclarationId::Trait(OriginTraitId::new(module, "Shared".to_owned()));

    // Same defining name across distinct categories stays distinct and discriminable.
    let mut seen = HashSet::new();
    assert!(seen.insert(type_id.clone()), "type id must be insertable");
    assert!(
        seen.insert(function.clone()),
        "function id must be distinct from type id"
    );
    assert!(
        seen.insert(constant.clone()),
        "constant id must be distinct from type and function ids"
    );
    assert!(
        seen.insert(trait_id.clone()),
        "trait id must be distinct from the other three ids"
    );
    assert_eq!(seen.len(), 4, "all four categories must remain distinct");

    // The unified id reports the owning module origin for every category.
    assert_eq!(
        type_id.module_origin(),
        function.module_origin(),
        "every unified id exposes its module origin"
    );
}

#[test]
fn hashset_deduplication_is_independent_of_insertion_order() {
    let module = module_origin("shapes");
    let shape = struct_type(module.clone(), "Shape");
    let square = OriginTypeId::new(
        module.clone(),
        "Square".to_owned(),
        OriginTypeCategory::Struct,
    );

    // Insert the same values in two different orders. The invariant is equality and deduplication
    // independent of insertion order: a standard HashSet does not provide deterministic hash
    // iteration order, but equal values always collapse to one entry, so the two sets are equal
    // regardless of insertion order.
    let first_order: Vec<OriginTypeId> = vec![
        shape.clone(),
        square.clone(),
        shape.clone(),
        square.clone(),
        shape.clone(),
    ];
    let second_order: Vec<OriginTypeId> = vec![
        square.clone(),
        shape.clone(),
        square.clone(),
        shape.clone(),
        square,
        shape,
    ];

    let set_a: HashSet<OriginTypeId> = first_order.into_iter().collect();
    let set_b: HashSet<OriginTypeId> = second_order.into_iter().collect();

    assert_eq!(
        set_a.len(),
        2,
        "duplicates must collapse to the distinct count"
    );
    assert_eq!(
        set_a, set_b,
        "deduplicated sets built in any insertion order must be equal"
    );
}

#[test]
fn distinct_projects_do_not_share_type_identity() {
    let project_module = module_origin("ui/button");
    let other_project_module = other_module_origin("ui/button");

    let project_button = struct_type(project_module, "Button");
    let other_project_button = struct_type(other_project_module, "Button");

    assert_ne!(
        project_button, other_project_button,
        "the same logical path and name in different projects must differ"
    );
}

// ---- Phase 3a: stable owned-source identity ----

fn owned_source_identity(
    logical_module_path: &str,
    relative_source_path: &str,
) -> StableOwnedSourceIdentity {
    StableOwnedSourceIdentity::from_relative_source_path(
        module_origin(logical_module_path),
        Path::new(relative_source_path),
    )
    .expect("a valid module-relative source path must build a stable owned-source identity")
}

#[test]
fn owned_source_identity_is_equal_for_equal_module_origin_and_relative_path() {
    let first = owned_source_identity("ui/button", "@page.moth");
    let second = owned_source_identity("ui/button", "@page.moth");

    assert_eq!(
        first, second,
        "equal origin and relative path must yield equal identity"
    );
    let mut set = HashSet::new();
    set.insert(first.clone());
    assert!(
        set.contains(&second),
        "equal identity must hash to the same slot"
    );
}

#[test]
fn owned_source_identity_carries_no_absolute_path_or_source_file_field() {
    let identity = owned_source_identity("ui/button", "@page.moth");
    let debug = format!("{identity:?}");

    assert!(
        !debug.contains("ui/button/@page.moth"),
        "the canonical physical source path must not be embedded in the stable identity: {debug}"
    );
    assert_eq!(identity.relative_source_path(), "@page.moth");
    assert_eq!(identity.module_origin().logical_module_path(), "ui/button");
}

#[test]
fn changing_module_origin_changes_owned_source_identity() {
    let button = owned_source_identity("ui/button", "@page.moth");
    let card = owned_source_identity("ui/card", "@page.moth");

    assert_ne!(
        button, card,
        "moving a source file between modules must change its owned-source identity"
    );
}

#[test]
fn changing_relative_source_path_changes_owned_source_identity() {
    let root_file = owned_source_identity("ui/button", "@page.moth");
    let nested_file = owned_source_identity("ui/button", "internal/renderer.moth");

    assert_ne!(
        root_file, nested_file,
        "changing the module-relative source path must change identity"
    );
    assert_eq!(root_file.relative_source_path(), "@page.moth");
    assert_eq!(nested_file.relative_source_path(), "internal/renderer.moth");
}

#[test]
fn owned_source_identity_distinguishes_same_name_in_different_modules() {
    let alpha = owned_source_identity("alpha", "@page.moth");
    let inner = owned_source_identity("alpha/inner", "@page.moth");

    assert_ne!(
        alpha, inner,
        "two root files named @page.moth in different modules must keep distinct identities"
    );
}

#[test]
fn owned_source_identity_rejects_invalid_relative_path_components() {
    let result = StableOwnedSourceIdentity::from_relative_source_path(
        module_origin("ui/button"),
        Path::new("../escape.moth"),
    );
    let error = result.expect_err("an invalid relative source path component must be rejected");
    assert_eq!(
        error.error_type,
        ErrorType::Compiler,
        "an invalid relative source path must use the internal compiler-error lane"
    );
    assert!(
        error.msg.contains("invalid component"),
        "internal error should mention the invalid component: {}",
        error.msg
    );
}

#[test]
fn owned_source_identity_rejects_absolute_relative_path() {
    let result = StableOwnedSourceIdentity::from_relative_source_path(
        module_origin("ui/button"),
        Path::new("/abs/@page.moth"),
    );
    assert!(
        matches!(result, Err(ref e) if e.error_type == ErrorType::Compiler && e.msg.contains("invalid component")),
        "an absolute relative source path must be rejected through the compiler-error lane"
    );
}

#[test]
fn owned_source_identity_rejects_empty_relative_source_path() {
    let result = StableOwnedSourceIdentity::from_relative_source_path(
        module_origin("ui/button"),
        Path::new(""),
    );
    let error = result.expect_err("an empty relative source path must be rejected");
    assert_eq!(
        error.error_type,
        ErrorType::Compiler,
        "an empty relative source path must use the internal compiler-error lane"
    );
    assert!(
        error.msg.contains("empty relative source path"),
        "internal error should mention the empty path: {}",
        error.msg
    );
}
