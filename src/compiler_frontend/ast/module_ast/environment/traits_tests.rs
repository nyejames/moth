//! Core trait registration unit tests for the AST environment builder.
//!
//! WHAT: covers the unified `register_core_cast_traits` path and the
//!      `core_trait_id_for_name` lookup for both `DISPLAYABLE` and the
//!      twelve core cast traits, plus the builtin evidence registration
//!      step.
//! WHY: the AST environment builder must register the core cast traits
//!      in one pass and must register builtin evidence rows that
//!      `builtin_for` can find. Tests here pin that contract without
//!      touching the full builder pipeline.

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::{
    CoreTraitKind, DISPLAYABLE_TRAIT_NAME, TraitEnvironment,
};
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::syntax::TraitReferenceSyntax;

use crate::compiler_frontend::ast::module_ast::environment::traits::AstModuleEnvironmentBuilder;

#[test]
fn displayable_registers_through_unified_core_path() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();

    let first_id =
        trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    let second_id =
        trait_environment.register_core_displayable(&mut type_environment, &mut string_table);

    assert_eq!(
        first_id, second_id,
        "register_core_displayable must be idempotent"
    );

    let resolved = trait_environment
        .core_trait_id_for_name(string_table.intern(DISPLAYABLE_TRAIT_NAME), &string_table);
    assert_eq!(resolved, Some(first_id));
}

#[test]
fn displayable_resolves_via_core_trait_id_for_name() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);

    let trait_ref = TraitReferenceSyntax {
        name: string_table.intern(DISPLAYABLE_TRAIT_NAME),
        location: SourceLocation::default(),
    };
    let resolved = trait_environment.core_trait_id_for_name(trait_ref.name, &string_table);
    assert!(
        resolved.is_some(),
        "DISPLAYABLE must resolve without dependency clauses"
    );
}

#[test]
fn register_core_trait_returns_same_id_for_repeated_calls() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();

    let int_type_id = type_environment.builtins().int;
    let first = trait_environment.register_core_trait(
        &mut type_environment,
        &mut string_table,
        "CASTABLE_TO_INT",
        "to_int",
        int_type_id,
        None,
    );
    let second = trait_environment.register_core_trait(
        &mut type_environment,
        &mut string_table,
        "CASTABLE_TO_INT",
        "to_int",
        int_type_id,
        None,
    );

    assert_eq!(first, second, "re-registration must return the original id");
}

#[test]
fn fallible_core_trait_appends_error_return_channel() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();

    let int_type_id = type_environment.builtins().int;
    let error_type_id = register_error_nominal_type(&mut type_environment, &mut string_table);
    let trait_id = trait_environment.register_core_trait(
        &mut type_environment,
        &mut string_table,
        "TRY_CASTABLE_TO_ERROR",
        "try_to_error",
        int_type_id,
        Some(error_type_id),
    );

    let definition = trait_environment
        .get(trait_id)
        .expect("fallible core trait must register a definition");
    let requirement = definition
        .requirements
        .first()
        .expect("fallible core trait must have a requirement");
    assert_eq!(
        requirement.returns.len(),
        2,
        "fallible core trait requirement must carry the Error! return slot"
    );
    assert!(requirement.returns.iter().any(|slot| matches!(
        slot.channel,
        crate::compiler_frontend::ast::statements::functions::ReturnChannel::Error
    )));
    let error_return = requirement
        .returns
        .iter()
        .find(|slot| {
            matches!(
                slot.channel,
                crate::compiler_frontend::ast::statements::functions::ReturnChannel::Error
            )
        })
        .expect("fallible core trait must have an Error! return slot");
    assert_eq!(error_return.type_id, error_type_id);
}

#[test]
fn register_core_cast_traits_populates_every_canonical_name() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);

    register_error_nominal_type(&mut type_environment, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
    )
    .expect("core cast traits should register");

    for kind in [
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::CastableToInt,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::TryCastableToInt,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::CastableToFloat,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::TryCastableToFloat,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::CastableToBool,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::TryCastableToBool,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::CastableToString,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::TryCastableToString,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::CastableToChar,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::TryCastableToChar,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::CastableToError,
        crate::compiler_frontend::builtins::casts::traits::CoreCastTrait::TryCastableToError,
    ] {
        let name = crate::compiler_frontend::builtins::casts::traits::builtin_cast_trait_name(kind);
        let resolved =
            trait_environment.core_trait_id_for_name(string_table.intern(name), &string_table);
        assert!(
            resolved.is_some(),
            "{name} must resolve through core_trait_id_for_name"
        );
    }
}

