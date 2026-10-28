//! Folded host-call transport fixtures.
//!
//! WHAT: proves that the result slots of a trusted folded host call survive the AST, HIR, JS,
//! constant-storage and export/import boundaries. Slots come from the real `materialise_success_slots`
//! and the carrier is committed by the same helper `fold_root` uses.
//! WHY: the single-value path is covered elsewhere. Multi-slot and optional slots only reach these
//! later boundaries through the carrier, so each boundary needs its own check. No second evaluator
//! and no public Core API are used.

use super::*;
use crate::backends::js::test_symbol_helpers::expected_dev_local_name;
use crate::backends::js::{JsLoweringConfig, lower_hir_to_js};
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::ast::ast_nodes::{MultiBindTargetKind, NodeKind};
use crate::compiler_frontend::ast::const_eval::external_calls::materialise_success_slots;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::{
    FoldedValueMaterialiser, materialize_public_folded_value,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::expression_build::commit_constant_result_block;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ProducedValues, ValueLexicalScope,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeIdentity, CanonicalTypeProjectionContext, NominalOriginResolver,
    intern_canonical_builtin,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::{
    ConstEvalLeafType, ConstEvalSlotType, ExternalFunctionId, ExternalPackageRegistry,
};
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, FoldedValueProjectionContext,
    convert_const_value_to_folded_value,
};
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX,
};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::paths::module_resources::ResourceId;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::semantic_identity::OriginTypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, function_node, node, symbol,
};
use crate::compiler_frontend::tests::type_id_fixture_support::multi_bind_target;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, lower_ast,
};
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn child(
    parent: PathId,
    name: &str,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> PathId {
    path_fork
        .try_intern_child(parent, string_table.intern(name))
        .expect("test path fits")
}

/// Interns `main.moth` and its implicit start function, returning `(entry, start)`.
fn entry_and_start(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> (PathId, PathId) {
    let entry = symbol("main.moth", path_fork, string_table);
    let start = child(entry, IMPLICIT_START_FUNC_NAME, path_fork, string_table);
    (entry, start)
}

fn no_returns() -> FunctionSignature {
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    }
}

fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("test I32 value fits the I32 domain")
}

fn i32_payload(value: i64) -> ConstValuePayload {
    ConstValuePayload::FixedScalar(i32_value(value))
}

fn slot(leaf: ConstEvalLeafType, optional: bool) -> ConstEvalSlotType {
    ConstEvalSlotType { leaf, optional }
}

/// Runs the real slot materialiser over a folded host call and stages the resulting roots.
fn stage_call_slots(
    builder: &AstExpressionBuilder,
    constants: &mut ConstValueStore,
    call: ExprId,
    slots: &[ConstEvalSlotType],
    payloads: Vec<Option<ConstValuePayload>>,
) -> Vec<ConstValueId> {
    let view = builder.view(call).expect("test host call should be stored");
    let provenance = SyntheticInterfaceProvenance::empty();
    constants
        .stage_values(
            |values| -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
                let mut ids = Vec::new();
                materialise_success_slots(
                    slots,
                    view.success_types(),
                    payloads.into_iter(),
                    view,
                    &provenance,
                    values,
                    &mut ids,
                )?;
                Ok(Some(ids))
            },
        )
        .expect("materialised slots should stage")
        .expect("staged slots should be selected")
}

/// Replaces `call` with the carrier `fold_root` builds for a multi-result folded host call.
fn folded_host_carrier(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    call: ExprId,
    slots: &[ConstEvalSlotType],
    payloads: Vec<Option<ConstValuePayload>>,
) -> ExprId {
    let results = stage_call_slots(builder, constants, call, slots, payloads);
    let (success, value_mode) = {
        let view = builder.view(call).expect("test host call should be stored");
        (view.success_shape(), view.value_mode())
    };
    let carrier = commit_constant_result_block(builder, call, &results, PathId::ROOT, None)
        .expect("folded slots should commit their result block")
        .id();
    let carrier_view = builder
        .view(carrier)
        .expect("committed carrier should be stored");
    assert_eq!(
        carrier_view.success_shape(),
        success,
        "carrier should keep the call's success shape"
    );
    assert_eq!(
        carrier_view.value_mode(),
        value_mode,
        "carrier should keep the call's value mode"
    );
    carrier
}

