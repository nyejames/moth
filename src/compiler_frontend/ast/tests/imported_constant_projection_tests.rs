//! Imported-constant declaration identity regression tests.
//!
//! WHAT: exercises the production append-and-publish boundary after semantic metadata holes and
//! compiler-owned rows.
//! WHY: imported constants must receive their ID from the declaration-table owner and publish that
//! exact ID to constant visibility; reconstructing either offset would let the two owners drift.

use super::*;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::headers::module_symbols::{
    CompilerOwnedDeclaration, CompilerOwnedDeclarationKind, DeclarationId,
    OrderedSemanticDeclaration, OrderedSemanticDeclarationKind,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn imported_constant_uses_the_next_table_id_and_publishes_it() {
    let mut string_table = StringTable::new();
    let alias_path = InternedPath::from_single_str("Alias", &mut string_table);
    let trait_path = InternedPath::from_single_str("Trait", &mut string_table);
    let start_path = InternedPath::from_single_str("start", &mut string_table);
    let builtin_path = InternedPath::from_single_str("Builtin", &mut string_table);
    let imported_path = InternedPath::from_single_str("imported", &mut string_table);

    let mut declaration_table = Rc::new(
        TopLevelDeclarationTable::from_stage3_order(
            vec![
                metadata_record(0, &alias_path, OrderedSemanticDeclarationKind::TypeAlias),
                metadata_record(1, &trait_path, OrderedSemanticDeclarationKind::Trait),
            ],
            vec![
                compiler_owned(
                    CompilerOwnedDeclarationKind::Start,
                    declaration(
                        &start_path,
                        DataType::Function(Box::new(None), Default::default()),
                    ),
                ),
                compiler_owned(
                    CompilerOwnedDeclarationKind::Builtin,
                    declaration(&builtin_path, DataType::Inferred),
                ),
            ],
        )
        .expect("semantic holes and compiler-owned rows should build"),
    );
    let mut resolved_constants = Rc::new(ResolvedConstantSet::default());

    let declaration_id = append_projected_constant(
        &mut declaration_table,
        &mut resolved_constants,
        declaration(&imported_path, DataType::Bool),
    )
    .expect("imported constant should append through the production owner");

    assert_eq!(declaration_id.index(), 4);
    assert_eq!(
        declaration_table.declaration_id_by_path(&imported_path),
        Some(declaration_id)
    );
    assert!(resolved_constants.contains(declaration_id));
}

fn metadata_record(
    index: usize,
    path: &InternedPath,
    kind: OrderedSemanticDeclarationKind,
) -> OrderedSemanticDeclaration {
    OrderedSemanticDeclaration {
        declaration_id: DeclarationId::from_index(index),
        header_index: index,
        path: path.clone(),
        kind,
        declaration: None,
    }
}

fn declaration(path: &InternedPath, data_type: DataType) -> Declaration {
    Declaration {
        id: path.clone(),
        value: Expression::no_value(
            SourceLocation::default(),
            data_type,
            ValueMode::ImmutableOwned,
        ),
    }
}

fn compiler_owned(
    kind: CompilerOwnedDeclarationKind,
    declaration: Declaration,
) -> CompilerOwnedDeclaration {
    CompilerOwnedDeclaration { kind, declaration }
}
