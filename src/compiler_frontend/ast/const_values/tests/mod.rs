//! Resolver tests use one explicit phase expression/content pair.
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue};
use crate::compiler_frontend::ast::const_values::facts::ConstFactValueKind;
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::type_id_fixture_support::runtime_expr;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::cell::RefCell;
use std::rc::Rc;

struct Fixture {
    builder: AstExpressionBuilder,
    values: ConstValueStore,
    strings: StringTable,
    paths: PathInternerFork,
    type_environment: TypeEnvironment,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            builder: AstExpressionBuilder::new().unwrap(),
            values: ConstValueStore::default(),
            strings: StringTable::default(),
            paths: PathInternerFork::empty(),
            type_environment: TypeEnvironment::default(),
        }
    }
}

impl Fixture {
    fn resolver(&mut self) -> ConstValueResolver<'_> {
        ConstValueResolver::new(
            &mut self.strings,
            &mut self.values,
            &mut self.builder,
            Rc::new(RefCell::new(TemplateIrStore::new())),
            &self.type_environment,
        )
    }
    fn path(&mut self, name: &str) -> PathId {
        self.paths
            .try_intern_portable_path(name, &mut self.strings)
            .expect("test path")
    }
    fn integer(&mut self, value: i64) -> ExprId {
        self.builder
            .fixed_scalar(
                &mut self.values,
                FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 fixture"),
                None,
                ValueMode::ImmutableOwned,
            )
            .expect("literal occurrence")
            .id()
    }
    fn reference(&mut self, path: PathId) -> ExprId {
        self.builder
            .reference_with_type_id(
                path,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )
            .expect("reference")
            .id()
    }
    fn operation(&mut self, left: ExprId, right: ExprId, operator: Operator) -> ExprId {
        runtime_expr(
            &mut self.builder,
            &mut self.values,
            vec![
                ExpressionRpnItem::Operand(left),
                ExpressionRpnItem::Operand(right),
                ExpressionRpnItem::Operator {
                    operator,
                    span: None,
                },
            ],
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            None,
            ValueMode::ImmutableOwned,
            &TypeEnvironment::new(),
        )
    }
    fn resolve(
        &mut self,
        expression: ExprId,
        environment: &ConstValueEnvironment,
    ) -> Result<ConstValueId, ConstResolutionError> {
        self.resolver().resolve_expression(expression, environment)
    }
    fn assert_integer(&self, value: ConstValueId, expected: i64) {
        assert!(
            matches!(self.values.payload(value), Some(ConstValuePayload::FixedScalar(value)) if value.as_i64() == Some(expected))
        );
    }
    fn declaration(&mut self, value: ExprId, mutable: bool) -> Declaration {
        Declaration {
            id: self.path("local"),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: if mutable {
                ValueMode::MutableOwned
            } else {
                ValueMode::ImmutableOwned
            },
            value: DeclarationValue::Initializer(value),
            binding_span: None,
            config_qualifier: None,
        }
    }
}

#[test]
fn literal_int_resolves_as_const() {
    let mut fixture = Fixture::default();
    let expression = fixture.integer(42);
    let value = fixture
        .resolve(expression, &ConstValueEnvironment::default())
        .expect("literal");
    fixture.assert_integer(value, 42);
}

#[test]
fn literal_string_resolves_as_const() {
    let mut fixture = Fixture::default();
    let text = fixture.strings.intern("hello");
    let expression = fixture
        .builder
        .string_slice(&mut fixture.values, text, None, ValueMode::ImmutableOwned)
        .expect("string")
        .id();
    let value = fixture
        .resolve(expression, &ConstValueEnvironment::default())
        .expect("literal");
    assert!(
        matches!(fixture.values.payload(value), Some(ConstValuePayload::String(ConstStringValue::Text(stored))) if *stored == text)
    );
}

#[test]
fn folded_arithmetic_resolves_to_literal() {
    let mut fixture = Fixture::default();
    let left = fixture.integer(1);
    let right = fixture.integer(2);
    let expression = fixture.operation(left, right, Operator::Add);
    let value = fixture
        .resolve(expression, &ConstValueEnvironment::default())
        .expect("arithmetic");
    fixture.assert_integer(value, 3);
}

fn substituted_arithmetic(coerced: bool) {
    let mut fixture = Fixture::default();
    let path = fixture.path("x");
    let local = fixture
        .values
        .insert_resolved(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ConstValuePayload::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, if coerced { 40 } else { 3 })
                    .expect("I32"),
            ),
            None,
        )
        .expect("binding");
    let mut environment = ConstValueEnvironment::default();
    environment.insert(path, local);
    let mut left = fixture.reference(path);
    if coerced {
        left = fixture
            .builder
            .coerced(
                fixture.builder.own(left),
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            )
            .expect("coercion")
            .id();
    }
    let right = fixture.integer(if coerced { 2 } else { 5 });
    let expression = fixture.operation(
        left,
        right,
        if coerced {
            Operator::Add
        } else {
            Operator::Multiply
        },
    );
    let value = fixture
        .resolve(expression, &environment)
        .expect("substitution");
    fixture.assert_integer(value, if coerced { 42 } else { 15 });
}

#[test]
fn folded_arithmetic_with_reference_substitution_resolves() {
    substituted_arithmetic(false);
}
#[test]
fn folded_arithmetic_with_coerced_reference_substitution_resolves() {
    substituted_arithmetic(true);
}