/// A folded host call must reach HIR as constants, never as a host call statement.
fn assert_no_host_call(module: &HirModule) {
    let has_call = module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| matches!(statement.kind, HirStatementKind::Call { .. }));
    assert!(
        !has_call,
        "folded host call should not lower to a HIR call statement"
    );
}

/// Every ordinary write in the module as `(target, value)`, in block and statement order.
fn ordered_writes(module: &HirModule) -> Vec<(HirWriteTarget, HirValueId)> {
    module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement.kind {
            HirStatementKind::Write { target, value } => Some((target, value)),
            _ => None,
        })
        .collect()
}

/// The single value that defines `local`. A local defined by a jump argument takes the value of
/// its source local, so the search follows that chain back to the expression that produced it.
fn defining_value(module: &HirModule, local: LocalId) -> HirValueId {
    let writes: Vec<HirValueId> = ordered_writes(module)
        .into_iter()
        .filter(|(target, _)| match target {
            HirWriteTarget::DefineLocal(defined) => *defined == local,
            HirWriteTarget::AssignPlace(place) => place.root == local,
        })
        .map(|(_, value)| value)
        .collect();
    let moved_from: Vec<LocalId> = module
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            HirTerminator::Jump { args, .. } => Some(args),
            _ => None,
        })
        .flatten()
        .filter(|argument| argument.destination == local)
        .map(|argument| argument.source)
        .collect();
    match (writes.as_slice(), moved_from.as_slice()) {
        ([value], []) => *value,
        ([], [source]) => defining_value(module, *source),
        _ => panic!(
            "local {local:?} should have one definition, found {} writes, {} jump args",
            writes.len(),
            moved_from.len()
        ),
    }
}

/// Follows local loads back to the expression that actually produced the value.
fn resolved_value(module: &HirModule, value: HirValueId) -> HirValueId {
    match &module.expressions.expression(value).kind {
        HirExpressionKind::Load(place) => {
            resolved_value(module, defining_value(module, place.root))
        }
        _ => value,
    }
}

fn resolved_kind(module: &HirModule, value: HirValueId) -> &HirExpressionKind {
    &module
        .expressions
        .expression(resolved_value(module, value))
        .kind
}

/// Whether `value` resolves to the I32 constant `expected`.
fn is_i32_constant(module: &HirModule, value: HirValueId, expected: i64) -> bool {
    matches!(
        resolved_kind(module, value),
        HirExpressionKind::FixedScalar(actual) if *actual == i32_value(expected)
    )
}

/// The HIR local whose source name is `name`.
fn local_named(
    module: &HirModule,
    name: &str,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> LocalId {
    module
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, path_fork, string_table)
                == Some(name)
        })
        .map(|local| local.id)
        .unwrap_or_else(|| panic!("HIR should declare a local named `{name}`"))
}

/// Nominal origins are never reached by these fixtures: the payloads are builtin types only.
struct NoNominalOrigins;

impl NominalOriginResolver for NoNominalOrigins {
    fn resolve_nominal_origin(
        &self,
        _nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, InfrastructureFailure> {
        Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "transport fixture has no nominal origins",
        ))
    }
}

/// Consumer side of the import. Builtin canonical types intern into the consumer's own environment.
struct ConsumerFixtureMaterialiser {
    type_environment: TypeEnvironment,
    path_fork: PathInternerFork,
}

impl FoldedValueMaterialiser for ConsumerFixtureMaterialiser {
    fn intern_resource_origin(
        &mut self,
        _origin: &StableResourceOriginId,
        _span: Option<SourceSpan>,
    ) -> Result<ResourceId, InfrastructureFailure> {
        Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "transport fixture has no resource origins",
        ))
    }

    fn intern_canonical_type(
        &mut self,
        identity: &CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<TypeId, ConstValueStoreError> {
        match identity {
            CanonicalTypeIdentity::Builtin(builtin) => {
                intern_canonical_builtin(*builtin, &mut self.type_environment).ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "builtin should intern in the consumer",
                    )
                    .into()
                })
            }
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "transport fixture imports builtin types only",
            )
            .into()),
        }
    }

    fn path_fork(&mut self) -> &mut PathInternerFork {
        &mut self.path_fork
    }

    fn type_environment(&self) -> &TypeEnvironment {
        &self.type_environment
    }
}

