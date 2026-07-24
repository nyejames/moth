//! Type-resolution regression tests.
//!
//! WHAT: validates diagnostic-type syntax conversion into canonical type identity.
//! WHY: these paths sit at the boundary between diagnostic type syntax and canonical
//!      type identity; mistakes here produce misleading errors or silent wrong-types.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{
    CollectionExpressionType, Expression, ExpressionKind,
};
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, resolve_diagnostic_type_to_type_id_checked,
    resolve_diagnostic_type_to_type_id_opt, resolve_parsed_type_annotation,
    resolve_struct_field_types, validate_map_key_type,
};
use crate::compiler_frontend::ast::{Ast, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidMapTypeReason, InvalidTypeAnnotationReason, NameNamespace,
    TypeAnnotationContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_result,
};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ---------------------------------------------------------------
//  Checked / optional diagnostic-type-to-TypeId bridge tests
// ---------------------------------------------------------------
//
// WHAT: prove that unresolved parse placeholders cannot silently
//       fall back to builtin `none` in production paths.
// WHY: the frontend type boundary cleanup removed unchecked
//      `resolve_diagnostic_type_to_type_id` from constructor
//      shells; these tests guard against regression.

#[test]
fn checked_conversion_rejects_inferred_type() {
    let mut type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
    let location = SourceLocation::default();

    let error = resolve_diagnostic_type_to_type_id_checked(
        &DataType::Inferred,
        &mut type_environment,
        &location,
    )
    .expect_err("checked conversion should reject Inferred placeholder");

    assert!(
        matches!(
            &error.payload,
            DiagnosticPayload::InvalidTypeAnnotation {
                context: TypeAnnotationContext::DeclarationTarget,
                reason: InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                    found: TokenKind::Eof
                },
            }
        ),
        "expected InvalidTypeAnnotation for Inferred, got {:?}",
        error.payload
    );
}

#[test]
fn checked_conversion_rejects_unresolved_namespaced_type() {
    let mut string_table = StringTable::new();
    let mut type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
    let location = SourceLocation::default();
    let namespace = string_table.intern("missing");
    let name = string_table.intern("Type");

    let error = resolve_diagnostic_type_to_type_id_checked(
        &DataType::NamespacedType {
            path: vec![namespace, name],
        },
        &mut type_environment,
        &location,
    )
    .expect_err("checked conversion should reject unresolved namespaced type");

    assert!(
        matches!(
            &error.payload,
            DiagnosticPayload::UnknownName {
                name: n,
                namespace: NameNamespace::Type,
            } if *n == name
        ),
        "expected UnknownName(type) for namespaced type, got {:?}",
        error.payload
    );
}

#[test]
fn optional_conversion_returns_none_for_unresolved_named_type() {
    let mut string_table = StringTable::new();
    let mut type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();
    let missing = string_table.intern("Missing");

    let result = resolve_diagnostic_type_to_type_id_opt(
        &DataType::NamedType(missing),
        &mut type_environment,
    );

    assert_eq!(
        result, None,
        "optional conversion must return None for unresolved named type"
    );
}

#[test]
fn optional_conversion_returns_none_for_inferred_type() {
    let mut type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();

    let result = resolve_diagnostic_type_to_type_id_opt(&DataType::Inferred, &mut type_environment);

    assert_eq!(
        result, None,
        "optional conversion must return None for Inferred placeholder"
    );
}

#[test]
fn optional_conversion_returns_some_for_resolved_builtin() {
    let mut type_environment =
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new();

    let result = resolve_diagnostic_type_to_type_id_opt(&DataType::Int, &mut type_environment);

    assert_eq!(
        result,
        Some(builtin_type_ids::INT),
        "optional conversion must return Some for resolved builtin"
    );
}

// ---------------------------------------------------------------
//  Fixed-collection capacity folding tests
// ---------------------------------------------------------------

#[test]
fn literal_capacity_resolves_to_fixed_collection() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        }),
        location: location.clone(),
        fixed_capacity: Some(ParsedCollectionCapacity::Literal {
            value: 64,
            location: location.clone(),
        }),
    };

    let resolved = resolve_parsed_type_annotation(
        parsed,
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("literal capacity should resolve");

    let type_id = resolved.type_id.expect("should have a type id");
    assert_eq!(
        resolution_context
            .type_environment
            .collection_fixed_capacity(type_id),
        Some(64),
        "capacity should be folded to 64"
    );
}

