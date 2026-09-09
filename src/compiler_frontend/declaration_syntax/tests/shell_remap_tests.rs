//! String-ID remapping tests for declaration shells and type-annotation helpers.
//!
//! WHAT: verifies that `DeclarationSyntax`, `InitializerReference`,
//!      and `ParsedTypeRef::Collection` can be remapped after a string-table merge.
//! WHY: header parsing produces declaration shells using local string tables; remapping must
//!      preserve all names, type annotations, and initializer tokens.

use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, InitializerReference,
};
use crate::compiler_frontend::source::LocalSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind};

fn make_symbol_token(name: StringId) -> Token {
    Token::new(TokenKind::Symbol(name), LocalSpan::source_start())
}

#[test]
fn collection_capacity_bare_constant_remap() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let cap_name = local.intern("capacity");

    let mut parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinInt { span: None }),
        span: None,
        fixed_capacity: Some(ParsedCollectionCapacity::BareConstant {
            name: cap_name,
            span: None,
        }),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            ..
        } => {
            assert_eq!(*element, ParsedTypeRef::BuiltinInt { span: None });
            let capacity = fixed_capacity.expect("capacity should be present");
            match capacity {
                ParsedCollectionCapacity::BareConstant { name, .. } => {
                    assert_eq!(global.resolve(name), "capacity");
                }
                other => panic!("expected BareConstant, got {:?}", other),
            }
        }
        other => panic!("expected Collection, got {:?}", other),
    }
}

#[test]
fn initializer_reference_remaps_names() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let ref_name = local.intern("other_const");
    let member_name = local.intern("content");

    let mut reference = InitializerReference {
        name: ref_name,
        dot_member: Some(member_name),
        span: None,
        followed_by_call: false,
        followed_by_choice_namespace: false,
    };

    let remap = global.merge_from(&local);
    reference.remap_string_ids(&remap);

    assert_eq!(global.resolve(reference.name), "other_const");
    assert_eq!(
        reference.dot_member.map(|member| global.resolve(member)),
        Some("content")
    );
}

#[test]
fn declaration_syntax_remaps_all_fields() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let type_name = local.intern("String");
    let init_name = local.intern("init_value");
    let ref_name = local.intern("ref_value");

    let mut declaration = DeclarationSyntax {
        span: None,
        binding_mode: BindingMode::MutableRuntime,
        type_annotation: ParsedTypeRef::Named {
            name: type_name,
            span: None,
        },
        config_qualifier: None,
        initializer_tokens: vec![make_symbol_token(init_name)],
        initializer_references: vec![InitializerReference {
            name: ref_name,
            dot_member: None,
            span: None,
            followed_by_call: true,
            followed_by_choice_namespace: false,
        }],
    };

    let remap = global.merge_from(&local);
    declaration.remap_string_ids(&remap);

    match declaration.type_annotation {
        ParsedTypeRef::Named { name, .. } => {
            assert_eq!(global.resolve(name), "String");
        }
        _ => panic!("expected Named type annotation"),
    }

    assert_eq!(declaration.initializer_tokens.len(), 1);
    match &declaration.initializer_tokens[0].kind {
        TokenKind::Symbol(id) => {
            assert_eq!(global.resolve(*id), "init_value");
        }
        _ => panic!("expected Symbol token"),
    }

    assert_eq!(declaration.initializer_references.len(), 1);
    assert_eq!(
        global.resolve(declaration.initializer_references[0].name),
        "ref_value"
    );
}
