//! Focused unit tests for the synthetic compile-time interface provenance vocabulary.
//!
//! WHAT: exercises the hidden invariants of the stable synthetic-interface member identity and
//!      provenance value that integration output cannot inspect: determinism, duplicate-free
//!      union, empty portability and preservation across a representative value derivation.
//! WHY: these are pure value invariants owned by
//!      `compiler_frontend::synthetic_interface_provenance`, so they own a focused test beside
//!      the module rather than an end-to-end case.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, CollectionExpressionType, MapLiteralExpressionType,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CastFacts, ConstructorField, MapEntry,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn member(
    class: SyntheticInterfaceClass,
    interface: &str,
    member_name: &str,
) -> SyntheticInterfaceMemberIdentity {
    SyntheticInterfaceMemberIdentity::new(class, interface, member_name)
}

/// One materialised `I32` fixture value; panics when the fixture literal is out of range.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range")
}

#[test]
fn empty_provenance_is_portable() {
    let provenance = SyntheticInterfaceProvenance::empty();
    assert!(provenance.is_empty());
    assert!(provenance.members().is_empty());
}

#[test]
fn single_member_provenance_is_not_empty() {
    let provenance = SyntheticInterfaceProvenance::single(member(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    ));
    assert!(!provenance.is_empty());
    assert_eq!(provenance.members().len(), 1);
}

#[test]
fn union_is_sorted_duplicate_free_and_deterministic() {
    let project_a = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let project_b = member(SyntheticInterfaceClass::ProjectContext, "render", "wasm");
    let builder_a = member(SyntheticInterfaceClass::Builder, "assets", "bundle");
    let duplicate_a = member(SyntheticInterfaceClass::ProjectContext, "render", "html");

    let lhs =
        SyntheticInterfaceProvenance::from_members(vec![project_a.clone(), builder_a.clone()]);
    let rhs = SyntheticInterfaceProvenance::from_members(vec![project_b.clone(), duplicate_a]);

    let combined = lhs.union(&rhs);

    // The result is sorted by (class, interface, member). ProjectContext comes before Builder
    // because it is the first declared enum variant.
    assert_eq!(
        combined.members(),
        &[
            member(SyntheticInterfaceClass::ProjectContext, "render", "html"),
            member(SyntheticInterfaceClass::ProjectContext, "render", "wasm"),
            member(SyntheticInterfaceClass::Builder, "assets", "bundle"),
        ]
    );

    // Union is idempotent: re-unioning the same sets produces the same canonical value.
    let recombined = combined.union(&SyntheticInterfaceProvenance::empty());
    assert_eq!(combined, recombined);
}

#[test]
fn merge_preserves_canonical_order_in_place() {
    let project_a = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let project_b = member(SyntheticInterfaceClass::ProjectContext, "render", "wasm");

    let mut provenance = SyntheticInterfaceProvenance::single(project_a);
    provenance.merge(&SyntheticInterfaceProvenance::single(project_b));

    assert_eq!(provenance.members().len(), 2);
    assert_eq!(
        provenance.members(),
        &[
            member(SyntheticInterfaceClass::ProjectContext, "render", "html"),
            member(SyntheticInterfaceClass::ProjectContext, "render", "wasm"),
        ]
    );
}

#[test]
fn identity_is_self_contained_and_deterministic() {
    let identity_a = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let identity_b = member(SyntheticInterfaceClass::ProjectContext, "render", "html");

    assert_eq!(identity_a, identity_b);

    let different_member = member(SyntheticInterfaceClass::ProjectContext, "render", "wasm");
    assert_ne!(identity_a, different_member);

    let different_class = member(SyntheticInterfaceClass::Builder, "render", "html");
    assert_ne!(identity_a, different_class);
}

