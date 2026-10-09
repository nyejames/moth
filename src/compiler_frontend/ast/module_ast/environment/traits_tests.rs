//! Trait registration and requirement substitution tests for the AST environment builder.
//!
//! WHAT: covers the unified `register_core_cast_traits` path and the
//!      `core_trait_id_for_name` lookup for both `DISPLAYABLE` and the
//!      thirty core cast traits, plus the builtin evidence registration
//!      step.
//! WHY: the AST environment builder must register the core cast traits
//!      in one pass and must register builtin evidence rows that
//!      `builtin_for` can find. Tests here pin that contract without
//!      touching the full builder pipeline.

use super::{signature_with_trait_this_as_parameter, trait_this_parameter_list};
use crate::compiler_frontend::ast::module_ast::environment::traits::AstModuleEnvironmentBuilder;
use crate::compiler_frontend::builtins::casts::evidence::{
    lookup_builtin_evidence, type_id_for_builtin_target,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidTraitConformanceReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::headers::SourceTokenOwner;
use crate::compiler_frontend::headers::parse_file_headers::{
    HeaderKind, HeaderParseOptions, parse_file_headers_with_table,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceDatabase};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_diagnostic;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::environment::{
    CoreTraitKind, DISPLAYABLE_TRAIT_NAME, TraitEnvironment,
};
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

#[test]
fn builtin_cast_evidence_registration_uses_exact_targets() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    register_error_nominal_type(&mut type_environment, &mut path_fork, &mut string_table);

    let mut trait_environment = TraitEnvironment::new();
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
        &mut path_fork,
    )
    .expect("core cast traits should register");

    let numeric_sources: Vec<_> = [
        BuiltinCastTarget::Int,
        BuiltinCastTarget::Uint,
        BuiltinCastTarget::Float,
    ]
    .into_iter()
    .chain(
        FixedScalar::ALL
            .into_iter()
            .filter(|scalar| *scalar != FixedScalar::Byte)
            .map(BuiltinCastTarget::Fixed),
    )
    .collect();
    let numeric_source_ids: Vec<_> = numeric_sources
        .into_iter()
        .map(|source| {
            (
                source,
                builtin_cast_type_id(source, &type_environment, &mut string_table, &mut path_fork),
            )
        })
        .collect();
    let string_source = builtin_cast_type_id(
        BuiltinCastTarget::String,
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let byte_source = builtin_cast_type_id(
        BuiltinCastTarget::Fixed(FixedScalar::Byte),
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let u8_source = builtin_cast_type_id(
        BuiltinCastTarget::Fixed(FixedScalar::U8),
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let bool_source = builtin_cast_type_id(
        BuiltinCastTarget::Bool,
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let char_source = builtin_cast_type_id(
        BuiltinCastTarget::Char,
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let error_source = builtin_cast_type_id(
        BuiltinCastTarget::Error,
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );

    let profiles = [
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ];

    for profile in profiles {
        let mut evidence_environment = TraitEvidenceEnvironment::new();
        AstModuleEnvironmentBuilder::register_builtin_cast_evidence(
            &trait_environment,
            &mut evidence_environment,
            &type_environment,
            &mut string_table,
            &mut path_fork,
            profile,
        )
        .expect("builtin evidence registration should succeed");

        let has_evidence =
            |source, trait_id| evidence_environment.builtin_for(source, trait_id).is_some();

        for scalar in FixedScalar::ALL
            .into_iter()
            .filter(|scalar| *scalar != FixedScalar::Byte)
        {
            let target = BuiltinCastTarget::Fixed(scalar);
            let target_name = scalar.name();
            let castable_trait = registered_core_cast_trait_id(
                &trait_environment,
                &mut string_table,
                &format!("CASTABLE_TO_{target_name}"),
            );
            let try_castable_trait = registered_core_cast_trait_id(
                &trait_environment,
                &mut string_table,
                &format!("TRY_CASTABLE_TO_{target_name}"),
            );

            for (source, source_type_id) in numeric_source_ids
                .iter()
                .copied()
                .chain([(BuiltinCastTarget::String, string_source)])
            {
                let expected_fallibility =
                    lookup_builtin_evidence(source, target, profile).map(|row| row.fallibility);
                assert_eq!(
                    has_evidence(source_type_id, castable_trait),
                    expected_fallibility == Some(BuiltinCastFallibility::Infallible),
                    "{source:?} -> {target:?} must use only its selected infallible evidence"
                );
                assert_eq!(
                    has_evidence(source_type_id, try_castable_trait),
                    expected_fallibility == Some(BuiltinCastFallibility::Fallible),
                    "{source:?} -> {target:?} must use only its selected fallible evidence"
                );
            }
        }

        let castable_to_u8 =
            registered_core_cast_trait_id(&trait_environment, &mut string_table, "CASTABLE_TO_U8");
        let castable_to_string = registered_core_cast_trait_id(
            &trait_environment,
            &mut string_table,
            "CASTABLE_TO_STRING",
        );
        let try_castable_to_bool = registered_core_cast_trait_id(
            &trait_environment,
            &mut string_table,
            "TRY_CASTABLE_TO_BOOL",
        );
        let try_castable_to_char = registered_core_cast_trait_id(
            &trait_environment,
            &mut string_table,
            "TRY_CASTABLE_TO_CHAR",
        );
        let castable_to_error = registered_core_cast_trait_id(
            &trait_environment,
            &mut string_table,
            "CASTABLE_TO_ERROR",
        );
        let try_castable_to_error = registered_core_cast_trait_id(
            &trait_environment,
            &mut string_table,
            "TRY_CASTABLE_TO_ERROR",
        );

        assert!(has_evidence(byte_source, castable_to_u8));
        assert!(!has_evidence(u8_source, castable_to_u8));
        assert!(has_evidence(bool_source, castable_to_string));
        assert!(has_evidence(char_source, castable_to_string));
        assert!(has_evidence(error_source, castable_to_string));
        assert!(has_evidence(string_source, try_castable_to_bool));
        assert!(has_evidence(string_source, try_castable_to_char));
        assert!(has_evidence(string_source, castable_to_error));
        assert!(!has_evidence(string_source, try_castable_to_error));
    }

    for removed_name in [
        "CASTABLE_TO_INT",
        "TRY_CASTABLE_TO_INT",
        "CASTABLE_TO_FLOAT",
        "TRY_CASTABLE_TO_FLOAT",
    ] {
        assert!(
            trait_environment
                .core_trait_id_for_name(string_table.intern(removed_name), &string_table)
                .is_none(),
            "{removed_name} must not remain a registered core cast trait"
        );
    }
}

#[test]
fn register_builtin_cast_evidence_fails_when_core_cast_traits_are_missing() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    register_error_nominal_type(&mut type_environment, &mut path_fork, &mut string_table);
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);

    let mut trait_evidence_environment = TraitEvidenceEnvironment::new();
    let result = AstModuleEnvironmentBuilder::register_builtin_cast_evidence(
        &trait_environment,
        &mut trait_evidence_environment,
        &type_environment,
        &mut string_table,
        &mut path_fork,
        NumericProfile::STANDARD,
    );

    assert!(
        result.is_err(),
        "registration must fail instead of skipping rows whose core cast trait is missing"
    );
}

#[test]
fn register_builtin_cast_evidence_fails_when_error_source_type_is_missing() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut trait_type_environment = TypeEnvironment::new();
    register_error_nominal_type(
        &mut trait_type_environment,
        &mut path_fork,
        &mut string_table,
    );
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut trait_type_environment, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut trait_type_environment,
        &mut string_table,
        &mut path_fork,
    )
    .expect("core cast traits should register");

    // The evidence environment lacks the builtin Error nominal, so the `Error -> String`
    // row's source type cannot resolve.
    let type_environment_without_error = TypeEnvironment::new();
    let mut trait_evidence_environment = TraitEvidenceEnvironment::new();
    let result = AstModuleEnvironmentBuilder::register_builtin_cast_evidence(
        &trait_environment,
        &mut trait_evidence_environment,
        &type_environment_without_error,
        &mut string_table,
        &mut path_fork,
        NumericProfile::STANDARD,
    );

    assert!(
        result.is_err(),
        "registration must fail instead of skipping rows whose source type is missing"
    );
}

