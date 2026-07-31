//! Shared HIR builders and assertions for JavaScript backend tests.
//!
//! WHAT: keeps test fixture construction in one place while each sibling module owns a backend
//! concern. WHY: the JS backend tests build HIR directly, so duplicated constructors make behavior
//! harder to audit than a single stage-local support surface.

pub(super) use crate::backends::js::JsFunctionEmissionPolicy;
pub(super) use crate::backends::js::test_symbol_helpers::{
    expected_dev_field_name, expected_dev_function_name, expected_dev_local_name,
};
pub(super) use crate::backends::js::{JsLoweringConfig, lower_hir_to_js};
pub(super) use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, BorrowStateSnapshot, LocalBorrowSnapshot, LocalMode,
};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::definitions::ChoiceTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, NominalTypeId, TypeConstructor,
};
pub(super) use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalFunctionId, IO_INPUT_EXTERNAL_TYPE_ID,
};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, ChoiceId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::{HirChoice, HirModule};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;

pub(super) use crate::compiler_frontend::symbols::interned_path::InternedPath;
pub(super) use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(super) use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

#[derive(Clone, Copy)]
pub(super) struct TypeIds {
    pub(super) unit: TypeId,
    pub(super) int: TypeId,
    pub(super) boolean: TypeId,
    pub(super) string: TypeId,
    pub(super) float: TypeId,
    pub(super) option_int: TypeId,
    pub(super) fallible_int_string: TypeId,
    pub(super) input_handle: TypeId,
    pub(super) choice_unit: TypeId,
    pub(super) collection_int: TypeId,
    pub(super) map_string_int: TypeId,
}

/// Returns the source text of a single top-level JS helper function.
///
/// WHAT: locates the helper by name and returns everything from its `function name(` declaration
///      through the matching closing brace of its body.
/// WHY: assertions focus on one helper at a time instead of the whole prelude. A simple
///      "next `function `" bound stops at nested function expressions such as the callback inside
///      `__moth_format_float`, so this helper balances braces to find the real end of the outer
///      function body.
pub(super) fn helper_source<'a>(source: &'a str, name: &str) -> &'a str {
    let prefix = format!("function {name}(");
    let start = source
        .find(&prefix)
        .unwrap_or_else(|| panic!("helper {name} must be present in emitted JS"));
    let rest = &source[start..];

    // Locate the opening brace of the helper body.
    let body_start = rest
        .find('{')
        .unwrap_or_else(|| panic!("helper {name} must have a body"));

    // Balance braces, treating `"` and `'` as simple string delimiters so literal braces inside
    // strings do not throw off the count. This is sufficient for the prelude helpers, which do not
    // contain nested template literals or escaped quote edge cases.
    let mut depth = 1usize;
    let mut in_string: Option<char> = None;
    let bytes = rest.as_bytes();

    for index in (body_start + 1)..bytes.len() {
        let byte = bytes[index] as char;

        if let Some(quote) = in_string {
            if byte == quote {
                in_string = None;
            }
            continue;
        }

        if byte == '"' || byte == '\'' {
            in_string = Some(byte);
            continue;
        }

        match byte {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..=index];
                }
            }
            _ => {}
        }
    }

    panic!("helper {name} body is not closed");
}

pub(super) fn loc(start: i32) -> SourceLocation {
    SourceLocation {
        scope: InternedPath::new(),
        start_pos: CharPosition {
            line_number: start,
            char_column: 0,
        },
        end_pos: CharPosition {
            line_number: start,
            char_column: 120, // Arbitrary number
        },
    }
}

pub(super) fn build_type_environment() -> (TypeEnvironment, TypeIds) {
    let mut env = TypeEnvironment::new();
    let builtins = env.builtins();

    let unit = builtins.none;
    let int = builtins.int;
    let boolean = builtins.bool;
    let string = builtins.string;

    let float = builtins.float;

    let option_int = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([int]),
    );

    let choice_def = ChoiceTypeDefinition {
        id: NominalTypeId(0), // overwritten by register_nominal_choice
        path: InternedPath::new(),
        variants: Box::new([]),
        generic_parameters: None,
    };
    let (_, choice_unit) = env.register_nominal_choice(choice_def);

    let collection_int = env.intern_collection(int, None);
    let map_string_int = env.intern_map(string, int);
    let fallible_int_string = env.intern_fallible_carrier(int, string);
    let input_handle = env.intern_external(IO_INPUT_EXTERNAL_TYPE_ID);

    (
        env,
        TypeIds {
            unit,
            int,
            boolean,
            string,
            float,
            option_int,
            fallible_int_string,
            input_handle,
            choice_unit,
            collection_int,
            map_string_int,
        },
    )
}