#[test]
fn provenance_preserved_through_cast() {
    let member_identity = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let source = builder
        .fixed_scalar(
            &mut constants,
            i32_value(42),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let source = builder
        .with_synthetic_interface_provenance(
            source,
            SyntheticInterfaceProvenance::single(member_identity.clone()),
        )
        .expect("fixture provenance fits builder");
    let cast_expression = builder
        .cast(
            source,
            CastFacts {
                source_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                target_type_id: builtin_type_ids::fixed_scalar(FixedScalar::F64),
                requires_optional_wrap_after_cast: false,
                fallibility: BuiltinCastFallibility::Infallible,
                evidence: ResolvedCastEvidence::Builtin {
                    policy: BuiltinCastPolicyId::NumericConversion {
                        source: NumericScalar::Fixed(FixedScalar::I32),
                        target: NumericScalar::Fixed(FixedScalar::F64),
                    },
                },
                handling: CastHandling::Implicit,
                span: None,
            },
            builtin_type_ids::fixed_scalar(FixedScalar::F64),
        )
        .expect("fixture cast fits builder");
    assert_eq!(
        builder
            .view_owned(cast_expression)
            .expect("fixture cast belongs to builder")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        &[member_identity]
    );
}

#[test]
fn aggregate_value_constructors_union_child_provenance() {
    let project_member = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let builder_member = member(SyntheticInterfaceClass::Builder, "assets", "bundle");
    let project_provenance = SyntheticInterfaceProvenance::single(project_member.clone());
    let builder_provenance = SyntheticInterfaceProvenance::single(builder_member.clone());
    let span = None;

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let first = builder
        .fixed_scalar(
            &mut constants,
            i32_value(1),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let first = builder
        .with_synthetic_interface_provenance(first, project_provenance.clone())
        .expect("fixture provenance fits builder");
    let second = builder
        .fixed_scalar(
            &mut constants,
            i32_value(2),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let second = builder
        .with_synthetic_interface_provenance(second, project_provenance.clone())
        .expect("fixture provenance fits builder");
    let collection = builder
        .collection_with_type_id(
            vec![first, second],
            CollectionExpressionType {
                element_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                fixed_capacity: None,
                collection_type_id: None,
            },
            &mut type_environment,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture collection fits builder");
    assert_eq!(
        builder
            .view_owned(collection)
            .expect("fixture collection belongs to builder")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        std::slice::from_ref(&project_member)
    );

    let key = builder
        .fixed_scalar(
            &mut constants,
            i32_value(1),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let key = builder
        .with_synthetic_interface_provenance(key, project_provenance.clone())
        .expect("fixture provenance fits builder");
    let value = builder
        .fixed_scalar(
            &mut constants,
            i32_value(2),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let value = builder
        .with_synthetic_interface_provenance(value, builder_provenance.clone())
        .expect("fixture provenance fits builder");
    let map = builder
        .map_literal_with_type_id(
            vec![MapEntry { key, value }],
            MapLiteralExpressionType {
                key_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                value_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                map_type_id: None,
            },
            &mut type_environment,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture map fits builder");
    assert_eq!(
        builder
            .view_owned(map)
            .expect("fixture map belongs to builder")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        &[project_member.clone(), builder_member.clone()]
    );

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let field_a = builder
        .fixed_scalar(
            &mut constants,
            i32_value(3),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let field_a = builder
        .with_synthetic_interface_provenance(field_a, project_provenance.clone())
        .expect("fixture provenance fits builder");
    let field_b = builder
        .fixed_scalar(
            &mut constants,
            i32_value(4),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let field_b = builder
        .with_synthetic_interface_provenance(field_b, builder_provenance.clone())
        .expect("fixture provenance fits builder");
    let struct_expression = builder
        .struct_instance(
            vec![
                ConstructorField {
                    name: path_fork
                        .try_intern_portable_path("first", &mut string_table)
                        .expect("test path fits"),
                    slot: 0,
                    value: field_a,
                },
                ConstructorField {
                    name: path_fork
                        .try_intern_portable_path("second", &mut string_table)
                        .expect("test path fits"),
                    slot: 1,
                    value: field_b,
                },
            ],
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            span,
            ValueMode::ImmutableOwned,
            false,
        )
        .expect("fixture struct fits builder");
    assert_eq!(
        builder
            .view_owned(struct_expression)
            .expect("fixture struct belongs to builder")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        &[project_member.clone(), builder_member.clone()]
    );

    let payload = builder
        .fixed_scalar(
            &mut constants,
            i32_value(5),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let payload = builder
        .with_synthetic_interface_provenance(payload, project_provenance)
        .expect("fixture provenance fits builder");
    let choice_expression = builder
        .choice_construct(ChoiceConstructInput {
            tag: 0,
            fields: vec![ConstructorField {
                name: path_fork
                    .try_intern_portable_path("payload", &mut string_table)
                    .expect("test path fits"),
                slot: 0,
                value: payload,
            }],
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            span: None,
            value_mode: ValueMode::ImmutableOwned,
        })
        .expect("fixture choice fits builder");
    assert_eq!(
        builder
            .view_owned(choice_expression)
            .expect("fixture choice belongs to builder")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        &[project_member,]
    );
}

#[test]
fn provenance_unioned_through_constant_folding() {
    // Binary constant folding must produce a result whose provenance is the sorted, duplicate-free
    // union of both operands' provenance.
    use crate::compiler_frontend::ast::const_eval::{ConstantFoldOutcome, constant_fold};
    use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_frontend::tests::type_id_fixture_support::{
        runtime_expr, runtime_operand_item, runtime_operator_item,
    };

    let lhs_member = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let rhs_member = member(SyntheticInterfaceClass::Builder, "assets", "bundle");

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let lhs = builder
        .fixed_scalar(
            &mut constants,
            i32_value(3),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let lhs = builder
        .with_synthetic_interface_provenance(lhs, SyntheticInterfaceProvenance::single(lhs_member))
        .expect("fixture provenance fits builder");
    let rhs = builder
        .fixed_scalar(
            &mut constants,
            i32_value(4),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let rhs = builder
        .with_synthetic_interface_provenance(rhs, SyntheticInterfaceProvenance::single(rhs_member))
        .expect("fixture provenance fits builder");
    let root = runtime_expr(
        &mut builder,
        &mut constants,
        vec![
            runtime_operand_item(lhs.id()),
            runtime_operand_item(rhs.id()),
            runtime_operator_item(Operator::Add, None),
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableOwned,
        &type_environment,
    );

    let mut string_table = StringTable::new();
    let folded = match constant_fold(root, &mut builder, &mut constants, &mut string_table, None)
        .expect("folding should succeed")
    {
        ConstantFoldOutcome::Folded(root) => root,
        ConstantFoldOutcome::NotConstant(_) => panic!("expected a fully folded stack"),
        ConstantFoldOutcome::TextUnavailable { .. } => {
            panic!("expected folded text to remain available")
        }
    };

    assert_eq!(
        builder
            .view(folded)
            .expect("folded root belongs to builder")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        [
            member(SyntheticInterfaceClass::ProjectContext, "render", "html"),
            member(SyntheticInterfaceClass::Builder, "assets", "bundle"),
        ]
    );
}

#[test]
fn empty_expression_has_empty_provenance() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expr = builder
        .fixed_scalar(
            &mut constants,
            i32_value(1),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    assert!(
        builder
            .view_owned(expr)
            .expect("fixture scalar belongs to builder")
            .occurrence_facts()
            .is_none_or(|facts| facts.provenance.is_empty())
    );
}