#[test]
fn displayable_registers_through_unified_core_path() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);

    let trait_name = string_table.intern(DISPLAYABLE_TRAIT_NAME);
    let resolved = trait_environment.core_trait_id_for_name(trait_name, &string_table);
    assert!(
        resolved.is_some(),
        "DISPLAYABLE must resolve without dependency clauses"
    );
}

#[test]
fn register_core_trait_returns_same_id_for_repeated_calls() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    let i32_type_id = builtin_cast_type_id(
        BuiltinCastTarget::Fixed(FixedScalar::I32),
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let first = trait_environment.register_core_trait(
        &mut type_environment,
        &mut string_table,
        "CASTABLE_TO_I32",
        "to_i32",
        i32_type_id,
        None,
    );
    let second = trait_environment.register_core_trait(
        &mut type_environment,
        &mut string_table,
        "CASTABLE_TO_I32",
        "to_i32",
        i32_type_id,
        None,
    );

    assert_eq!(first, second, "re-registration must return the original id");
}

#[test]
fn fallible_core_trait_appends_error_return_channel() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();

    let i32_type_id = builtin_cast_type_id(
        BuiltinCastTarget::Fixed(FixedScalar::I32),
        &type_environment,
        &mut string_table,
        &mut path_fork,
    );
    let error_type_id =
        register_error_nominal_type(&mut type_environment, &mut path_fork, &mut string_table);
    let trait_id = trait_environment.register_core_trait(
        &mut type_environment,
        &mut string_table,
        "TRY_CASTABLE_TO_I32",
        "try_to_i32",
        i32_type_id,
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
fn register_core_cast_traits_exposes_exact_target_contracts() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    let error_type_id =
        register_error_nominal_type(&mut type_environment, &mut path_fork, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
        &mut path_fork,
    )
    .expect("core cast traits should register");

    let target_families = FixedScalar::ALL
        .into_iter()
        .filter(|scalar| *scalar != FixedScalar::Byte)
        .map(|scalar| {
            (
                BuiltinCastTarget::Fixed(scalar),
                scalar.name().to_owned(),
                scalar.name().to_ascii_lowercase(),
            )
        })
        .chain([
            (
                BuiltinCastTarget::Bool,
                "BOOL".to_owned(),
                "bool".to_owned(),
            ),
            (
                BuiltinCastTarget::String,
                "STRING".to_owned(),
                "string".to_owned(),
            ),
            (
                BuiltinCastTarget::Char,
                "CHAR".to_owned(),
                "char".to_owned(),
            ),
            (
                BuiltinCastTarget::Error,
                "ERROR".to_owned(),
                "error".to_owned(),
            ),
        ]);

    for (target, target_name, method_suffix) in target_families {
        let target_type_id =
            builtin_cast_type_id(target, &type_environment, &mut string_table, &mut path_fork);
        for (fallibility, trait_prefix, method_prefix) in [
            (BuiltinCastFallibility::Infallible, "CASTABLE_TO", "to"),
            (
                BuiltinCastFallibility::Fallible,
                "TRY_CASTABLE_TO",
                "try_to",
            ),
        ] {
            let trait_name = format!("{trait_prefix}_{target_name}");
            let requirement_name = format!("{method_prefix}_{method_suffix}");
            let trait_id =
                registered_core_cast_trait_id(&trait_environment, &mut string_table, &trait_name);
            assert_registered_core_cast_trait(
                &trait_environment,
                &string_table,
                trait_id,
                &trait_name,
                &requirement_name,
                target,
                fallibility,
                target_type_id,
                error_type_id,
            );
        }
    }
}

