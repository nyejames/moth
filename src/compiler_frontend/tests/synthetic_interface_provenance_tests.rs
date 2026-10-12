//! Focused unit tests for the synthetic compile-time interface provenance vocabulary.
//!
//! WHAT: exercises the hidden invariants of the stable synthetic-interface member identity and
//!      provenance value that integration output cannot inspect: determinism, duplicate-free
//!      union, empty portability and preservation across a representative value derivation.
//! WHY: these are pure value invariants owned by
//!      `compiler_frontend::synthetic_interface_provenance`, so they own a focused test beside
//!      the module rather than an end-to-end case.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_eval::ConstantFoldOutcome;
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, CollectionExpressionType, Expression, MapLiteralEntry,
    MapLiteralExpressionType,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
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
    let source = Expression::fixed_scalar(i32_value(42), None, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            member_identity.clone(),
        ));
    let type_environment = TypeEnvironment::new();
    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        target_type_id: builtin_type_ids::fixed_scalar(FixedScalar::F64),
        target: BuiltinCastTarget::Fixed(FixedScalar::F64),
        requires_optional_wrap_after_cast: false,
        fallibility: BuiltinCastFallibility::Infallible,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar::Fixed(
                    FixedScalar::I32,
                ),
                target: crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar::Fixed(
                    FixedScalar::F64,
                ),
            },
        },
        handling: CastHandling::Implicit,
        span: None,
    };

    let cast_expression = Expression::cast(
        cast,
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        &type_environment,
    );

    assert_eq!(
        cast_expression.synthetic_interface_provenance.members(),
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

    let mut type_environment = TypeEnvironment::new();
    let collection = Expression::collection_with_type_id(
        vec![
            Expression::fixed_scalar(i32_value(1), span, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance.clone()),
            Expression::fixed_scalar(i32_value(2), span, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance.clone()),
        ],
        CollectionExpressionType {
            element_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            element_diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            fixed_capacity: None,
            collection_type_id: None,
        },
        &mut type_environment,
        span,
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        collection.synthetic_interface_provenance.members(),
        std::slice::from_ref(&project_member)
    );

    let map = Expression::map_literal_with_type_id(
        vec![MapLiteralEntry {
            key: Expression::fixed_scalar(i32_value(1), span, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance.clone()),
            value: Expression::fixed_scalar(i32_value(2), span, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(builder_provenance.clone()),
        }],
        MapLiteralExpressionType {
            key_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            value_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            key_diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            map_type_id: None,
        },
        &mut type_environment,
        span,
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        map.synthetic_interface_provenance.members(),
        &[project_member.clone(), builder_member.clone()]
    );

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let field_a = Declaration {
        id: path_fork
            .try_intern_portable_path("first", &mut string_table)
            .expect("test path fits"),
        value: Expression::fixed_scalar(i32_value(3), span, ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(project_provenance.clone()),
        binding_span: None,
        config_qualifier: None,
    };
    let field_b = Declaration {
        id: path_fork
            .try_intern_portable_path("second", &mut string_table)
            .expect("test path fits"),
        value: Expression::fixed_scalar(i32_value(4), span, ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(builder_provenance.clone()),
        binding_span: None,
        config_qualifier: None,
    };
    let struct_expression = Expression::struct_instance(
        path_fork
            .try_intern_portable_path("Record", &mut string_table)
            .expect("test path fits"),
        vec![field_a, field_b],
        span,
        ValueMode::ImmutableOwned,
        false,
        None,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    assert_eq!(
        struct_expression.synthetic_interface_provenance.members(),
        &[project_member.clone(), builder_member.clone()]
    );

    let choice_expression = Expression::choice_construct(ChoiceConstructInput {
        nominal_path: path_fork
            .try_intern_portable_path("Choice", &mut string_table)
            .expect("test path fits"),
        tag: 0,
        fields: vec![Declaration {
            id: path_fork
                .try_intern_portable_path("payload", &mut string_table)
                .expect("test path fits"),
            value: Expression::fixed_scalar(i32_value(5), span, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance),
            binding_span: None,
            config_qualifier: None,
        }],
        diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span: None,
        value_mode: ValueMode::ImmutableOwned,
    });
    assert_eq!(
        choice_expression.synthetic_interface_provenance.members(),
        &[project_member,]
    );
}

#[test]
fn provenance_unioned_through_constant_folding() {
    // Binary constant folding must produce a result whose provenance is the sorted, duplicate-free
    // union of both operands' provenance.
    use crate::compiler_frontend::ast::const_eval::constant_fold;
    use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
    use crate::compiler_frontend::ast::expressions::expression_rpn::{
        ExpressionRpn, ExpressionRpnItem,
    };
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    let lhs_member = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let rhs_member = member(SyntheticInterfaceClass::Builder, "assets", "bundle");

    let lhs = Expression::fixed_scalar(i32_value(3), None, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(lhs_member));
    let rhs = Expression::fixed_scalar(i32_value(4), None, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(rhs_member));

    let rpn = ExpressionRpn {
        items: vec![
            ExpressionRpnItem::Operand(lhs),
            ExpressionRpnItem::Operand(rhs),
            ExpressionRpnItem::Operator {
                operator: Operator::Add,
                span: None,
            },
        ],
    };

    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let folded =
        match constant_fold(rpn.items, &mut string_table, None).expect("folding should succeed") {
            ConstantFoldOutcome::Folded(stack) => stack,
            ConstantFoldOutcome::NotConstant(_) => panic!("expected a fully folded stack"),
            ConstantFoldOutcome::TextUnavailable { .. } => {
                panic!("expected folded text to remain available")
            }
        };

    assert_eq!(folded.len(), 1);
    let ExpressionRpnItem::Operand(result) = &folded[0] else {
        panic!("expected a single folded operand");
    };

    assert_eq!(
        result.synthetic_interface_provenance.members(),
        &[
            member(SyntheticInterfaceClass::ProjectContext, "render", "html"),
            member(SyntheticInterfaceClass::Builder, "assets", "bundle"),
        ]
    );
}

#[test]
fn empty_expression_has_empty_provenance() {
    let expr = Expression::new(
        crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind::FixedScalar(
            i32_value(1),
        ),
        None,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        DataType::FixedScalar(FixedScalar::I32),
        ValueMode::ImmutableOwned,
    );

    assert!(expr.synthetic_interface_provenance.is_empty());
}