#[test]
fn multi_bind_of_folded_slots_lowers_in_order_to_hir_and_js() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (entry, start) = entry_and_start(&mut path_fork, &mut string_table);
    let a_path = child(start, "a", &mut path_fork, &mut string_table);
    let b_path = child(start, "b", &mut path_fork, &mut string_table);
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let mut builder =
        AstExpressionBuilder::new().expect("test expression builder should construct");
    let mut constants = ConstValueStore::default();
    let call = builder
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::IoPrint,
            vec![],
            vec![i32_type, i32_type],
            false,
            None,
        )
        .expect("test host call should construct")
        .id();
    let carrier = folded_host_carrier(
        &mut builder,
        &mut constants,
        call,
        &[
            slot(ConstEvalLeafType::I32, false),
            slot(ConstEvalLeafType::I32, false),
        ],
        vec![Some(i32_payload(7)), Some(i32_payload(11))],
    );
    let targets = vec![
        multi_bind_target(
            a_path,
            i32_type,
            ValueMode::ImmutableOwned,
            MultiBindTargetKind::Declaration,
            None,
        ),
        multi_bind_target(
            b_path,
            i32_type,
            ValueMode::ImmutableOwned,
            MultiBindTargetKind::Declaration,
            None,
        ),
    ];
    let body = vec![node(
        NodeKind::MultiBind {
            targets,
            value: carrier,
        },
        None,
    )];
    let start_function = function_node(start, no_returns(), body, None);
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("folded multi-bind should lower to HIR");

    assert_no_host_call(&module);
    let a_local = local_named(&module, "a", &path_fork, &string_table);
    let b_local = local_named(&module, "b", &path_fork, &string_table);
    assert!(
        is_i32_constant(&module, defining_value(&module, a_local), 7),
        "a should bind the first folded slot, 7"
    );
    assert!(
        is_i32_constant(&module, defining_value(&module, b_local), 11),
        "b should bind the second folded slot, 11"
    );

    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
        .expect("folded multi-bind HIR should validate");

    let js = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("folded multi-bind HIR should lower to JS");
    let a_js = expected_dev_local_name("a", a_local.0);
    let b_js = expected_dev_local_name("b", b_local.0);
    let seven = js
        .source
        .find("__moth_binding(7);")
        .expect("slot 7 should be emitted as a constant");
    let eleven = js
        .source
        .find("__moth_binding(11);")
        .expect("slot 11 should be emitted as a constant");
    let a_assign = js
        .source
        .find(&format!("{a_js} = "))
        .expect("a should be assigned");
    let b_assign = js
        .source
        .find(&format!("{b_js} = "))
        .expect("b should be assigned");
    assert!(
        seven < eleven,
        "slot constants should be emitted in declared order:\n{}",
        js.source
    );
    assert!(
        eleven < a_assign,
        "slots should be staged before the target assignments:\n{}",
        js.source
    );
    assert!(
        a_assign < b_assign,
        "targets should be assigned in declared order:\n{}",
        js.source
    );
    assert!(
        !js.source.contains("__moth_io_print"),
        "folded host call should not emit a host helper:\n{}",
        js.source
    );
}

