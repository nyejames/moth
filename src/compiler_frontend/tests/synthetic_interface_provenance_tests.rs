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
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn member(
    class: SyntheticInterfaceClass,
    interface: &str,
    member_name: &str,
) -> SyntheticInterfaceMemberIdentity {
    SyntheticInterfaceMemberIdentity::new(class, interface, member_name)
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
fn provenance_preserved_through_coercion() {
    // The `coerced` constructor must preserve the inner value's synthetic-interface provenance
    // because coercion keeps the value's semantic meaning.
    let member_identity = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let provenance = SyntheticInterfaceProvenance::single(member_identity);

    let inner = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(provenance);

    let coerced = Expression::coerced(inner, builtin_type_ids::FLOAT);

    assert_eq!(
        coerced.synthetic_interface_provenance.members(),
        &[member(
            SyntheticInterfaceClass::ProjectContext,
            "render",
            "html"
        )]
    );
}

#[test]
fn provenance_preserved_through_cast() {
    let member_identity = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let source = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            member_identity.clone(),
        ));
    let type_environment = TypeEnvironment::new();
    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id: builtin_type_ids::INT,
        target_type_id: builtin_type_ids::FLOAT,
        target: BuiltinCastTarget::Float,
        requires_optional_wrap_after_cast: false,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::IntToFloat,
        },
        handling: CastHandling::Infallible,
        location: SourceLocation::default(),
    };

    let cast_expression = Expression::cast(cast, builtin_type_ids::FLOAT, &type_environment);

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
    let location = SourceLocation::default();

    let mut type_environment = TypeEnvironment::new();
    let collection = Expression::collection_with_type_id(
        vec![
            Expression::int(1, location.clone(), ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance.clone()),
            Expression::int(2, location.clone(), ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance.clone()),
        ],
        CollectionExpressionType {
            element_type_id: builtin_type_ids::INT,
            element_diagnostic_type: DataType::Int,
            fixed_capacity: None,
            collection_type_id: None,
        },
        &mut type_environment,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        collection.synthetic_interface_provenance.members(),
        std::slice::from_ref(&project_member)
    );

    let map = Expression::map_literal_with_type_id(
        vec![MapLiteralEntry {
            key: Expression::int(1, location.clone(), ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance.clone()),
            value: Expression::int(2, location.clone(), ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(builder_provenance.clone()),
        }],
        MapLiteralExpressionType {
            key_type_id: builtin_type_ids::INT,
            value_type_id: builtin_type_ids::INT,
            key_diagnostic_type: DataType::Int,
            value_diagnostic_type: DataType::Int,
            map_type_id: None,
        },
        &mut type_environment,
        location.clone(),
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        map.synthetic_interface_provenance.members(),
        &[project_member.clone(), builder_member.clone()]
    );

    let mut string_table = StringTable::new();
    let field_a = Declaration {
        id: InternedPath::from_single_str("first", &mut string_table),
        value: Expression::int(3, location.clone(), ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(project_provenance.clone()),
    };
    let field_b = Declaration {
        id: InternedPath::from_single_str("second", &mut string_table),
        value: Expression::int(4, location.clone(), ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(builder_provenance.clone()),
    };
    let struct_expression = Expression::struct_instance(
        InternedPath::from_single_str("Record", &mut string_table),
        vec![field_a, field_b],
        location.clone(),
        ValueMode::ImmutableOwned,
        false,
        None,
        builtin_type_ids::INT,
    );
    assert_eq!(
        struct_expression.synthetic_interface_provenance.members(),
        &[project_member.clone(), builder_member.clone()]
    );

    let choice_expression = Expression::choice_construct(ChoiceConstructInput {
        nominal_path: InternedPath::from_single_str("Choice", &mut string_table),
        tag: 0,
        fields: vec![Declaration {
            id: InternedPath::from_single_str("payload", &mut string_table),
            value: Expression::int(5, location.clone(), ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(project_provenance),
        }],
        diagnostic_type: DataType::Int,
        type_id: builtin_type_ids::INT,
        location,
        value_mode: ValueMode::ImmutableOwned,
    });
    assert_eq!(
        choice_expression.synthetic_interface_provenance.members(),
        &[project_member,]
    );
}

#[test]
fn provenance_preserved_through_int_to_float_coercion() {
    // The `Int -> Float` coercion in `coerce_expression_to_declared_type` must preserve the
    // original value's provenance because it derives the new value from the old one.
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_declared_type;

    let member_identity = member(SyntheticInterfaceClass::ProjectContext, "render", "html");
    let provenance = SyntheticInterfaceProvenance::single(member_identity.clone());

    let int_expr = Expression::int(7, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(provenance);

    let type_environment = TypeEnvironment::new();
    let coerced =
        coerce_expression_to_declared_type(int_expr, builtin_type_ids::FLOAT, &type_environment);

    assert_eq!(
        coerced.synthetic_interface_provenance.members(),
        &[member_identity]
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

    let lhs = Expression::int(3, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(lhs_member));
    let rhs = Expression::int(4, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(rhs_member));

    let rpn = ExpressionRpn {
        items: vec![
            ExpressionRpnItem::Operand(lhs),
            ExpressionRpnItem::Operand(rhs),
            ExpressionRpnItem::Operator {
                operator: Operator::Add,
                location: SourceLocation::default(),
            },
        ],
    };

    let mut string_table = StringTable::new();
    let folded = match constant_fold(rpn.items, &mut string_table).expect("folding should succeed")
    {
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
        crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind::Int(1),
        SourceLocation::default(),
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableOwned,
    );

    assert!(expr.synthetic_interface_provenance.is_empty());
}