#[test]
fn constant_capacity_resolves_to_fixed_collection() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let capacity_name = string_table.intern("capacity");

    // Build a scope context with a local constant declaration.
    let mut scope_context = ScopeContext::new_for_tests(
        ContextKind::Function,
        InternedPath::new(),
        declaration_table.clone(),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    let constant_declaration = Declaration {
        id: InternedPath::from_components(vec![capacity_name]),
        value: Expression::new(
            ExpressionKind::Int(42),
            location.clone(),
            builtin_type_ids::INT,
            DataType::Int,
            ValueMode::ImmutableOwned,
        ),
    };
    scope_context.add_compile_time_var(constant_declaration, SourceLocation::default());

    let parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        }),
        location: location.clone(),
        fixed_capacity: Some(ParsedCollectionCapacity::BareConstant {
            name: capacity_name,
            location: location.clone(),
        }),
    };

    let resolved = resolve_parsed_type_annotation(
        parsed,
        &location,
        &mut resolution_context,
        &mut string_table,
        Some(&scope_context),
    )
    .expect("constant capacity should resolve");

    let type_id = resolved.type_id.expect("should have a type id");
    assert_eq!(
        resolution_context
            .type_environment
            .collection_fixed_capacity(type_id),
        Some(42),
        "capacity should be folded from constant to 42"
    );
}

#[test]
fn nested_fixed_collections_fold_both_capacities() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let inner = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        }),
        location: location.clone(),
        fixed_capacity: Some(ParsedCollectionCapacity::Literal {
            value: 4,
            location: location.clone(),
        }),
    };
    let outer = ParsedTypeRef::Collection {
        element: Box::new(inner),
        location: location.clone(),
        fixed_capacity: Some(ParsedCollectionCapacity::Literal {
            value: 8,
            location: location.clone(),
        }),
    };

    let resolved = resolve_parsed_type_annotation(
        outer,
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("nested fixed collections should resolve");

    let outer_type_id = resolved.type_id.expect("should have a type id");
    let outer_shape = resolution_context
        .type_environment
        .collection_shape(outer_type_id)
        .expect("outer should be a collection");
    assert_eq!(
        outer_shape.fixed_capacity,
        Some(8),
        "outer capacity should be 8"
    );

    let inner_type_id = outer_shape.element_type;
    let inner_shape = resolution_context
        .type_environment
        .collection_shape(inner_type_id)
        .expect("inner should be a collection");
    assert_eq!(
        inner_shape.fixed_capacity,
        Some(4),
        "inner capacity should be 4"
    );
}

#[test]
fn struct_field_alias_bare_capacity_constant_folds_after_constants() {
    let source = r#"
capacity #Int = 4
Names as {capacity String}

Buffer = |
    items Names,
|
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);

    let buffer_name = string_table.intern("Buffer");
    let buffer_path =
        InternedPath::from_single_str("#page.moth", &mut string_table).append(buffer_name);
    let buffer_type_id = ast
        .type_environment
        .nominal_id_for_path(&buffer_path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("Buffer should have a nominal TypeId");
    let items_name = string_table.intern("items");
    let items_field = ast
        .type_environment
        .field_for(buffer_type_id, items_name)
        .expect("Buffer.items should be registered");

    assert_eq!(
        ast.type_environment
            .collection_fixed_capacity(items_field.type_id),
        Some(4),
        "type alias capacity should fold before final struct field TypeId registration"
    );
}

/// Builds a slot-bearing template registered directly in a TIR store.
///
/// WHAT: the Composed TIR root contains only a slot.
/// WHY: struct-field const inlining must resolve the root through the supplied module store.
fn slot_field_default_template(template_ir_store: &mut TemplateIrStore) -> Template {
    let location = SourceLocation::default();
    let mut builder = TemplateIrBuilder::new(template_ir_store);
    let slot_node = builder.push_slot_node(SlotKey::Default, location.clone());
    let template_id = builder.finish_template(
        slot_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        location.clone(),
    );

    Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location,
    }
}

#[test]
fn struct_field_default_inlines_slot_template_through_module_store() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut type_environment = TypeEnvironment::new();
    let location = SourceLocation::default();
    let wrapper_path = InternedPath::from_single_str("wrapper", &mut string_table);
    let wrapper_template = slot_field_default_template(&mut template_ir_store.borrow_mut());
    let wrapper_declaration = Declaration {
        id: wrapper_path.clone(),
        value: Expression::template(wrapper_template, ValueMode::ImmutableOwned),
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(vec![wrapper_declaration]));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);
    let struct_path = InternedPath::from_single_str("Card", &mut string_table);
    let field_path = struct_path.clone().append(string_table.intern("content"));
    let field = Declaration {
        id: field_path,
        value: Expression::reference(
            wrapper_path,
            DataType::Template,
            location,
            ValueMode::ImmutableReference,
        ),
    };

    let resolved_fields = resolve_struct_field_types(
        &struct_path,
        &[field],
        &mut resolution_context,
        &template_ir_store,
        &mut string_table,
    )
    .unwrap_or_else(|_| {
        panic!("slot-bearing const template should inline as a valid field default")
    });

    assert!(matches!(
        resolved_fields[0].value.kind,
        ExpressionKind::Template(_)
    ));
}