/// A read of the I32 local at `path`, as a `then` row produces it.
fn i32_read(builder: &mut AstExpressionBuilder, path: PathId) -> ExprId {
    builder
        .reference_with_type_id(
            path,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("test reference should construct")
        .id()
}

/// The parser rejects a `then` row whose producer yields several values, so the folded carrier is
/// bound by an inner multi-bind and the block yields the two bound locals in row order.
#[test]
fn folded_slots_flow_through_a_then_row_into_a_multi_bind() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (entry, start) = entry_and_start(&mut path_fork, &mut string_table);
    let a_path = child(start, "a", &mut path_fork, &mut string_table);
    let b_path = child(start, "b", &mut path_fork, &mut string_table);
    let x_path = child(start, "x", &mut path_fork, &mut string_table);
    let y_path = child(start, "y", &mut path_fork, &mut string_table);
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let mut builder =
        AstExpressionBuilder::new().expect("test expression builder should construct");
    let mut constants = ConstValueStore::default();
    let call = builder
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::IoPrint,
            vec![],
            vec![i32_type, i32_type],
            false,
            None,
        )
        .expect("test host call should construct")
        .id();
    let carrier = folded_host_carrier(
        &mut builder,
        &mut constants,
        call,
        &[
            slot(ConstEvalLeafType::I32, false),
            slot(ConstEvalLeafType::I32, false),
        ],
        vec![Some(i32_payload(7)), Some(i32_payload(11))],
    );
    let bind_a_and_b = node(
        NodeKind::MultiBind {
            targets: vec![
                multi_bind_target(
                    a_path,
                    i32_type,
                    ValueMode::ImmutableOwned,
                    MultiBindTargetKind::Declaration,
                    None,
                ),
                multi_bind_target(
                    b_path,
                    i32_type,
                    ValueMode::ImmutableOwned,
                    MultiBindTargetKind::Declaration,
                    None,
                ),
            ],
            value: carrier,
        },
        None,
    );
    let a_read = i32_read(&mut builder, a_path);
    let b_read = i32_read(&mut builder, b_path);
    let yield_a_and_b = node(
        NodeKind::ThenValue(ProducedValues {
            expressions: vec![a_read, b_read],
            span: None,
        }),
        None,
    );
    let success = builder
        .success_from_slots(vec![i32_type, i32_type], None)
        .expect("two-slot success shape should construct");
    let scope_block = builder
        .value_block(
            ValueBlock::LexicalScope(ValueLexicalScope {
                body: vec![bind_a_and_b, yield_a_and_b],
                scope: start,
                result_type_ids: vec![i32_type, i32_type],
            }),
            None,
            success,
            ValueMode::ImmutableOwned,
        )
        .expect("lexical scope block should construct")
        .id();
    let outer_targets = vec![
        multi_bind_target(
            x_path,
            i32_type,
            ValueMode::ImmutableOwned,
            MultiBindTargetKind::Declaration,
            None,
        ),
        multi_bind_target(
            y_path,
            i32_type,
            ValueMode::ImmutableOwned,
            MultiBindTargetKind::Declaration,
            None,
        ),
    ];
    let body = vec![node(
        NodeKind::MultiBind {
            targets: outer_targets,
            value: scope_block,
        },
        None,
    )];
    let start_function = function_node(start, no_returns(), body, None);
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("folded slots through a then row should lower to HIR");

    assert_no_host_call(&module);
    let a_local = local_named(&module, "a", &path_fork, &string_table);
    let b_local = local_named(&module, "b", &path_fork, &string_table);
    let x_local = local_named(&module, "x", &path_fork, &string_table);
    let y_local = local_named(&module, "y", &path_fork, &string_table);
    assert!(
        is_i32_constant(&module, defining_value(&module, a_local), 7),
        "a should bind the first folded slot, 7"
    );
    assert!(
        is_i32_constant(&module, defining_value(&module, b_local), 11),
        "b should bind the second folded slot, 11"
    );

    for (local, expected) in [(x_local, 7), (y_local, 11)] {
        assert!(
            is_i32_constant(&module, defining_value(&module, local), expected),
            "{local:?} should receive then value {expected}"
        );
    }

    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
        .expect("folded slots through a then row should validate");
}

