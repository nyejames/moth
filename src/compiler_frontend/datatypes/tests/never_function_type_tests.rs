//! Exact function-type identity for the Never return contract.
//!
//! WHAT: proves that a `Never` function-type contract is its own interned identity, is not equal
//!       to an ordinary no-value contract, survives substitution, and spells as `-> !`.
//! WHY: Moth has no Never `TypeId` or value type, so the containing function type is the only
//!       owner that can distinguish `Function() -> !` from `Function() ->`. Lossy identity there
//!       would make exact contract matching and public fingerprints unsound.

use crate::compiler_frontend::ast::type_resolution::intern_resolved_function_signature;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    FunctionTypeKey, GenericParameterId, TypeId, builtin_type_ids,
};
use crate::compiler_frontend::datatypes::{CallableReturnShape, FunctionReturnContract};
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::function_signature_by_name;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use rustc_hash::FxHashMap;

fn never_key(parameters: Box<[TypeId]>) -> FunctionTypeKey {
    FunctionTypeKey {
        parameters,
        return_contract: FunctionReturnContract::Never,
    }
}

fn ordinary_no_value_key(parameters: Box<[TypeId]>) -> FunctionTypeKey {
    FunctionTypeKey {
        parameters,
        return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots([], None)),
    }
}

#[test]
fn never_and_ordinary_no_value_contracts_are_distinct_identities() {
    let mut environment = TypeEnvironment::new();

    let never = environment.intern_function(never_key(Box::new([])));
    let never_again = environment.intern_function(never_key(Box::new([])));
    let ordinary = environment.intern_function(ordinary_no_value_key(Box::new([])));

    assert_eq!(
        never, never_again,
        "identical Never keys must reuse the canonical function TypeId"
    );
    assert_ne!(
        never, ordinary,
        "Never and an ordinary no-value contract must not share a function TypeId"
    );

    let Some(TypeDefinition::Function(never_definition)) = environment.get(never) else {
        panic!("the Never function type must intern as a function definition");
    };
    assert!(never_definition.return_contract.is_never());
    assert!(never_definition.return_contract.ordinary().is_none());

    let Some(TypeDefinition::Function(ordinary_definition)) = environment.get(ordinary) else {
        panic!("the no-value function type must intern as a function definition");
    };
    assert!(!ordinary_definition.return_contract.is_never());
    assert_eq!(
        ordinary_definition
            .return_contract
            .ordinary()
            .expect("an ordinary contract keeps its shape")
            .success(),
        &[]
    );
}

#[test]
fn substitution_preserves_the_never_contract_variant() {
    let mut environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter = environment.intern_generic_parameter(parameter_id, string_table.intern("T"));
    let int_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let generic_function = environment.intern_function(never_key(Box::new([parameter])));
    let mut mapping = FxHashMap::default();
    mapping.insert(parameter_id, int_type_id);

    let substituted = environment.substitute_type_id(generic_function, &mapping);
    let substituted_again = environment.substitute_type_id(generic_function, &mapping);
    assert_eq!(
        substituted, substituted_again,
        "repeated substitution must reuse the canonical Never function TypeId"
    );

    let Some(TypeDefinition::Function(definition)) = environment.get(substituted) else {
        panic!("substitution must preserve the function type definition");
    };
    assert_eq!(definition.parameters[0].type_id, int_type_id);
    assert!(
        definition.return_contract.is_never(),
        "substitution must not rewrite Never into an empty ordinary contract"
    );
}

#[test]
fn never_function_type_display_spells_a_bare_bang_after_the_arrow() {
    let mut environment = TypeEnvironment::new();
    let paths = PathInternerBuilder::new();
    let strings = StringTable::new();
    let int_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let function = environment.intern_function(never_key(Box::new([int_type_id])));

    assert_eq!(
        display_type(function, &environment, &strings, paths.paths()),
        "Function(I32 -> !)"
    );
}

#[test]
fn declared_function_value_type_preserves_the_never_contract() {
    let (mut ast, path_fork, string_table) = parse_single_file_ast(
        "fatal || -> !:\n    assert(false, \"fatal\")\n;\n\nplain ||:\n    return\n;\n",
    );
    let fatal_signature =
        function_signature_by_name(&ast, &path_fork, &string_table, "fatal").clone();
    let plain_signature =
        function_signature_by_name(&ast, &path_fork, &string_table, "plain").clone();
    let fatal_type =
        intern_resolved_function_signature(&fatal_signature, &mut ast.type_environment);
    let plain_type =
        intern_resolved_function_signature(&plain_signature, &mut ast.type_environment);

    let Some(TypeDefinition::Function(fatal_definition)) = ast.type_environment.get(fatal_type)
    else {
        panic!("a declared `-> !` function must intern as a function type");
    };
    assert!(
        fatal_definition.return_contract.is_never(),
        "intern_resolved_function_signature must preserve the Never contract"
    );
    assert_ne!(
        fatal_type, plain_type,
        "a Never function value must not share a TypeId with an ordinary no-value function"
    );
}