#[test]
fn register_core_cast_traits_records_incompatibility_pairs_symmetrically() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    register_error_nominal_type(&mut type_environment, &mut path_fork, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
        &mut path_fork,
    )
    .expect("core cast traits should register");

    let target_names = FixedScalar::ALL
        .into_iter()
        .filter(|scalar| *scalar != FixedScalar::Byte)
        .map(|scalar| scalar.name().to_owned())
        .chain(
            ["BOOL", "STRING", "CHAR", "ERROR"]
                .into_iter()
                .map(|name| name.to_owned()),
        );

    for target_name in target_names {
        let left_name = format!("CASTABLE_TO_{target_name}");
        let right_name = format!("TRY_CASTABLE_TO_{target_name}");
        let left_id =
            registered_core_cast_trait_id(&trait_environment, &mut string_table, &left_name);
        let right_id =
            registered_core_cast_trait_id(&trait_environment, &mut string_table, &right_name);

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
    let string_trait_id =
        registered_core_cast_trait_id(&trait_environment, &mut string_table, "CASTABLE_TO_STRING");
    assert!(
        !trait_environment.traits_are_incompatible(displayable_id, string_trait_id),
        "DISPLAYABLE must not be marked incompatible with an unrelated core cast trait"
    );
    let i8_trait_id =
        registered_core_cast_trait_id(&trait_environment, &mut string_table, "CASTABLE_TO_I8");
    let try_i16_trait_id =
        registered_core_cast_trait_id(&trait_environment, &mut string_table, "TRY_CASTABLE_TO_I16");
    assert!(
        !trait_environment.traits_are_incompatible(i8_trait_id, try_i16_trait_id),
        "different target families must not be marked incompatible"
    );
}