#[test]
fn return_of_two_result_carrier_yields_constants_in_order() {
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (entry, start) = entry_and_start(&mut path_fork, &mut string_table);
    let pair_path = child(entry, "pair", &mut path_fork, &mut string_table);

    let mut builder =
        AstExpressionBuilder::new().expect("test expression builder should construct");
    let mut constants = ConstValueStore::default();
    let call = builder
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::IoPrint,
            vec![],
            vec![i32_type, builtin_type_ids::BOOL],
            false,
            None,
        )
        .expect("test host call should construct")
        .id();
    let carrier = folded_host_carrier(
        &mut builder,
        &mut constants,
        call,
        &[
            slot(ConstEvalLeafType::I32, false),
            slot(ConstEvalLeafType::Bool, false),
        ],
        vec![Some(i32_payload(7)), Some(ConstValuePayload::Bool(true))],
    );
    let pair_signature = FunctionSignature {
        parameters: vec![],
        returns: fresh_success_returns(vec![i32_type, builtin_type_ids::BOOL]),
    };
    let pair_body = vec![node(NodeKind::Return(vec![carrier]), None)];
    let pair_function = function_node(pair_path, pair_signature, pair_body, None);
    let start_function = function_node(start, no_returns(), vec![], None);
    let ast = build_ast_with_registered_types(
        vec![pair_function, start_function],
        entry,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("returned folded carrier should lower to HIR");

    assert_no_host_call(&module);
    let returned = module
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            HirTerminator::Return(range) => {
                let values = module.expressions.values(*range);
                (values.len() == 2).then(|| values.to_vec())
            }
            _ => None,
        })
        .expect("the pair function should return two values");
    assert!(
        is_i32_constant(&module, returned[0], 7),
        "the first return should be the folded I32 7"
    );
    assert!(
        matches!(
            resolved_kind(&module, returned[1]),
            HirExpressionKind::Bool(true)
        ),
        "the second return should be the folded Bool true"
    );
    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
        .expect("returned folded carrier HIR should validate");
}

#[test]
fn mixed_bool_and_optional_carrier_binds_present_and_absent() {
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let cases = [
        ("present", Some(i32_payload(5)), OPTION_SOME_VARIANT_INDEX),
        ("absent", None, 0),
    ];

    for (case, optional_payload, expected_variant) in cases {
        let present = optional_payload.is_some();
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let (entry, start) = entry_and_start(&mut path_fork, &mut string_table);
        let flag_path = child(start, "flag", &mut path_fork, &mut string_table);
        let maybe_path = child(start, "maybe", &mut path_fork, &mut string_table);

        let mut type_environment = TypeEnvironment::new();
        let option_i32 = type_environment.intern_option(i32_type);
        let mut builder =
            AstExpressionBuilder::new().expect("test expression builder should construct");
        let mut constants = ConstValueStore::default();
        let call = builder
            .host_function_call_with_typed_arguments(
                ExternalFunctionId::IoPrint,
                vec![],
                vec![builtin_type_ids::BOOL, option_i32],
                false,
                None,
            )
            .expect("test host call should construct")
            .id();
        let carrier = folded_host_carrier(
            &mut builder,
            &mut constants,
            call,
            &[
                slot(ConstEvalLeafType::Bool, false),
                slot(ConstEvalLeafType::I32, true),
            ],
            vec![Some(ConstValuePayload::Bool(true)), optional_payload],
        );
        let targets = vec![
            multi_bind_target(
                flag_path,
                builtin_type_ids::BOOL,
                ValueMode::ImmutableOwned,
                MultiBindTargetKind::Declaration,
                None,
            ),
            multi_bind_target(
                maybe_path,
                option_i32,
                ValueMode::ImmutableOwned,
                MultiBindTargetKind::Declaration,
                None,
            ),
        ];
        let body = vec![node(
            NodeKind::MultiBind {
                targets,
                value: carrier,
            },
            None,
        )];
        let start_function = function_node(start, no_returns(), body, None);
        let ast = build_ast_with_registered_types(
            vec![start_function],
            entry,
            builder,
            constants,
            type_environment,
        );
        let (module, type_environment) = lower_ast(
            ast,
            &mut string_table,
            &mut path_fork,
            &ExternalPackageRegistry::new(),
        )
        .expect("mixed carrier should lower to HIR");

        assert_no_host_call(&module);
        let flag = local_named(&module, "flag", &path_fork, &string_table);
        let maybe = local_named(&module, "maybe", &path_fork, &string_table);
        assert!(
            matches!(
                resolved_kind(&module, defining_value(&module, flag)),
                HirExpressionKind::Bool(true)
            ),
            "{case}: flag should bind the folded Bool"
        );
        let maybe_value = resolved_value(&module, defining_value(&module, maybe));
        let expression = module.expressions.expression(maybe_value);
        assert_eq!(
            expression.ty, option_i32,
            "{case}: maybe should keep its Option type"
        );
        let HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index,
            fields,
        } = &expression.kind
        else {
            panic!(
                "{case}: maybe should bind an Option variant, found {:?}",
                expression.kind
            );
        };
        assert_eq!(
            *variant_index, expected_variant,
            "{case}: variant should match presence"
        );
        let fields = module.expressions.variant_fields(*fields);
        if present {
            assert_eq!(
                fields.len(),
                1,
                "{case}: present Option should carry one payload field"
            );
            assert_eq!(
                fields[0].name.map(|name| string_table.resolve(name)),
                Some("value"),
                "{case}: payload field should be named `value`"
            );
            let payload = resolved_value(&module, fields[0].value);
            assert!(
                is_i32_constant(&module, payload, 5),
                "{case}: payload should be the folded I32 5"
            );
        } else {
            assert!(
                fields.is_empty(),
                "{case}: absent Option should carry no payload"
            );
        }
        validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
            .expect("mixed carrier HIR should validate");
    }
}

