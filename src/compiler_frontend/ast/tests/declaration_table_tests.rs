//! Stable declaration table regression tests.
//!
//! WHAT: checks the AST environment-owned top-level declaration table independent of parser
//! setup.
//! WHY: phase 3 relies on updates preserving placeholder slots so later lookups observe resolved
//! metadata through the shared environment-owned table.

use super::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn updates_existing_declaration_slot_without_reordering() {
    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first", &mut string_table);
    let second_path = InternedPath::from_single_str("second", &mut string_table);

    let mut table = TopLevelDeclarationTable::new(vec![
        declaration(&first_path, DataType::Inferred),
        declaration(&second_path, DataType::Bool),
    ]);

    table
        .replace_by_path(declaration(&first_path, DataType::StringSlice))
        .expect("existing declaration path should update in place");

    let declarations = table.iter().collect::<Vec<_>>();
    assert_eq!(declarations[0].id, first_path);
    assert_eq!(declarations[0].value.diagnostic_type, DataType::StringSlice);
    assert_eq!(declarations[1].id, second_path);
    assert_eq!(declarations[1].value.diagnostic_type, DataType::Bool);

    let first_name = first_path.name().expect("test path should have a name");
    let by_name = table
        .get_visible_resolved_by_name(first_name, None)
        .expect("name lookup should see updated declaration");
    assert_eq!(by_name.value.diagnostic_type, DataType::StringSlice);
}

#[test]
fn appending_declaration_updates_path_and_name_indexes() {
    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first", &mut string_table);
    let appended_path = InternedPath::from_single_str("appended", &mut string_table);

    let mut table = TopLevelDeclarationTable::new(vec![declaration(&first_path, DataType::Bool)]);
    let appended = declaration(&appended_path, DataType::StringSlice);

    table
        .append_for_construction(appended)
        .expect("new declaration path should append during construction");

    assert_eq!(table.iter().count(), 2);
    assert_eq!(
        table
            .get_by_path(&appended_path)
            .expect("path index should include appended declaration")
            .value
            .diagnostic_type,
        DataType::StringSlice
    );
    assert_eq!(
        table
            .get_visible_resolved_by_name(
                appended_path
                    .name()
                    .expect("appended test path should have a name"),
                None,
            )
            .expect("name index should include appended declaration")
            .id,
        appended_path
    );
    assert!(
        table
            .append_for_construction(declaration(&appended_path, DataType::Bool))
            .is_none()
    );
}

fn declaration(path: &InternedPath, data_type: DataType) -> Declaration {
    Declaration {
        id: path.to_owned(),
        value: Expression::no_value(
            SourceLocation::default(),
            data_type,
            ValueMode::ImmutableOwned,
        ),
    }
}