#[test]
fn core_trait_kind_classifier_records_target_and_fallibility() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.register_core_displayable(&mut type_environment, &mut string_table);
    register_error_nominal_type(&mut type_environment, &mut path_fork, &mut string_table);
    AstModuleEnvironmentBuilder::register_core_cast_traits(
        &mut trait_environment,
        &mut type_environment,
        &mut string_table,
        &mut path_fork,
    )
    .expect("core cast traits should register");

    let infallible_trait_id =
        registered_core_cast_trait_id(&trait_environment, &mut string_table, "CASTABLE_TO_I32");
    assert!(matches!(
        trait_environment.core_trait_kind(infallible_trait_id),
        Some(CoreTraitKind::Castable {
            target: BuiltinCastTarget::Fixed(FixedScalar::I32),
            fallibility: BuiltinCastFallibility::Infallible
        })
    ));

    let fallible_trait_id =
        registered_core_cast_trait_id(&trait_environment, &mut string_table, "TRY_CASTABLE_TO_F16");
    assert!(matches!(
        trait_environment.core_trait_kind(fallible_trait_id),
        Some(CoreTraitKind::Castable {
            target: BuiltinCastTarget::Fixed(FixedScalar::F16),
            fallibility: BuiltinCastFallibility::Fallible
        })
    ));
}