/// A permitted per-slot leaf type, with the folded payload that each case stages for it.
#[derive(Clone, Copy, Debug)]
enum SlotLeaf {
    I32,
    Bool,
    Text,
}

impl SlotLeaf {
    const ALL: [SlotLeaf; 3] = [SlotLeaf::I32, SlotLeaf::Bool, SlotLeaf::Text];

    fn slot_leaf(self) -> ConstEvalLeafType {
        match self {
            SlotLeaf::I32 => ConstEvalLeafType::I32,
            SlotLeaf::Bool => ConstEvalLeafType::Bool,
            SlotLeaf::Text => ConstEvalLeafType::Text,
        }
    }

    fn type_id(self) -> TypeId {
        match self {
            SlotLeaf::I32 => builtin_type_ids::fixed_scalar(FixedScalar::I32),
            SlotLeaf::Bool => builtin_type_ids::BOOL,
            SlotLeaf::Text => builtin_type_ids::STRING,
        }
    }

    fn payload(self, strings: &mut StringTable) -> ConstValuePayload {
        match self {
            SlotLeaf::I32 => i32_payload(5),
            SlotLeaf::Bool => ConstValuePayload::Bool(true),
            SlotLeaf::Text => {
                ConstValuePayload::String(ConstStringValue::Text(strings.intern("hi")))
            }
        }
    }

    /// The JS literal that this payload lowers to.
    fn js_literal(self) -> &'static str {
        match self {
            SlotLeaf::I32 => "5",
            SlotLeaf::Bool => "true",
            SlotLeaf::Text => "\"hi\"",
        }
    }

    /// Whether `value` resolves to this leaf's folded payload.
    fn is_payload(&self, module: &HirModule, value: HirValueId) -> bool {
        match self {
            SlotLeaf::I32 => is_i32_constant(module, value, 5),
            SlotLeaf::Bool => matches!(resolved_kind(module, value), HirExpressionKind::Bool(true)),
            SlotLeaf::Text => matches!(
                resolved_kind(module, value),
                HirExpressionKind::StringLiteral(text) if text == "hi"
            ),
        }
    }
}

/// The JS body of the emitted `imported` function, from its header to its closing brace.
fn js_imported_body(source: &str) -> &str {
    let start = source
        .find("function moth_imported_fn")
        .expect("the imported function should be emitted");
    let end = source[start..]
        .find("\n}")
        .map(|offset| start + offset)
        .expect("the imported function should close");
    &source[start..end]
}