#[test]
fn struct_field_constant_inlining_preserves_surrounding_provenance() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut type_environment = TypeEnvironment::new();
    let location = SourceLocation::default();
    let constant_path = InternedPath::from_single_str("value", &mut string_table);
    let project_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let builder_member =
        SyntheticInterfaceMemberIdentity::new(SyntheticInterfaceClass::Builder, "assets", "bundle");
    let constant = Declaration {
        id: constant_path.clone(),
        value: Expression::int(7, location.clone(), ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
                project_member.clone(),
            )),
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(vec![constant]));
    let collection_type_id = type_environment.intern_collection(builtin_type_ids::INT, None);
    let collection = Expression::collection_with_type_id(
        vec![Expression::reference_with_type_id(
            constant_path,
            DataType::Int,
            builtin_type_ids::INT,
            location.clone(),
            ValueMode::ImmutableReference,
            crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
        )
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            project_member.clone(),
        ))],
        CollectionExpressionType {
            element_type_id: builtin_type_ids::INT,
            element_diagnostic_type: DataType::Int,
            fixed_capacity: None,
            collection_type_id: Some(collection_type_id),
        },
        &mut type_environment,
        location.clone(),
        ValueMode::ImmutableOwned,
    )
    .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
        builder_member.clone(),
    ));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);
    let struct_path = InternedPath::from_single_str("Card", &mut string_table);
    let field = Declaration {
        id: struct_path.clone().append(string_table.intern("values")),
        value: collection,
    };

    let resolved_fields = resolve_struct_field_types(
        &struct_path,
        &[field],
        &mut resolution_context,
        &template_ir_store,
        &mut string_table,
    )
    .unwrap_or_else(|_| panic!("collection field default should resolve"));

    assert_eq!(
        resolved_fields[0]
            .value
            .synthetic_interface_provenance
            .members(),
        &[builder_member]
    );
    let ExpressionKind::Collection(elements) = &resolved_fields[0].value.kind else {
        panic!("expected collection field default");
    };
    assert_eq!(
        elements[0].synthetic_interface_provenance.members(),
        &[project_member]
    );
}

#[test]
fn function_return_bare_capacity_constant_folds_to_type_id() {
    let source = r#"
capacity #Int = 3

make || -> {capacity Int}:
    assert(false, "signature only")
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);

    let make_name = string_table.intern("make");
    let make_path =
        InternedPath::from_single_str("#page.moth", &mut string_table).append(make_name);
    let signature = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _) if path == &make_path => Some(signature),
            _ => None,
        })
        .expect("make signature should be registered");
    let return_type_id = signature.returns[0]
        .type_id
        .expect("return slot should have a TypeId");

    assert_eq!(
        ast.type_environment
            .collection_fixed_capacity(return_type_id),
        Some(3),
        "fixed capacity in return annotation should fold into the canonical return TypeId"
    );
}

// ---------------------------------------------------------------
//  Map type resolution tests
// ---------------------------------------------------------------

fn page_path(name: &str, string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str("#page.moth", string_table).append(string_table.intern(name))
}