fn register_error_nominal_type(
    type_environment: &mut TypeEnvironment,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> crate::compiler_frontend::datatypes::ids::TypeId {
    let error_path = crate::compiler_frontend::builtins::error_type::builtin_error_type_path(
        path_fork,
        string_table,
    );
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

fn builtin_cast_type_id(
    target: BuiltinCastTarget,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> crate::compiler_frontend::datatypes::ids::TypeId {
    type_id_for_builtin_target(target, type_environment, string_table, path_fork)
        .unwrap_or_else(|| panic!("builtin type {target:?} must be registered"))
}

fn registered_core_cast_trait_id(
    trait_environment: &TraitEnvironment,
    string_table: &mut StringTable,
    name: &str,
) -> crate::compiler_frontend::traits::ids::TraitId {
    trait_environment
        .core_trait_id_for_name(string_table.intern(name), string_table)
        .unwrap_or_else(|| panic!("{name} must be registered"))
}

fn assert_registered_core_cast_trait(
    trait_environment: &TraitEnvironment,
    string_table: &StringTable,
    trait_id: crate::compiler_frontend::traits::ids::TraitId,
    trait_name: &str,
    requirement_name: &str,
    target: BuiltinCastTarget,
    fallibility: BuiltinCastFallibility,
    target_type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    error_type_id: crate::compiler_frontend::datatypes::ids::TypeId,
) {
    assert!(matches!(
        trait_environment.core_trait_kind(trait_id),
        Some(CoreTraitKind::Castable {
            target: registered_target,
            fallibility: registered_fallibility,
        }) if registered_target == target && registered_fallibility == fallibility
    ));

    let definition = trait_environment
        .get(trait_id)
        .expect("registered core cast trait must have a definition");
    assert_eq!(string_table.resolve(definition.name), trait_name);
    assert_eq!(definition.visibility, TraitVisibility::Core);
    assert_eq!(definition.requirements.len(), 1);
    let requirement = definition
        .requirements
        .first()
        .expect("core cast trait must have one requirement");
    assert_eq!(string_table.resolve(requirement.name), requirement_name);
    assert!(requirement.parameters.is_empty());

    let expected_return_count = match fallibility {
        BuiltinCastFallibility::Infallible => 1,
        BuiltinCastFallibility::Fallible => 2,
    };
    assert_eq!(requirement.returns.len(), expected_return_count);
    assert_eq!(requirement.returns[0].type_id, target_type_id);
    assert!(matches!(
        requirement.returns[0].channel,
        crate::compiler_frontend::ast::statements::functions::ReturnChannel::Success
    ));

    if fallibility == BuiltinCastFallibility::Fallible {
        let error_return = &requirement.returns[1];
        assert_eq!(error_return.type_id, error_type_id);
        assert!(matches!(
            error_return.channel,
            crate::compiler_frontend::ast::statements::functions::ReturnChannel::Error
        ));
    }
}

#[test]
fn trait_this_substitution_preserves_authored_signature_spans() {
    let source = "CLONE_VALUE must:\n    clone_value |This, other This| -> This\n;\n";
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = std::path::PathBuf::from("requirement.moth");
    let sources =
        SourceDatabase::build([&path], &path, None, &mut strings).expect("registered source");
    let source_id = sources
        .get_by_canonical_path(&path)
        .expect("source identity")
        .id;
    let scope = path_fork
        .try_intern_filesystem_path(&path, &mut strings)
        .expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("signature tokens");
    let owner = SourceTokenOwner::new(lexed.tokens);
    let prepared = parse_file_headers_with_table(
        owner,
        scope,
        lexed.path_syntax,
        &path,
        &HeaderParseOptions::default(),
        &mut strings,
        &mut path_fork,
        0,
        0,
        &mut spans,
    )
    .expect("trait declaration should prepare");
    let declaration = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Trait { declaration } => Some(declaration),
            _ => None,
        })
        .expect("authored trait declaration");
    let signature = &declaration.requirements[0].signature;
    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(signature.returns.len(), 1);
    let concrete_name = strings.intern("Concrete");
    let original_parameter_type_spans: Vec<_> = signature
        .parameters
        .iter()
        .map(|parameter| match &parameter.type_annotation {
            ParsedTypeRef::This { span, .. } => *span,
            other => panic!("expected authored This parameter, got {other:?}"),
        })
        .collect();
    let original_return_type_span = match &signature.returns[0].value.type_annotation {
        ParsedTypeRef::This { span, .. } => *span,
        other => panic!("expected authored This return, got {other:?}"),
    };
    let this_name = strings.intern("This");
    let synthetic_parameters = trait_this_parameter_list(this_name, Some(declaration.name_span));
    let synthetic_parameter = synthetic_parameters
        .parameters
        .first()
        .expect("trait This parameter");
    assert_eq!(synthetic_parameter.name, this_name);
    assert_eq!(
        synthetic_parameter.span,
        Some(declaration.name_span),
        "synthetic This must preserve the supplied exact span"
    );
    let synthetic_range = synthetic_parameter
        .span
        .expect("synthetic This must carry its supplied span")
        .resolve_with(spans.resolver_for(source_id));
    assert_eq!(
        &source[synthetic_range.start() as usize..synthetic_range.end() as usize],
        "CLONE_VALUE"
    );
    let substituted = signature_with_trait_this_as_parameter(signature, concrete_name);
    assert_eq!(substituted.parameters.len(), signature.parameters.len());
    assert_eq!(substituted.returns.len(), signature.returns.len());
    for (parameter, original) in substituted.parameters.iter().zip(&signature.parameters) {
        assert_eq!(parameter.span, original.span);
    }
    assert_eq!(
        substituted.returns[0].value.span,
        signature.returns[0].value.span
    );
    let receiver = substituted.parameters[0]
        .span
        .expect("substituted receiver should retain its span")
        .resolve_with(spans.resolver_for(source_id));
    let result = substituted.returns[0]
        .value
        .span
        .expect("substituted return should retain its span")
        .resolve_with(spans.resolver_for(source_id));
    assert_eq!(
        &source[receiver.start() as usize..receiver.end() as usize],
        "This"
    );
    assert_eq!(
        &source[result.start() as usize..result.end() as usize],
        "This"
    );
    for (parameter, expected_span) in substituted
        .parameters
        .iter()
        .zip(&original_parameter_type_spans)
    {
        let (span, range) = match &parameter.type_annotation {
            ParsedTypeRef::Named { name, span, .. } if *name == concrete_name => {
                let span = *span;
                let range = span
                    .expect("substituted parameter type should retain its span")
                    .resolve_with(spans.resolver_for(source_id));
                (span, range)
            }
            other => panic!("expected substituted Named parameter, got {other:?}"),
        };
        assert_eq!(span, *expected_span);
        assert_eq!(
            &source[range.start() as usize..range.end() as usize],
            "This"
        );
    }
    let (return_type_span, return_type_range) = match &substituted.returns[0].value.type_annotation
    {
        ParsedTypeRef::Named { name, span, .. } if *name == concrete_name => {
            let span = *span;
            let range = span
                .expect("substituted return type should retain its span")
                .resolve_with(spans.resolver_for(source_id));
            (span, range)
        }
        other => panic!("expected substituted Named return, got {other:?}"),
    };
    assert_eq!(return_type_span, original_return_type_span);
    assert_eq!(
        &source[return_type_range.start() as usize..return_type_range.end() as usize],
        "This"
    );
}

#[test]
fn duplicate_trait_requirement_diagnostic_retains_exact_requirement_spans() {
    let source =
        "-- é🦋\nRENDERABLE must:\n    render |This| -> String\n    render |This| -> String\n;\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    let primary_span = diagnostic
        .primary_span
        .expect("duplicate requirement should retain its exact primary span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);

    let empty_span_builder = ExtendedSpanBuilder::new();
    let resolver = empty_span_builder.resolver_for(SourceId::COMPILATION_ROOT);
    let primary_range = primary_span.resolve_with(resolver);
    assert_eq!(
        &source[primary_range.start() as usize..primary_range.end() as usize],
        "render"
    );
    assert_eq!(diagnostic.labels.len(), 1);

    let first_span = diagnostic.labels[0]
        .span
        .expect("previous requirement label should retain its exact span");
    assert_eq!(first_span.source(), SourceId::COMPILATION_ROOT);
    let first_span_builder = ExtendedSpanBuilder::new();
    let first_range =
        first_span.resolve_with(first_span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[first_range.start() as usize..first_range.end() as usize],
        "render"
    );
}

#[test]
fn builtin_numeric_conformance_targets_report_exact_target_spans() {
    for target in ["Int", "Uint", "Float"]
        .into_iter()
        .chain(FixedScalar::ALL.into_iter().map(|scalar| scalar.name()))
    {
        let source = format!("DISPLAYABLE must:\n;\n{target} must DISPLAYABLE\n");
        let diagnostic = parse_single_file_ast_diagnostic(&source);
        assert!(
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidTraitConformance {
                    reason: InvalidTraitConformanceReason::BuiltinTarget,
                    ..
                }
            ),
            "{target} should be rejected as a builtin conformance target"
        );
        let target_span = diagnostic
            .primary_span
            .expect("builtin-target diagnostics should retain their target span");
        assert_eq!(target_span.source(), SourceId::COMPILATION_ROOT);

        let span_builder = ExtendedSpanBuilder::new();
        let range = target_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
        assert_eq!(
            &source[range.start() as usize..range.end() as usize],
            target
        );
    }
}

#[test]
fn incompatible_fixed_cast_conformances_report_both_source_spans() {
    let source = "Thing = | value I8 |\nThing must CASTABLE_TO_I8\nThing must TRY_CASTABLE_TO_I8\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidTraitConformance {
            reason: InvalidTraitConformanceReason::IncompatibleTraitEvidence { .. },
            ..
        }
    ));

    let conflicting_trait_span = diagnostic
        .primary_span
        .expect("incompatible conformance diagnostics should retain the second trait span");
    assert_eq!(conflicting_trait_span.source(), SourceId::COMPILATION_ROOT);
    let span_builder = ExtendedSpanBuilder::new();
    let conflicting_range =
        conflicting_trait_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[conflicting_range.start() as usize..conflicting_range.end() as usize],
        "TRY_CASTABLE_TO_I8"
    );

    let previous_span = diagnostic
        .labels
        .first()
        .and_then(|label| label.span)
        .expect("incompatible conformance diagnostics should label the first target");
    assert_eq!(previous_span.source(), SourceId::COMPILATION_ROOT);
    let label_builder = ExtendedSpanBuilder::new();
    let previous_range =
        previous_span.resolve_with(label_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[previous_range.start() as usize..previous_range.end() as usize],
        "Thing"
    );
}