/// Every permitted leaf as a plain slot, and as an optional slot that is present and absent.
/// Each must keep its type through constant storage, export/import, HIR and JS lowering.
#[test]
fn folded_slot_types_survive_constant_storage_export_and_import() {
    let external_registry = ExternalPackageRegistry::new();
    let cases = SlotLeaf::ALL
        .into_iter()
        .flat_map(|leaf| [(leaf, false, true), (leaf, true, true), (leaf, true, false)]);

    for (leaf, optional, present) in cases {
        let case = format!("{leaf:?}, optional: {optional}, present: {present}");
        let leaf_type = leaf.type_id();

        // Producer: the real slot materialiser stages the constant, then it is published.
        let mut producer_types = TypeEnvironment::new();
        let producer_type = if optional {
            producer_types.intern_option(leaf_type)
        } else {
            leaf_type
        };
        let mut producer_strings = StringTable::new();
        let mut producer_paths = PathInternerFork::empty();
        let mut producer_builder =
            AstExpressionBuilder::new().expect("test expression builder should construct");
        let mut producer_constants = ConstValueStore::default();
        let call = producer_builder
            .host_function_call_with_typed_arguments(
                ExternalFunctionId::IoPrint,
                vec![],
                vec![producer_type],
                false,
                None,
            )
            .expect("test host call should construct")
            .id();
        let payload = present.then(|| leaf.payload(&mut producer_strings));
        let staged = stage_call_slots(
            &producer_builder,
            &mut producer_constants,
            call,
            &[slot(leaf.slot_leaf(), optional)],
            vec![payload],
        );
        let [constant] = staged.as_slice() else {
            panic!("{case}: one result slot should stage one constant");
        };
        let constant = *constant;
        let config_path = symbol("CONFIG", &mut producer_paths, &mut producer_strings);
        producer_constants
            .bind_module_constant(config_path, constant)
            .expect("module constant should bind");
        assert_eq!(
            producer_constants.value_for_path(&config_path),
            Some(constant),
            "{case}: published path should resolve to the staged constant"
        );

        // Export the published constant as a public folded value.
        let nominal_origins = NoNominalOrigins;
        let projection = CanonicalTypeProjectionContext::new(
            &nominal_origins,
            &FoldedValueGenericParameterResolver,
            &external_registry,
        );
        let export_context = FoldedValueProjectionContext {
            type_environment: &producer_types,
            string_table: &producer_strings,
            projection_context: &projection,
            resources: None,
            path_fork: &producer_paths,
        };
        let folded =
            convert_const_value_to_folded_value(&producer_constants, constant, &export_context)
                .expect("constant should export");

        // Import into a consumer environment, then return the constant from a consumer function.
        let mut consumer = ConsumerFixtureMaterialiser {
            type_environment: TypeEnvironment::new(),
            path_fork: PathInternerFork::empty(),
        };
        let consumer_type = if optional {
            consumer.type_environment.intern_option(leaf_type)
        } else {
            leaf_type
        };
        let mut consumer_constants = ConstValueStore::default();
        let mut consumer_strings = StringTable::new();
        let imported = materialize_public_folded_value(
            &mut consumer,
            &folded,
            consumer_type,
            &mut consumer_constants,
            &mut consumer_strings,
            None,
        )
        .unwrap_or_else(|_| panic!("{case}: constant should import"));
        let ConsumerFixtureMaterialiser {
            type_environment,
            mut path_fork,
        } = consumer;
        let (entry, start) = entry_and_start(&mut path_fork, &mut consumer_strings);
        let imported_path = child(entry, "imported", &mut path_fork, &mut consumer_strings);
        let mut builder =
            AstExpressionBuilder::new().expect("test expression builder should construct");
        let returned = builder
            .constant(imported, consumer_type, None, ValueMode::ImmutableOwned)
            .expect("imported constant should construct")
            .id();
        let imported_signature = FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![consumer_type]),
        };
        let imported_body = vec![node(NodeKind::Return(vec![returned]), None)];
        let imported_function =
            function_node(imported_path, imported_signature, imported_body, None);
        let start_function = function_node(start, no_returns(), vec![], None);
        let ast = build_ast_with_registered_types(
            vec![imported_function, start_function],
            entry,
            builder,
            consumer_constants,
            type_environment,
        );
        let (module, type_environment) = lower_ast(
            ast,
            &mut consumer_strings,
            &mut path_fork,
            &external_registry,
        )
        .expect("imported constant should lower to HIR");

        assert_no_host_call(&module);
        let returned_value = module
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                HirTerminator::Return(range) => match module.expressions.values(*range) {
                    [value] => Some(*value),
                    _ => None,
                },
                _ => None,
            })
            .expect("the imported function should return one value");
        let value = resolved_value(&module, returned_value);
        let expression = module.expressions.expression(value);
        assert_eq!(
            expression.ty, consumer_type,
            "{case}: the returned constant should keep its consumer type"
        );
        if optional {
            let HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index,
                fields,
            } = &expression.kind
            else {
                panic!(
                    "{case}: expected an Option variant, found {:?}",
                    expression.kind
                );
            };
            let expected_variant = if present {
                OPTION_SOME_VARIANT_INDEX
            } else {
                0
            };
            assert_eq!(
                *variant_index, expected_variant,
                "{case}: variant index should match presence"
            );
            let fields = module.expressions.variant_fields(*fields);
            if present {
                assert_eq!(
                    fields.len(),
                    1,
                    "{case}: present Option should carry one payload field"
                );
                assert_eq!(
                    fields[0].name.map(|name| consumer_strings.resolve(name)),
                    Some("value"),
                    "{case}: payload field should be named `value`"
                );
                let payload = resolved_value(&module, fields[0].value);
                assert!(
                    leaf.is_payload(&module, payload),
                    "{case}: payload should be the folded constant"
                );
                assert_eq!(
                    module.expressions.expression(payload).ty,
                    leaf_type,
                    "{case}: payload should keep the slot type"
                );
            } else {
                assert!(
                    fields.is_empty(),
                    "{case}: absent Option should carry no payload"
                );
            }
        } else {
            assert!(
                leaf.is_payload(&module, value),
                "{case}: the returned constant should be the folded payload"
            );
        }
        validate_hir_module(&module, &type_environment, &external_registry)
            .expect("imported constant HIR should validate");

        // Backend: the imported function returns the same value, Option-wrapped when optional.
        let js = lower_hir_to_js(
            &module,
            &NumericProofs::default(),
            &consumer_strings,
            JsLoweringConfig::direct_js(false),
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .expect("imported constant HIR should lower to JS");
        let body = js_imported_body(&js.source);
        let expected_return = match (optional, present) {
            (false, _) => format!("return {};", leaf.js_literal()),
            (true, true) => format!(
                "return {{ tag: \"some\", \"value\": {} }};",
                leaf.js_literal()
            ),
            (true, false) => "return { tag: \"none\" };".to_owned(),
        };
        assert!(
            body.contains(&expected_return),
            "{case}: imported function should contain `{expected_return}`:\n{body}"
        );
    }
}