fn nominal_type_id(ast: &Ast, string_table: &mut StringTable, name: &str) -> TypeId {
    let path = page_path(name, string_table);
    ast.type_environment
        .nominal_id_for_path(&path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .unwrap_or_else(|| panic!("{name} should have a nominal TypeId"))
}

fn assert_map_shape(ast: &Ast, type_id: TypeId, expected_key: TypeId, expected_value: TypeId) {
    assert!(
        ast.type_environment.is_map_type(type_id),
        "expected {:?} to be a map TypeId",
        type_id
    );
    assert_eq!(
        ast.type_environment.map_key_type(type_id),
        Some(expected_key),
        "map key TypeId mismatch"
    );
    assert_eq!(
        ast.type_environment.map_value_type(type_id),
        Some(expected_value),
        "map value TypeId mismatch"
    );
}

fn map_value_type(ast: &Ast, type_id: TypeId) -> TypeId {
    ast.type_environment
        .map_value_type(type_id)
        .expect("expected map value TypeId")
}

fn assert_source_invalid_map_type(
    source: &str,
    expected_reason: impl FnOnce(&InvalidMapTypeReason) -> bool,
    expected_description: &str,
) {
    let diagnostic = match parse_single_file_ast_result(source) {
        Ok(_) => panic!("{expected_description}: source parsed successfully"),
        Err(diagnostic) => diagnostic.as_ref().to_owned(),
    };
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidMapType { reason } if expected_reason(reason)
        ),
        "expected {expected_description}, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn map_type_syntax_resolves_across_source_type_surfaces() {
    let source = r#"
StringScores as {String = Int}
NestedScores as {String = StringScores}

User = |
    scores StringScores,
    grouped {Bool = StringScores},
|

Payload ::
    WithMap | values NestedScores |
;

use_maps |scores StringScores, direct {Bool = {String = Int}}| -> {Char = StringScores}:
    assert(false, "signature only")
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let builtins = *ast.type_environment.builtins();

    let user_type_id = nominal_type_id(&ast, &mut string_table, "User");
    let scores_name = string_table.intern("scores");
    let scores_type_id = ast
        .type_environment
        .field_for(user_type_id, scores_name)
        .expect("User.scores should be registered")
        .type_id;
    assert_map_shape(&ast, scores_type_id, builtins.string, builtins.int);

    let grouped_name = string_table.intern("grouped");
    let grouped_type_id = ast
        .type_environment
        .field_for(user_type_id, grouped_name)
        .expect("User.grouped should be registered")
        .type_id;
    assert_map_shape(&ast, grouped_type_id, builtins.bool, scores_type_id);
    assert_map_shape(
        &ast,
        map_value_type(&ast, grouped_type_id),
        builtins.string,
        builtins.int,
    );

    let function_path = page_path("use_maps", &mut string_table);
    let signature = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _) if path == &function_path => Some(signature),
            _ => None,
        })
        .expect("use_maps signature should be registered");
    assert_map_shape(
        &ast,
        signature.parameters[0].value.type_id,
        builtins.string,
        builtins.int,
    );
    assert_map_shape(
        &ast,
        signature.parameters[1].value.type_id,
        builtins.bool,
        scores_type_id,
    );
    let return_type_id = signature.returns[0]
        .type_id
        .expect("map return should have a TypeId");
    assert_map_shape(&ast, return_type_id, builtins.char, scores_type_id);

    let payload_type_id = nominal_type_id(&ast, &mut string_table, "Payload");
    let payload_definition = ast
        .type_environment
        .choice_definition_for(payload_type_id)
        .expect("Payload should be a choice");
    let with_map_name = string_table.intern("WithMap");
    let with_map = payload_definition
        .variants
        .iter()
        .find(|variant| variant.name == with_map_name)
        .expect("WithMap variant should be registered");
    let ChoiceVariantPayloadDefinition::Record { fields } = &with_map.payload else {
        panic!("WithMap should have a record payload");
    };
    let values_type_id = fields
        .iter()
        .find(|field| field.name.name_str(&string_table) == Some("values"))
        .expect("WithMap.values should be registered")
        .type_id;
    assert_map_shape(&ast, values_type_id, builtins.string, scores_type_id);
}

#[test]
fn map_type_syntax_rejects_invalid_source_key_types() {
    assert_source_invalid_map_type(
        "bad |items {Float = Int}|:\n;\n",
        |reason| matches!(reason, InvalidMapTypeReason::UnsupportedKeyType { .. }),
        "UnsupportedKeyType for Float keys",
    );
    assert_source_invalid_map_type(
        "User = |\n    id Int,\n|\n\nbad |items {User = Int}|:\n;\n",
        |reason| matches!(reason, InvalidMapTypeReason::UnsupportedKeyType { .. }),
        "UnsupportedKeyType for struct keys",
    );
    assert_source_invalid_map_type(
        "bad |items {{String} = Int}|:\n;\n",
        |reason| matches!(reason, InvalidMapTypeReason::UnsupportedKeyType { .. }),
        "UnsupportedKeyType for collection keys",
    );
    assert_source_invalid_map_type(
        "bad |items {{String = Int} = Int}|:\n;\n",
        |reason| matches!(reason, InvalidMapTypeReason::UnsupportedKeyType { .. }),
        "UnsupportedKeyType for map keys",
    );
    assert_source_invalid_map_type(
        "bad |items {String = {String = {String = Int}}}|:\n;\n",
        |reason| {
            matches!(
                reason,
                InvalidMapTypeReason::ExcessiveInlineNesting { depth: 3 }
            )
        },
        "ExcessiveInlineNesting for depth-three inline maps",
    );
    assert_source_invalid_map_type(
        "Holder type Key = |\n    values {Key = Int},\n|\n",
        |reason| matches!(reason, InvalidMapTypeReason::UnsupportedKeyType { .. }),
        "UnsupportedKeyType for generic map keys",
    );
}