#[test]
fn dec_conformance_targets_report_builtin_target_with_exact_spans() {
    for target in ["Dec", "Dec2", "_Dec", "__dEc01"] {
        let source = format!("DISPLAYABLE must:\n;\n{target} must DISPLAYABLE\n");
        let diagnostic = parse_single_file_ast_diagnostic(&source);
        assert!(
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidTraitConformance {
                    reason: InvalidTraitConformanceReason::BuiltinTarget,
                    ..
                }
            ),
            "{target} should be rejected as a builtin conformance target"
        );
        let target_span = diagnostic
            .primary_span
            .expect("builtin-target diagnostics should retain their target span");
        assert_eq!(target_span.source(), SourceId::COMPILATION_ROOT);

        let span_builder = ExtendedSpanBuilder::new();
        let range = target_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
        assert_eq!(
            &source[range.start() as usize..range.end() as usize],
            target
        );
    }
}

#[test]
fn uint_conformance_target_reports_builtin_target_with_exact_span() {
    let source = "DISPLAYABLE must:\n;\nUint must DISPLAYABLE\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidTraitConformance {
                reason: InvalidTraitConformanceReason::BuiltinTarget,
                ..
            }
        ),
        "Uint should be rejected as a builtin conformance target, got {:?}",
        diagnostic.payload
    );
    let target_span = diagnostic
        .primary_span
        .expect("builtin-target diagnostics should retain their target span");
    assert_eq!(target_span.source(), SourceId::COMPILATION_ROOT);

    let span_builder = ExtendedSpanBuilder::new();
    let range = target_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "Uint"
    );
}

#[test]
fn unknown_trait_reference_diagnostic_retains_exact_reference_span() {
    let source = "Thing = | value Int |\nThing must UNKNOWN\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    let trait_span = diagnostic
        .primary_span
        .unwrap_or_else(|| panic!("unknown trait reference diagnostic lacks span: {diagnostic:?}"));
    assert_eq!(trait_span.source(), SourceId::COMPILATION_ROOT);

    let span_builder = ExtendedSpanBuilder::new();
    let range = trait_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "UNKNOWN"
    );
}