#[test]
fn zero_result_carrier_expression_statement_adds_no_hir_statement() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (entry, start) = entry_and_start(&mut path_fork, &mut string_table);

    let mut builder =
        AstExpressionBuilder::new().expect("test expression builder should construct");
    let mut constants = ConstValueStore::default();
    let call = builder
        .host_function_call_with_typed_arguments(
            ExternalFunctionId::IoPrint,
            vec![],
            vec![],
            false,
            None,
        )
        .expect("test host call should construct")
        .id();
    let carrier = folded_host_carrier(&mut builder, &mut constants, call, &[], vec![]);
    let body = vec![node(NodeKind::ExpressionStatement(carrier), None)];
    let start_function = function_node(start, no_returns(), body, None);
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("zero-result carrier statement should lower to HIR");

    assert_no_host_call(&module);
    // The entry `start()` fragment accumulator is the only statement. The zero-result carrier
    // adds none: no call, no expression statement and no result write.
    let statements: Vec<&HirStatementKind> = module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(|statement| &statement.kind)
        .collect();
    assert!(
        matches!(
            statements.as_slice(),
            [HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(_),
                ..
            }]
        ),
        "zero-result carrier should add no statement beyond the entry accumulator: {statements:?}"
    );
    validate_hir_module(&module, &type_environment, &ExternalPackageRegistry::new())
        .expect("zero-result carrier HIR should validate");
}