#[test]
fn register_builtin_cast_evidence_registers_initial_14_rows() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    register_error_nominal_type(&mut type_environment, &mut string_table);

    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
    )
    .expect("core cast traits should register");

    let mut trait_evidence_environment = TraitEvidenceEnvironment::new();
    AstModuleEnvironmentBuilder::register_builtin_cast_evidence(
        &trait_environment,
        &mut trait_evidence_environment,
        &type_environment,
        &mut string_table,
    )
    .expect("builtin evidence registration should succeed");

    // 14 distinct (source, target) rows from the plan.
    let rows = crate::compiler_frontend::builtins::casts::evidence::builtin_evidence_rows();
    for &row in rows {
        let source_type_id =
            crate::compiler_frontend::builtins::casts::evidence::type_id_for_builtin_target(
                row.source,
                &type_environment,
                &mut string_table,
            )
            .expect("source builtin type must resolve to a TypeId");
        let trait_kind = crate::compiler_frontend::builtins::casts::evidence::builtin_evidence_trait_kind_for_row(row)
            .expect("every builtin evidence row must map to a core cast trait");
        let trait_name =
            crate::compiler_frontend::builtins::casts::traits::builtin_cast_trait_name(trait_kind);
        let trait_id = trait_environment
            .core_trait_id_for_name(string_table.intern(trait_name), &string_table)
            .expect("core cast trait id must be registered");

        let evidence_id = trait_evidence_environment
            .builtin_for(source_type_id, trait_id)
            .unwrap_or_else(|| panic!("builtin evidence must exist for source {source_type_id:?} and trait {trait_name}"));

        let evidence = trait_evidence_environment
            .get(evidence_id)
            .expect("builtin evidence id must resolve");
        assert_eq!(evidence.trait_id, trait_id);
        assert_eq!(evidence.target_type_id, source_type_id);
    }
}

#[test]
fn register_core_cast_traits_records_incompatibility_pairs_symmetrically() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    register_error_nominal_type(&mut type_environment, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
    )
    .expect("core cast traits should register");

    let pairs = [
        ("CASTABLE_TO_INT", "TRY_CASTABLE_TO_INT"),
        ("CASTABLE_TO_FLOAT", "TRY_CASTABLE_TO_FLOAT"),
        ("CASTABLE_TO_BOOL", "TRY_CASTABLE_TO_BOOL"),
        ("CASTABLE_TO_STRING", "TRY_CASTABLE_TO_STRING"),
        ("CASTABLE_TO_CHAR", "TRY_CASTABLE_TO_CHAR"),
        ("CASTABLE_TO_ERROR", "TRY_CASTABLE_TO_ERROR"),
    ];

    for (left_name, right_name) in pairs {
        let left_id = trait_environment
            .core_trait_id_for_name(string_table.intern(left_name), &string_table)
            .unwrap_or_else(|| panic!("{left_name} must be registered"));
        let right_id = trait_environment
            .core_trait_id_for_name(string_table.intern(right_name), &string_table)
            .unwrap_or_else(|| panic!("{right_name} must be registered"));

        assert!(
            trait_environment.traits_are_incompatible(left_id, right_id),
            "{left_name} must be incompatible with {right_name}"
        );
        assert!(
            trait_environment.traits_are_incompatible(right_id, left_id),
            "incompatibility must be symmetric for {right_name} and {left_name}"
        );
    }

    let displayable_id = trait_environment
        .core_trait_id_for_name(string_table.intern(DISPLAYABLE_TRAIT_NAME), &string_table)
        .expect("DISPLAYABLE must be registered");
    let string_trait_id = trait_environment
        .core_trait_id_for_name(string_table.intern("CASTABLE_TO_STRING"), &string_table)
        .expect("CASTABLE_TO_STRING must be registered");
    assert!(
        !trait_environment.traits_are_incompatible(displayable_id, string_trait_id),
        "DISPLAYABLE must not be marked incompatible with an unrelated core cast trait"
    );
}

#[test]
fn core_trait_kind_classifier_records_target_and_fallibility() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    register_error_nominal_type(&mut type_environment, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
    )
    .expect("core cast traits should register");

    let int_trait_id = trait_environment
        .core_trait_id_for_name(string_table.intern("CASTABLE_TO_INT"), &string_table)
        .expect("CASTABLE_TO_INT must register");
    let int_kind = trait_environment
        .core_trait_kind(int_trait_id)
        .expect("core cast trait must record its kind");
    assert!(matches!(
        int_kind,
        CoreTraitKind::Castable {
            target: BuiltinCastTarget::Int,
            fallibility: BuiltinCastFallibility::Infallible
        }
    ));

    let try_error_trait_id = trait_environment
        .core_trait_id_for_name(string_table.intern("TRY_CASTABLE_TO_ERROR"), &string_table)
        .expect("TRY_CASTABLE_TO_ERROR must register");
    let try_error_kind = trait_environment
        .core_trait_kind(try_error_trait_id)
        .expect("core cast trait must record its kind");
    assert!(matches!(
        try_error_kind,
        CoreTraitKind::Castable {
            target: BuiltinCastTarget::Error,
            fallibility: BuiltinCastFallibility::Fallible
        }
    ));
}

fn register_error_nominal_type(
    type_environment: &mut TypeEnvironment,
    string_table: &mut StringTable,
) -> crate::compiler_frontend::datatypes::ids::TypeId {
    let error_path =
        crate::compiler_frontend::builtins::error_type::builtin_error_type_path(string_table);
    let struct_def = crate::compiler_frontend::datatypes::definitions::StructTypeDefinition {
        id: crate::compiler_frontend::datatypes::ids::NominalTypeId(0),
        path: error_path,
        fields: Vec::new().into(),
        generic_parameters: None,
        const_record: false,
    };
    let (_, error_type_id) = type_environment.register_nominal_struct(struct_def);
    error_type_id
}
