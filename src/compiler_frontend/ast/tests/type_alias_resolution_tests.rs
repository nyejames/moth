//! Local type-alias identity invariants after the identity/member split.
//!
//! WHAT: asserts that aliases targeting local nominals, options and collections type-check,
//! and that a nominal field using an earlier alias keeps the target's canonical `TypeId`.
//! WHY: integration output cannot inspect the alias table; these cases own the hidden
//! producer contract that generic materialisation later freezes.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, NameNamespace,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeConstructor, TypeConstructor, TypeId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

fn page_path(name: &str, string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str("@page.moth", string_table).append(string_table.intern(name))
}

fn nominal_type_id(ast: &Ast, string_table: &mut StringTable, name: &str) -> TypeId {
    let path = page_path(name, string_table);
    ast.type_environment
        .nominal_id_for_path(&path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .unwrap_or_else(|| panic!("{name} should have a nominal TypeId"))
}

fn field_type_id(
    ast: &Ast,
    owner: TypeId,
    field_name: &str,
    string_table: &mut StringTable,
) -> TypeId {
    let name = string_table.intern(field_name);
    ast.type_environment
        .field_for(owner, name)
        .map(|field| field.type_id)
        .unwrap_or_else(|| panic!("{field_name} should be a registered field"))
}

#[test]
fn builtin_alias_field_uses_int_identity() {
    let source = r#"
TaskId as Int

Task = |
    id TaskId,
|
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let task_type_id = nominal_type_id(&ast, &mut string_table, "Task");
    assert_eq!(
        field_type_id(&ast, task_type_id, "id", &mut string_table),
        ast.type_environment.builtins().int
    );
}

#[test]
fn local_nominal_option_and_collection_aliases_keep_nominal_identity() {
    let source = r#"
TaskId as Int

Task = |
    id TaskId,
|

TaskList as {Task}
MaybeTask as Task?

Holder = |
    items TaskList,
    maybe MaybeTask,
|
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let task_type_id = nominal_type_id(&ast, &mut string_table, "Task");
    let holder_type_id = nominal_type_id(&ast, &mut string_table, "Holder");

    assert_eq!(
        field_type_id(&ast, task_type_id, "id", &mut string_table),
        ast.type_environment.builtins().int
    );

    let items_type_id = field_type_id(&ast, holder_type_id, "items", &mut string_table);
    assert_eq!(
        ast.type_environment
            .collection_shape(items_type_id)
            .expect("collection alias should resolve to a collection")
            .element_type,
        task_type_id,
        "collection alias element must keep the local nominal identity of Task"
    );

    let maybe_type_id = field_type_id(&ast, holder_type_id, "maybe", &mut string_table);
    assert_eq!(
        ast.type_environment.option_inner_type(maybe_type_id),
        Some(task_type_id),
        "option alias inner type must keep the local nominal identity of Task"
    );
}

#[test]
fn local_choice_alias_keeps_choice_identity() {
    let source = r#"
Priority ::
    Low,
    High,
;

P as Priority

Holder = |
    level P,
|
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let priority_type_id = nominal_type_id(&ast, &mut string_table, "Priority");
    let holder_type_id = nominal_type_id(&ast, &mut string_table, "Holder");

    assert!(
        ast.type_environment
            .choice_definition_for(priority_type_id)
            .is_some(),
        "Priority should be registered as a choice"
    );
    assert_eq!(
        field_type_id(&ast, holder_type_id, "level", &mut string_table),
        priority_type_id,
        "choice alias must keep the local nominal identity of Priority"
    );
}

#[test]
fn alias_chain_resolves_to_the_final_local_nominal() {
    let source = r#"
Item = |
    n Int,
|

Alias as Item
Chain as Alias

Holder = |
    item Chain,
|
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let item_type_id = nominal_type_id(&ast, &mut string_table, "Item");
    let holder_type_id = nominal_type_id(&ast, &mut string_table, "Holder");

    assert_eq!(
        field_type_id(&ast, holder_type_id, "item", &mut string_table),
        item_type_id,
        "alias chain must resolve to the final local nominal identity"
    );
}

#[test]
fn alias_chain_through_a_capacity_constant_resolves_to_the_folded_capacity() {
    let source = r#"
capacity #Int = 2

Names as {capacity String}
MoreNames as Names

Holder = |
    names MoreNames,
|
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let holder_type_id = nominal_type_id(&ast, &mut string_table, "Holder");
    let names_type_id = field_type_id(&ast, holder_type_id, "names", &mut string_table);

    // The member uses an alias that names a constant-dependent alias, so both aliases must be
    // published from the constant walk before this shell is built. A provisional target would
    // leave an unbounded collection here.
    let Some(TypeDefinition::Constructed(collection)) = ast.type_environment.get(names_type_id)
    else {
        panic!("the aliased member should be a constructed collection type");
    };
    assert_eq!(
        collection.constructor,
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: Some(2)
        })
    );
    assert_eq!(
        collection.arguments.as_ref(),
        [ast.type_environment.builtins().string]
    );
}

#[test]
fn unknown_alias_target_is_a_user_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic("Missing as Unknown\n");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnknownName {
                namespace: NameNamespace::Type,
                ..
            }
        ),
        "expected unknown type diagnostic, got {:?}",
        diagnostic.payload
    );

    let location = &diagnostic.primary_location;
    assert_eq!(
        (
            location.start_pos.line_number,
            location.start_pos.char_column
        ),
        (0, 12),
        "unknown alias target must point at the target spelling, got {location:?}"
    );
}

#[test]
fn alias_cycle_remains_circular_dependency() {
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
A as B
B as A
"#,
    );

    assert!(
        matches!(
            diagnostic,
            CompilerDiagnostic {
                payload: DiagnosticPayload::CircularDependency { .. },
                ..
            }
        ),
        "expected circular dependency, got {:?}",
        diagnostic.payload
    );
}