#[test]
fn map_type_resolves_for_supported_key() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString {
            location: location.clone(),
        }),
        value: Box::new(ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        }),
        location: location.clone(),
    };

    let resolved = resolve_parsed_type_annotation(
        parsed,
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("String key map should resolve");

    let type_id = resolved.type_id.expect("should have a type id");
    assert!(resolution_context.type_environment.is_map_type(type_id));
    assert_eq!(
        resolution_context.type_environment.map_key_type(type_id),
        Some(resolution_context.type_environment.builtins().string)
    );
    assert_eq!(
        resolution_context.type_environment.map_value_type(type_id),
        Some(resolution_context.type_environment.builtins().int)
    );
}

#[test]
fn map_type_rejects_unsupported_key() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinFloat {
            location: location.clone(),
        }),
        value: Box::new(ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        }),
        location: location.clone(),
    };

    let error = resolve_parsed_type_annotation(
        parsed,
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("Float key should be rejected");

    assert!(
        matches!(
            &error.payload,
            DiagnosticPayload::InvalidMapType {
                reason: crate::compiler_frontend::compiler_messages::InvalidMapTypeReason::UnsupportedKeyType { .. },
                ..
            }
        ),
        "expected UnsupportedKeyType error, got {:?}",
        error.payload
    );
}

#[test]
fn map_key_capability_rejects_generic_key_as_unsupported() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let parameter_name = string_table.intern("Key");
    let key_type_id = type_environment.register_synthetic_generic_parameter(parameter_name);

    let error = validate_map_key_type(key_type_id, &type_environment, &SourceLocation::default())
        .expect_err("generic map keys should be rejected by the scalar-key policy");

    assert!(
        matches!(
            &error.payload,
            DiagnosticPayload::InvalidMapType {
                reason: InvalidMapTypeReason::UnsupportedKeyType { key_type },
                ..
            } if *key_type == key_type_id
        ),
        "expected UnsupportedKeyType error, got {:?}",
        error.payload
    );
}

#[test]
fn map_type_rejects_excessive_inline_nesting() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    // {String = {String = {String = Int}}} is depth 3 and should be rejected
    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString {
            location: location.clone(),
        }),
        value: Box::new(ParsedTypeRef::Map {
            key: Box::new(ParsedTypeRef::BuiltinString {
                location: location.clone(),
            }),
            value: Box::new(ParsedTypeRef::Map {
                key: Box::new(ParsedTypeRef::BuiltinString {
                    location: location.clone(),
                }),
                value: Box::new(ParsedTypeRef::BuiltinInt {
                    location: location.clone(),
                }),
                location: location.clone(),
            }),
            location: location.clone(),
        }),
        location: location.clone(),
    };

    let error = resolve_parsed_type_annotation(
        parsed,
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect_err("depth-3 nesting should be rejected");

    assert!(
        matches!(
            &error.payload,
            DiagnosticPayload::InvalidMapType {
                reason: crate::compiler_frontend::compiler_messages::InvalidMapTypeReason::ExcessiveInlineNesting { depth: 3 },
                ..
            }
        ),
        "expected ExcessiveInlineNesting error, got {:?}",
        error.payload
    );
}

#[test]
fn map_type_allows_two_level_nesting() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new()));
    let mut resolution_context =
        TypeResolutionContext::from_declaration_table(&declaration_table, &mut type_environment);

    let location = SourceLocation::default();
    // {String = {String = Int}} is depth 2 and should be allowed
    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString {
            location: location.clone(),
        }),
        value: Box::new(ParsedTypeRef::Map {
            key: Box::new(ParsedTypeRef::BuiltinString {
                location: location.clone(),
            }),
            value: Box::new(ParsedTypeRef::BuiltinInt {
                location: location.clone(),
            }),
            location: location.clone(),
        }),
        location: location.clone(),
    };

    let resolved = resolve_parsed_type_annotation(
        parsed,
        &location,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("depth-2 nesting should be allowed");

    let type_id = resolved.type_id.expect("should have a type id");
    assert!(resolution_context.type_environment.is_map_type(type_id));
}
