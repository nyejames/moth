//! HIR const fact projection tests.
//!
//! WHAT: asserts that AST const facts are correctly projected into HIR metadata
//!       during lowering without affecting HIR shape or semantics.
//! WHY: the facts are advisory; their only requirement is faithful copy/projection.

use crate::compiler_frontend::ast::const_values::facts::{
    AstConstDeclarationFact, AstConstFacts, ConstBindingScope, ConstBindingSource,
    ConstFactValueKind,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::hir::const_facts::HirConstFacts;
use crate::compiler_frontend::hir::hir_builder::{build_ast, lower_ast};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{function_node, test_location};

use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn projects_ast_const_facts_into_hir_metadata() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let mut ast = build_ast(vec![start_function], entry_path.clone());

    let explicit_path = InternedPath::from_single_str("site_name", &mut string_table);
    let private_path = InternedPath::from_single_str("page_title", &mut string_table);

    ast.const_facts.declarations.insert(
        explicit_path.clone(),
        AstConstDeclarationFact {
            declaration_path: explicit_path.clone(),
            scope: ConstBindingScope::ExplicitTopLevel,
            source: ConstBindingSource::ExplicitHash,
            value_kind: ConstFactValueKind::Literal,
            resolved_expression: Expression::string_slice(
                string_table.intern("Moth"),
                test_location(2),
                ValueMode::ImmutableOwned,
            ),
            location: test_location(2),
        },
    );

    ast.const_facts.declarations.insert(
        private_path.clone(),
        AstConstDeclarationFact {
            declaration_path: private_path.clone(),
            scope: ConstBindingScope::PrivateTopLevel,
            source: ConstBindingSource::InferredImmutable,
            value_kind: ConstFactValueKind::Literal,
            resolved_expression: Expression::int(42, test_location(3), ValueMode::ImmutableOwned),
            location: test_location(3),
        },
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    assert_eq!(module.const_facts.declarations.len(), 2);

    let explicit = module
        .const_facts
        .declarations
        .get(&explicit_path)
        .expect("explicit fact should be present");
    assert_eq!(explicit.declaration_path, explicit_path);
    assert_eq!(explicit.scope, ConstBindingScope::ExplicitTopLevel);
    assert_eq!(explicit.source, ConstBindingSource::ExplicitHash);
    assert_eq!(explicit.value_kind, ConstFactValueKind::Literal);
    assert_eq!(explicit.location, test_location(2));

    let private = module
        .const_facts
        .declarations
        .get(&private_path)
        .expect("private fact should be present");
    assert_eq!(private.declaration_path, private_path);
    assert_eq!(private.scope, ConstBindingScope::PrivateTopLevel);
    assert_eq!(private.source, ConstBindingSource::InferredImmutable);
    assert_eq!(private.value_kind, ConstFactValueKind::Literal);
    assert_eq!(private.location, test_location(3));
}

#[test]
fn empty_ast_const_facts_produces_empty_hir_const_facts() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let ast = build_ast(vec![start_function], entry_path);

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    assert!(module.const_facts.declarations.is_empty());
}

#[test]
fn remaps_const_fact_keys_and_payload_paths() {
    let mut source_table = StringTable::new();
    let original_path = InternedPath::from_single_str("site_name", &mut source_table);
    let location = SourceLocation::new(
        original_path.clone(),
        CharPosition {
            line_number: 4,
            char_column: 0,
        },
        CharPosition {
            line_number: 4,
            char_column: 9,
        },
    );

    let mut ast_facts = AstConstFacts::default();
    ast_facts.declarations.insert(
        original_path.clone(),
        AstConstDeclarationFact {
            declaration_path: original_path.clone(),
            scope: ConstBindingScope::ExplicitTopLevel,
            source: ConstBindingSource::ExplicitHash,
            value_kind: ConstFactValueKind::Literal,
            resolved_expression: Expression::int(1, location.clone(), ValueMode::ImmutableOwned),
            location,
        },
    );

    let mut hir_facts = HirConstFacts::from(&ast_facts);
    let mut target_table = StringTable::new();
    target_table.intern("prefix");
    let remap = target_table.merge_from(&source_table);

    let mut remapped_path = original_path.clone();
    remapped_path.remap_string_ids(&remap);
    hir_facts.remap_string_ids(&remap);

    assert!(!hir_facts.declarations.contains_key(&original_path));
    let fact = hir_facts
        .declarations
        .get(&remapped_path)
        .expect("remapped fact should be keyed by remapped path");
    assert_eq!(fact.declaration_path, remapped_path);
    assert_eq!(fact.location.scope, remapped_path);
}