pub(super) fn expression(
    id: u32,
    kind: HirExpressionKind,
    ty: TypeId,
    region: RegionId,
    value_kind: ValueKind,
) -> HirExpression {
    HirExpression {
        id: crate::compiler_frontend::hir::ids::HirValueId(id),
        kind,
        ty,
        value_kind,
        region,
    }
}

pub(super) fn unit_expression(id: u32, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::TupleConstruct { elements: vec![] },
        ty,
        region,
        ValueKind::Const,
    )
}

pub(super) fn int_expression(id: u32, value: i32, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Int(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(super) fn float_expression(id: u32, value: f64, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Float(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(super) fn bool_expression(id: u32, value: bool, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Bool(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(super) fn string_expression(
    id: u32,
    value: &str,
    ty: TypeId,
    region: RegionId,
) -> HirExpression {
    expression(
        id,
        HirExpressionKind::StringLiteral(value.to_owned()),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(super) fn statement(id: u32, kind: HirStatementKind, line: i32) -> HirStatement {
    HirStatement {
        id: crate::compiler_frontend::hir::ids::HirNodeId(id),
        kind,
        location: loc(line),
    }
}

pub(super) fn local(local_id: u32, ty: TypeId, region: RegionId) -> HirLocal {
    HirLocal {
        id: LocalId(local_id),
        ty,
        mutable: true,
        region,
        source_info: Some(loc(1)),
    }
}

pub(super) fn build_module(
    string_table: &mut StringTable,
    function_name: &str,
    blocks: Vec<HirBlock>,
    function: HirFunction,
    local_names: &[(LocalId, &str)],
) -> HirModule {
    let mut module = HirModule::new();
    let function_id = function.id;
    module.blocks = blocks;
    module.start_function = Some(function_id);
    module.functions = vec![function];
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.choices = vec![HirChoice {
        id: ChoiceId(0),
        frontend_type_id: TypeId(0),
        variants: vec![],
    }];

    let function_path = InternedPath::from_single_str(function_name, string_table);
    module
        .side_table
        .bind_function_name(function_id, function_path.clone());

    for (local_id, local_name) in local_names {
        let local_path = InternedPath::from_single_str(local_name, string_table);
        module.side_table.bind_local_name(*local_id, local_path);
    }

    module
}

/// Builds and lowers a minimal single-function module with an empty body.
///
/// WHY: most prelude and identifier tests only need a module to exist so the prelude is emitted;
/// they do not care about the function body.
pub(super) fn lower_minimal_module(function_name: &str) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(0, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, function_name, vec![block], function, &[]);

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

/// Builds and lowers a minimal module that constructs a map literal.
///
/// WHY: map runtime helpers are emitted only for map-using modules, so runtime-helper tests need
/// a focused fixture that exercises the map prelude without duplicating HIR setup everywhere.
pub(super) fn lower_minimal_map_module(function_name: &str) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let map_expression = expression(
        1,
        HirExpressionKind::MapLiteral(vec![HirMapEntry {
            key: string_expression(2, "Priya", types.string, region),
            value: int_expression(3, 10, types.int, region),
        }]),
        types.map_string_int,
        region,
        ValueKind::RValue,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![statement(1, HirStatementKind::Expr(map_expression), 1)],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, function_name, vec![block], function, &[]);

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

pub(super) fn default_config() -> JsLoweringConfig {
    JsLoweringConfig::direct_js(false)
}

/// Builds and lowers a minimal module that performs one runtime cast expression.
///
/// WHY: demand-driven cast-helper tests need focused HIR fixtures, but the
/// module/function/lowering setup is identical for every policy. Keeping that
/// setup in one helper lets each public fixture name only its source expression,
/// target type, and policy.
fn lower_minimal_module_with_cast(
    function_name: &str,
    policy: BuiltinCastPolicyId,
    source_expression: impl FnOnce(&TypeIds, RegionId) -> HirExpression,
    result_type: impl FnOnce(&TypeIds) -> TypeId,
) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let source = source_expression(&types, region);
    let cast_type = result_type(&types);
    let cast_call = expression(
        2,
        HirExpressionKind::Cast {
            source: Box::new(source),
            policy,
        },
        cast_type,
        region,
        ValueKind::RValue,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![statement(1, HirStatementKind::Expr(cast_call), 1)],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, function_name, vec![block], function, &[]);

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

/// Builds and lowers a minimal module that performs a `String -> Int` cast at runtime.
///
/// WHY: prelude-presence tests want to prove that demand-driven helper emission
/// only emits the numeric parsing helpers when a numeric cast policy is reachable.
pub(super) fn lower_minimal_module_with_string_int_cast(function_name: &str) -> String {
    lower_minimal_module_with_cast(
        function_name,
        BuiltinCastPolicyId::StringToInt,
        |types, region| string_expression(1, "0", types.string, region),
        |types| types.int,
    )
}

/// Builds and lowers a minimal module that only performs an `Int -> Float` cast at runtime.
///
/// WHY: prelude-presence tests want to prove that identity casts do not drag the
/// numeric parsing helpers into the prelude.
pub(super) fn lower_minimal_module_with_int_to_float_cast(function_name: &str) -> String {
    lower_minimal_module_with_cast(
        function_name,
        BuiltinCastPolicyId::IntToFloat,
        |types, region| int_expression(1, 0, types.int, region),
        |types| types.float,
    )
}

/// Builds and lowers a minimal module that performs a `String -> Float` cast at runtime.
///
/// WHY: runtime-helper tests need a module that emits only the float parser and
/// its shared normalizer, without also making the integer parser reachable.
pub(super) fn lower_minimal_module_with_string_float_cast(function_name: &str) -> String {
    lower_minimal_module_with_cast(
        function_name,
        BuiltinCastPolicyId::StringToFloat,
        |types, region| string_expression(1, "0.5", types.string, region),
        |types| types.float,
    )
}

/// Builds and lowers a minimal module that performs a `Float -> String` expression cast.
///
/// WHY: reactive Float template subscriptions use this lazy expression shape so their snapshot
/// function can re-read and format the current source value on every rerender.
pub(super) fn lower_minimal_module_with_float_string_cast(function_name: &str) -> String {
    lower_minimal_module_with_cast(
        function_name,
        BuiltinCastPolicyId::FloatToString,
        |types, region| float_expression(1, 1.5, types.float, region),
        |types| types.string,
    )
}

/// Builds and lowers a minimal module that calls one `@core/io` console function.
///
/// WHY: demand-driven IO helper tests need a focused fixture per console function without
/// duplicating HIR construction in every assertion.
pub(super) fn lower_minimal_module_with_io_call(
    function_name: &str,
    io_function_id: crate::compiler_frontend::external_packages::ExternalFunctionId,
) -> String {
    use crate::compiler_frontend::external_packages::CallTarget;

    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let call_statement = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(io_function_id),
            args: vec![string_expression(2, "hello", types.string, region)],
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![call_statement],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, function_name, vec![block], function, &[]);

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering with IO call should succeed")
    .source
}

/// Builds and lowers a minimal module that calls one `@core/io` input function.
///
/// WHY: demand-driven input helper tests need a focused fixture per input function without
/// duplicating HIR construction in every assertion.
pub(super) fn lower_minimal_module_with_io_input_call(
    function_name: &str,
    io_function_id: ExternalFunctionId,
) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let input_local = local(0, types.input_handle, region);
    let input_load = || {
        expression(
            2,
            HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
            types.input_handle,
            region,
            ValueKind::RValue,
        )
    };

    let args = match io_function_id {
        ExternalFunctionId::IoInputNew => vec![],
        ExternalFunctionId::IoInputUpdate | ExternalFunctionId::IoInputClose => vec![input_load()],
        ExternalFunctionId::IoInputPointerX | ExternalFunctionId::IoInputPointerY => {
            vec![input_load()]
        }
        ExternalFunctionId::IoInputLastKeyPressed
        | ExternalFunctionId::IoInputLastKeyReleased
        | ExternalFunctionId::IoInputLastPointerPressed
        | ExternalFunctionId::IoInputLastPointerReleased => vec![input_load()],
        _ => vec![
            input_load(),
            string_expression(3, "d", types.string, region),
        ],
    };

    let call_statement = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(io_function_id),
            args,
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![input_local],
        statements: vec![call_statement],
        terminator: HirTerminator::Return(unit_expression(4, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, function_name, vec![block], function, &[]);

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering with input IO call should succeed")
    .source
}