#[test]
fn reference_to_known_const_resolves() {
    let mut fixture = Fixture::default();
    let path = fixture.path("ratio");
    let scalar = FixedScalarValue::binary_float(FixedScalar::F64, 2.71).expect("finite F64");
    let local = fixture
        .values
        .insert_resolved(
            builtin_type_ids::fixed_scalar(FixedScalar::F64),
            ConstValuePayload::FixedScalar(scalar),
            None,
        )
        .expect("binding");
    let mut environment = ConstValueEnvironment::default();
    environment.insert(path, local);
    let expression = fixture
        .builder
        .reference_with_type_id(
            path,
            builtin_type_ids::fixed_scalar(FixedScalar::F64),
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        )
        .expect("reference")
        .id();
    let value = fixture
        .resolve(expression, &environment)
        .expect("reference resolves");
    assert!(
        matches!(fixture.values.payload(value), Some(ConstValuePayload::FixedScalar(value)) if *value == scalar)
    );
}

#[test]
fn unresolved_reference_fails() {
    let mut fixture = Fixture::default();
    let path = fixture.path("unknown");
    let expression = fixture.reference(path);
    assert_eq!(
        fixture
            .resolve(expression, &ConstValueEnvironment::default())
            .expect_err("unresolved"),
        ConstResolutionError::UnresolvedReference
    );
}

#[test]
fn function_call_fails_const_resolution() {
    let mut fixture = Fixture::default();
    let path = fixture.path("foo");
    let expression = fixture
        .builder
        .function_call_with_typed_arguments(
            path,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            None,
        )
        .expect("call")
        .id();
    assert_eq!(
        fixture
            .resolve(expression, &ConstValueEnvironment::default())
            .expect_err("call"),
        ConstResolutionError::CallInConstContext
    );
}

#[test]
fn mutable_declaration_fails_private_const_resolution() {
    let mut fixture = Fixture::default();
    let value = fixture.integer(1);
    let declaration = fixture.declaration(value, true);
    assert_eq!(
        fixture
            .resolver()
            .resolve_private_top_level_declaration(&declaration, &ConstValueEnvironment::default())
            .expect_err("mutable"),
        ConstResolutionError::MutableDeclaration
    );
}

#[test]
fn fact_value_kind_from_literal_is_literal() {
    assert_eq!(
        ConstFactValueKind::from_const_value_kind(ConstValueKind::Literal),
        ConstFactValueKind::Literal
    );
}
#[test]
fn fact_value_kind_from_runtime_is_non_const() {
    assert_eq!(
        ConstFactValueKind::from_const_value_kind(ConstValueKind::NonConst),
        ConstFactValueKind::NonConst
    );
}

#[test]
fn coerced_expression_resolves_inner_value() {
    let mut fixture = Fixture::default();
    let inner = fixture.integer(7);
    let expression = fixture
        .builder
        .coerced(
            fixture.builder.own(inner),
            builtin_type_ids::fixed_scalar(FixedScalar::F64),
        )
        .expect("coercion")
        .id();
    let value = fixture
        .resolve(expression, &ConstValueEnvironment::default())
        .expect("coerced literal");
    assert!(matches!(
        fixture.values.payload(value),
        Some(ConstValuePayload::Coerced(_))
    ));
    assert_eq!(
        fixture.values.metadata(value).expect("metadata").type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn runtime_operation_with_unresolved_reference_fails() {
    let mut fixture = Fixture::default();
    let path = fixture.path("missing");
    let left = fixture.reference(path);
    let right = fixture.integer(2);
    let expression = fixture.operation(left, right, Operator::Add);
    assert_eq!(
        fixture
            .resolve(expression, &ConstValueEnvironment::default())
            .expect_err("unresolved"),
        ConstResolutionError::UnresolvedReference
    );
}

#[test]
fn body_local_immutable_literal_resolves() {
    let mut fixture = Fixture::default();
    let value = fixture.integer(99);
    let declaration = fixture.declaration(value, false);
    let result = fixture
        .resolver()
        .resolve_body_local_declaration(&declaration, &ConstValueEnvironment::default())
        .expect("immutable local");
    assert_eq!(result.value_kind, ConstFactValueKind::Literal);
    fixture.assert_integer(result.value, 99);
    assert_eq!(
        fixture
            .values
            .metadata(result.value)
            .expect("metadata")
            .span,
        None
    );
}

#[test]
fn body_local_mutable_declaration_fails() {
    let mut fixture = Fixture::default();
    let value = fixture.integer(99);
    let declaration = fixture.declaration(value, true);
    assert_eq!(
        fixture
            .resolver()
            .resolve_body_local_declaration(&declaration, &ConstValueEnvironment::default())
            .expect_err("mutable"),
        ConstResolutionError::MutableDeclaration
    );
}

#[test]
fn deep_reference_validation_preserves_unresolved_error_on_one_mib_stack() {
    std::thread::Builder::new()
        .name("deep-reference-validation".into())
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut fixture = Fixture::default();
            let path = fixture.path("missing");
            let mut expression = fixture.reference(path);
            for _ in 0..4096 {
                let right = fixture.integer(1);
                expression = fixture.operation(expression, right, Operator::Add);
            }
            assert_eq!(
                fixture
                    .resolver()
                    .resolve_expression(expression, &ConstValueEnvironment::default()),
                Err(ConstResolutionError::UnresolvedReference)
            );
        })
        .expect("spawn reference validation thread")
        .join()
        .expect("reference validation completes");
}
