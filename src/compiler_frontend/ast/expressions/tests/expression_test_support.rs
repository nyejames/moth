//! Test-only expression constructors for hand-built AST fixtures.
//!
//! WHAT: keeps unit-test factories close to the expression owner without
//! mixing them into the production expression data and constructor file.
//! WHY: HIR, borrow-checker, and const-evaluation tests build small AST fragments
//! directly; these helpers preserve that ergonomic surface while production
//! callers use constructors that require canonical `TypeId`s.

use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleCarrierVariant, FallibleExpressionHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::{DataType, PathTypeKind};
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePaths;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

impl Expression {
    pub fn runtime(
        rpn: ExpressionRpn,
        data_type: DataType,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        // Detects whether any nested node performs regular division so the
        // resulting expression can carry the correct provenance flag.
        let type_id = test_builtin_type_id_for_data_type(&data_type);
        let contains_regular_division = rpn.contains_regular_division();

        Self::new(
            ExpressionKind::Runtime(rpn),
            location,
            type_id,
            data_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division)
    }

    pub fn path(compile_time_paths: CompileTimePaths, location: SourceLocation) -> Self {
        // Derives the path type kind from the first resolved path so tests can
        // construct the same AST shape produced by parser-side path handling.
        let path_type_kind = compile_time_paths
            .paths
            .first()
            .map(|path| PathTypeKind::from(path.kind.clone()))
            .unwrap_or(PathTypeKind::File);

        Self::new(
            ExpressionKind::Path(Box::new(compile_time_paths)),
            location,
            builtin_type_ids::STRING,
            DataType::Path(path_type_kind),
            ValueMode::ImmutableOwned,
        )
    }

    pub fn reference(
        id: InternedPath,
        data_type: DataType,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        let type_id = test_builtin_type_id_for_data_type(&data_type);
        Self::reference_with_type_id(
            id,
            data_type,
            type_id,
            location,
            value_mode,
            ConstRecordState::RuntimeValue,
        )
    }

    pub fn function_call(
        name: InternedPath,
        args: Vec<Expression>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    ) -> Self {
        Self::function_call_with_arguments(
            name,
            shared_positional_call_arguments(args),
            result_type_ids,
            location,
        )
    }

    pub fn function_call_with_arguments(
        name: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    ) -> Self {
        call_expression(
            ExpressionKind::FunctionCall {
                name,
                args,
                result_type_ids: result_type_ids.clone(),
            },
            result_type_ids,
            location,
        )
    }

    pub fn handled_fallible_function_call(
        name: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        handling: FallibleExpressionHandling,
        location: SourceLocation,
    ) -> Self {
        call_expression(
            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids: result_type_ids.clone(),
                handling,
            },
            result_type_ids,
            location,
        )
    }

    pub fn host_function_call(
        id: ExternalFunctionId,
        args: Vec<Expression>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    ) -> Self {
        Self::host_function_call_with_arguments(
            id,
            shared_positional_call_arguments(args),
            result_type_ids,
            location,
        )
    }

    pub fn host_function_call_with_arguments(
        id: ExternalFunctionId,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    ) -> Self {
        call_expression(
            ExpressionKind::HostFunctionCall {
                id,
                args,
                result_type_ids: result_type_ids.clone(),
            },
            result_type_ids,
            location,
        )
    }

    pub fn result_construct(
        variant: FallibleCarrierVariant,
        value: Expression,
        type_id: TypeId,
        location: SourceLocation,
        value_mode: ValueMode,
    ) -> Self {
        Self::result_construct_with_type_id(
            variant,
            value,
            DataType::Inferred,
            type_id,
            location,
            value_mode,
        )
    }
}

/// Best-effort builtin `TypeId` for hand-built AST tests that do not have a `TypeEnvironment`.
///
/// Falls back to `NONE` for nominal, generic, or complex types; tests that need canonical
/// identities for those should use the typed production constructor path.
fn test_builtin_type_id_for_data_type(data_type: &DataType) -> TypeId {
    match data_type {
        DataType::Bool | DataType::True | DataType::False => builtin_type_ids::BOOL,
        DataType::Int => builtin_type_ids::INT,
        DataType::Float => builtin_type_ids::FLOAT,
        DataType::Decimal => builtin_type_ids::DECIMAL,
        DataType::StringSlice | DataType::Template | DataType::Path(_) => builtin_type_ids::STRING,
        DataType::Char => builtin_type_ids::CHAR,
        DataType::Range => builtin_type_ids::RANGE,
        DataType::None => builtin_type_ids::NONE,
        _ => builtin_type_ids::NONE,
    }
}

fn call_expression(
    kind: ExpressionKind,
    result_type_ids: Vec<TypeId>,
    location: SourceLocation,
) -> Expression {
    let expression_type_id = match result_type_ids.as_slice() {
        [] => builtin_type_ids::NONE,
        [single] => *single,
        // Hand-built tests that need canonical tuple IDs should use the typed
        // production constructor and pass a real `TypeEnvironment`.
        _ => builtin_type_ids::NONE,
    };

    Expression::new(
        kind,
        location,
        expression_type_id,
        // Test-only fallback: exact diagnostic spelling requires a TypeEnvironment.
        // Call-site diagnostics should render from canonical TypeId.
        DataType::Inferred,
        // Planned: derive ownership from alias-aware return signatures once
        // signature alias metadata is threaded through expression construction.
        // If the return signature is a reference (the name of a parameter passed in),
        // then this is a reference to that parameter.
        ValueMode::MutableOwned,
    )
}

/// Wraps plain `Expression` values in shared positional `CallArgument` nodes.
///
/// WHAT: test helper that avoids repeating the same `CallAccessMode::Shared` and
///       location-clone boilerplate at every hand-built call site.
fn shared_positional_call_arguments(values: Vec<Expression>) -> Vec<CallArgument> {
    values
        .into_iter()
        .map(|value| {
            let location = value.location.clone();
            CallArgument::positional(value, CallAccessMode::Shared, location)
        })
        .collect()
}
