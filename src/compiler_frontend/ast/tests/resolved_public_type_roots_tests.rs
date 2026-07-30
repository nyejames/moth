//! Focused unit tests for the transient AST-owned resolved public type-root handoff.
//!
//! WHAT: validates that `build_resolved_public_type_roots` admits only directly-defined
//! active-root public declarations, retains every required root category with resolved
//! `TypeId` facts, keeps deterministic sorted-header order, selects receiver methods in a
//! separate pass by public nominal receiver ownership (ignoring the method's own export
//! mode), and reports missing resolved facts as internal `CompilerError` invariants.
//! WHY: the root table is a hidden side-table consumed immediately before HIR lowering;
//! integration output cannot expose its contents or ordering.

use super::environment::resolved_public_type_roots::ResolvedPublicTypeRootKind;
use super::environment::{
    BuildResolvedPublicTypeRootsInput, ResolvedPublicTypeRootTable, TopLevelDeclarationTable,
    build_resolved_public_type_roots,
};
use super::scope_context::{ReceiverMethodCatalog, ReceiverMethodEntry};
use crate::compiler_frontend::ast::ResolvedTraitSourceFact;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionReturn, FunctionSignature, ReturnSlot,
};
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedFunctionSignature, ResolvedTypeAnnotation,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{GenericParameterListId, NominalTypeId, TypeId};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileRole, Header, HeaderExportMode, HeaderKind,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::traits::definitions::{ResolvedTraitDefinition, TraitVisibility};
use crate::compiler_frontend::traits::environment::{CoreTraitKind, TraitEnvironment};
use crate::compiler_frontend::traits::ids::TraitId;
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::{FxHashMap, FxHashSet};

fn path(name: &str, string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str(name, string_table)
}

fn header(
    kind: HeaderKind,
    src_path: InternedPath,
    file_role: FileRole,
    export_mode: HeaderExportMode,
    string_table: &mut StringTable,
) -> Header {
    Header {
        kind,
        file_role,
        export_mode,
        local_ordering_hints: std::collections::HashSet::new(),
        name_location: SourceLocation::default(),
        tokens: FileTokens::new(src_path, Vec::new()),
        source_file: InternedPath::from_single_str("root.moth", string_table),
        capacity_references: Vec::new(),
    }
}

fn function_kind() -> HeaderKind {
    HeaderKind::Function {
        generic_parameters: Default::default(),
        signature: Default::default(),
    }
}

fn struct_kind() -> HeaderKind {
    HeaderKind::Struct {
        generic_parameters: Default::default(),
        fields: Vec::new(),
    }
}

fn choice_kind() -> HeaderKind {
    HeaderKind::Choice {
        generic_parameters: Default::default(),
        variants: Vec::new(),
    }
}

fn alias_kind() -> HeaderKind {
    HeaderKind::TypeAlias {
        target: ParsedTypeRef::Inferred,
    }
}

fn constant_kind() -> HeaderKind {
    HeaderKind::Constant {
        declaration: DeclarationSyntax {
            binding_mode: BindingMode::default(),
            type_annotation: ParsedTypeRef::Inferred,
            initializer_tokens: Vec::new(),
            initializer_references: Vec::new(),
            location: SourceLocation::default(),
        },
    }
}

fn resolved_free_signature(int_type_id: TypeId) -> ResolvedFunctionSignature {
    let parameter = Declaration {
        id: InternedPath::new(),
        value: Expression::no_value_with_type_id(
            SourceLocation::default(),
            DataType::Int,
            int_type_id,
            ValueMode::default(),
        ),
    };
    let mut return_slot = ReturnSlot::success(FunctionReturn::Value(DataType::Int));
    return_slot.type_id = Some(int_type_id);
    ResolvedFunctionSignature {
        receiver: None,
        signature: FunctionSignature {
            parameters: vec![parameter],
            returns: vec![return_slot],
        },
    }
}

fn receiver_signature(int_type_id: TypeId) -> FunctionSignature {
    let parameter = Declaration {
        id: InternedPath::new(),
        value: Expression::no_value_with_type_id(
            SourceLocation::default(),
            DataType::Int,
            int_type_id,
            ValueMode::default(),
        ),
    };
    FunctionSignature {
        parameters: vec![parameter],
        returns: vec![],
    }
}

fn resolved_alias(target_type_id: TypeId) -> ResolvedTypeAnnotation {
    ResolvedTypeAnnotation {
        source_ref: ParsedTypeRef::Inferred,
        diagnostic_type: DataType::Inferred,
        type_id: Some(target_type_id),
    }
}

fn constant_declaration(type_id: TypeId, decl_path: InternedPath) -> Declaration {
    Declaration {
        id: decl_path,
        value: Expression::no_value_with_type_id(
            SourceLocation::default(),
            DataType::Int,
            type_id,
            ValueMode::default(),
        ),
    }
}

fn receiver_entry(
    function_path: InternedPath,
    receiver: ReceiverKey,
    signature: FunctionSignature,
    string_table: &mut StringTable,
) -> ReceiverMethodEntry {
    ReceiverMethodEntry {
        function_path,
        receiver,
        source_file: InternedPath::from_single_str("root.moth", string_table),
        receiver_mutable: false,
        signature,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_table(
    headers: Vec<Header>,
    signatures: FxHashMap<InternedPath, ResolvedFunctionSignature>,
    nominal_ids: FxHashMap<InternedPath, TypeId>,
    aliases: FxHashMap<InternedPath, ResolvedTypeAnnotation>,
    declarations: Vec<Declaration>,
    struct_fields: FxHashMap<InternedPath, Vec<Declaration>>,
    receiver_methods: ReceiverMethodCatalog,
    type_environment: &TypeEnvironment,
    trait_environment: &TraitEnvironment,
    string_table: &StringTable,
) -> Result<ResolvedPublicTypeRootTable, CompilerError> {
    let declaration_table = TopLevelDeclarationTable::new(declarations);
    build_resolved_public_type_roots(BuildResolvedPublicTypeRootsInput {
        sorted_headers: &headers,
        resolved_struct_fields_by_path: &struct_fields,
        resolved_function_signatures_by_path: &signatures,
        nominal_type_ids_by_path: &nominal_ids,
        resolved_type_aliases_by_path: &aliases,
        declaration_table: &declaration_table,
        generic_function_templates_by_path: &FxHashMap::default(),
        receiver_methods: &receiver_methods,
        trait_environment,
        type_environment,
        string_table,
        reexport_target_paths: &FxHashSet::default(),
    })
}

#[test]
fn retains_every_public_root_category_in_sorted_header_order() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let func_path = path("free_func", &mut string_table);
    let struct_path = path("public_struct", &mut string_table);
    let choice_path = path("public_choice", &mut string_table);
    let alias_path = path("public_alias", &mut string_table);
    let const_path = path("public_const", &mut string_table);

    // Register real struct and choice definitions so bound-trait collection resolves the
    // nominal TypeIds. The function, alias and constant still use the builtin int TypeId.
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let (_, choice_type_id) = type_environment.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: choice_path.clone(),
        variants: Box::new([]),
        generic_parameters: None,
    });

    // Headers are intentionally in a non-alphabetical order to confirm the table preserves
    // sorted-header input order rather than re-sorting.
    let headers = vec![
        header(
            struct_kind(),
            struct_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            function_kind(),
            func_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            choice_kind(),
            choice_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            alias_kind(),
            alias_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            constant_kind(),
            const_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
    ];

    let mut signatures = FxHashMap::default();
    signatures.insert(func_path.to_owned(), resolved_free_signature(int_type_id));

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(struct_path.to_owned(), struct_type_id);
    nominal_ids.insert(choice_path.to_owned(), choice_type_id);

    let mut aliases = FxHashMap::default();
    aliases.insert(alias_path.to_owned(), resolved_alias(int_type_id));

    let mut struct_fields = FxHashMap::default();
    struct_fields.insert(struct_path.to_owned(), Vec::new());

    let table = build_table(
        headers,
        signatures,
        nominal_ids,
        aliases,
        vec![constant_declaration(int_type_id, const_path.to_owned())],
        struct_fields,
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    )
    .expect("all root categories with resolved facts should be retained");

    assert_eq!(
        table.roots.len(),
        5,
        "all five public root categories must be retained"
    );

    // Order matches sorted-header input: struct, function, choice, alias, constant.
    assert_eq!(table.roots[0].path, struct_path);
    assert!(matches!(
        &table.roots[0].kind,
        ResolvedPublicTypeRootKind::Struct { type_id, .. } if *type_id == struct_type_id
    ));

    assert_eq!(table.roots[1].path, func_path);
    let ResolvedPublicTypeRootKind::Function { signature, .. } = &table.roots[1].kind else {
        panic!("expected Function root");
    };
    assert_eq!(signature.parameters.len(), 1);
    assert_eq!(signature.parameters[0].value.type_id, int_type_id);
    assert_eq!(signature.returns.len(), 1);
    assert_eq!(signature.returns[0].type_id, Some(int_type_id));

    assert_eq!(table.roots[2].path, choice_path);
    assert!(matches!(
        &table.roots[2].kind,
        ResolvedPublicTypeRootKind::Choice { type_id } if *type_id == choice_type_id
    ));

    assert_eq!(table.roots[3].path, alias_path);
    assert!(matches!(
        &table.roots[3].kind,
        ResolvedPublicTypeRootKind::TransparentAlias { target_type_id } if *target_type_id == int_type_id
    ));

    assert_eq!(table.roots[4].path, const_path);
    assert!(matches!(
        &table.roots[4].kind,
        ResolvedPublicTypeRootKind::Constant { type_id } if *type_id == int_type_id
    ));
}

#[test]
fn excludes_imported_root_private_and_non_declaration_headers() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let imported_struct = path("imported_struct", &mut string_table);
    let private_func = path("private_func", &mut string_table);
    let public_func = path("public_func", &mut string_table);
    let start_path = path("start", &mut string_table);

    let headers = vec![
        header(
            struct_kind(),
            imported_struct.to_owned(),
            FileRole::ImportedModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            function_kind(),
            private_func.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Private,
            &mut string_table,
        ),
        header(
            function_kind(),
            public_func.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            HeaderKind::StartFunction,
            start_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
    ];

    let mut signatures = FxHashMap::default();
    signatures.insert(
        private_func.to_owned(),
        resolved_free_signature(int_type_id),
    );
    signatures.insert(public_func.to_owned(), resolved_free_signature(int_type_id));

    let table = build_table(
        headers,
        signatures,
        FxHashMap::default(),
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    )
    .expect("imported/private/start exclusion should not fail");

    assert_eq!(
        table.roots.len(),
        1,
        "only the directly-defined active-root public free function must be retained"
    );
    assert_eq!(table.roots[0].path, public_func);
    assert!(matches!(
        &table.roots[0].kind,
        ResolvedPublicTypeRootKind::Function { .. }
    ));
}

#[test]
fn retains_private_receiver_methods_for_public_nominal_receivers_in_order_independent_pass() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let public_struct = path("public_struct", &mut string_table);
    let private_struct = path("private_struct", &mut string_table);
    let public_method_path = path("public_struct_method", &mut string_table);
    let private_method_path = path("private_struct_method", &mut string_table);

    // Register real struct definitions so bound-trait collection resolves the nominal TypeIds.
    let (_, public_struct_type_id) =
        type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: public_struct.clone(),
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });
    let (_, private_struct_type_id) =
        type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: private_struct.clone(),
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });

    // The private method header for the public receiver precedes the public struct in
    // sorted-header order. This proves the two-pass design collects the complete public
    // nominal set before selecting methods, so order does not affect method admission.
    // Real methods on a public receiver are normally private headers outside `export:`.
    let headers = vec![
        header(
            function_kind(),
            public_method_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Private,
            &mut string_table,
        ),
        header(
            function_kind(),
            private_method_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Private,
            &mut string_table,
        ),
        header(
            struct_kind(),
            public_struct.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            struct_kind(),
            private_struct.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Private,
            &mut string_table,
        ),
    ];

    let public_receiver = ReceiverKey::Struct(public_struct.to_owned());
    let private_receiver = ReceiverKey::Struct(private_struct.to_owned());

    let mut signatures = FxHashMap::default();
    signatures.insert(
        public_method_path.to_owned(),
        ResolvedFunctionSignature {
            receiver: Some(public_receiver.clone()),
            signature: receiver_signature(int_type_id),
        },
    );
    signatures.insert(
        private_method_path.to_owned(),
        ResolvedFunctionSignature {
            receiver: Some(private_receiver.clone()),
            signature: receiver_signature(int_type_id),
        },
    );

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(public_struct.to_owned(), public_struct_type_id);
    nominal_ids.insert(private_struct.to_owned(), private_struct_type_id);

    let mut receiver_catalog = ReceiverMethodCatalog::default();
    let public_entry = receiver_entry(
        public_method_path.to_owned(),
        public_receiver,
        receiver_signature(int_type_id),
        &mut string_table,
    );
    let private_entry = receiver_entry(
        private_method_path.to_owned(),
        private_receiver,
        receiver_signature(int_type_id),
        &mut string_table,
    );
    receiver_catalog
        .by_function_path
        .insert(public_method_path.to_owned(), public_entry.clone());
    receiver_catalog
        .by_function_path
        .insert(private_method_path.to_owned(), private_entry);

    let mut struct_fields = FxHashMap::default();
    struct_fields.insert(public_struct.to_owned(), Vec::new());

    let table = build_table(
        headers,
        signatures,
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        struct_fields,
        receiver_catalog,
        &type_environment,
        &trait_environment,
        &string_table,
    )
    .expect("receiver filtering should not fail");

    // Receiver methods are not free export bindings, and the private struct is not a public
    // root, so the only retained root is the public struct.
    assert_eq!(
        table.roots.len(),
        1,
        "only the public struct is a retained root"
    );
    assert_eq!(table.roots[0].path, public_struct);
    assert_eq!(
        table.receiver_methods.len(),
        1,
        "only the public receiver method is retained"
    );
    assert_eq!(table.receiver_methods[0].function_path, public_method_path);
    assert!(matches!(
        &table.receiver_methods[0].receiver,
        ReceiverKey::Struct(retained) if *retained == public_struct
    ));
}

#[test]
fn missing_alias_type_id_is_internal_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();

    let alias_path = path("unretained_alias", &mut string_table);
    let headers = vec![header(
        alias_kind(),
        alias_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let mut aliases = FxHashMap::default();
    // The public-surface validation owner should have materialized and retained the target
    // TypeId. A None here is an internal invariant failure; the table must not resolve the
    // alias target a second time.
    aliases.insert(
        alias_path.to_owned(),
        ResolvedTypeAnnotation {
            source_ref: ParsedTypeRef::Inferred,
            diagnostic_type: DataType::Int,
            type_id: None,
        },
    );

    let result = build_table(
        headers,
        FxHashMap::default(),
        FxHashMap::default(),
        aliases,
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a public alias without a retained target TypeId must be an internal error"
    );
}

#[test]
fn missing_resolved_function_signature_is_internal_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();

    let func_path = path("missing_sig_func", &mut string_table);
    let headers = vec![header(
        function_kind(),
        func_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let result = build_table(
        headers,
        FxHashMap::default(),
        FxHashMap::default(),
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a public function without a resolved signature must be an internal error"
    );
}

#[test]
fn missing_nominal_type_id_is_internal_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();

    let struct_path = path("missing_nominal_struct", &mut string_table);
    let headers = vec![header(
        struct_kind(),
        struct_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let result = build_table(
        headers,
        FxHashMap::default(),
        FxHashMap::default(),
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a public struct without a canonical TypeId must be an internal error"
    );
}

#[test]
fn missing_resolved_constant_is_internal_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();

    let const_path = path("missing_const", &mut string_table);
    let headers = vec![header(
        constant_kind(),
        const_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let result = build_table(
        headers,
        FxHashMap::default(),
        FxHashMap::default(),
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a public constant without a resolved declaration must be an internal error"
    );
}

#[test]
fn missing_receiver_catalog_entry_is_internal_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let public_struct = path("public_struct", &mut string_table);
    let method_path = path("orphan_method", &mut string_table);

    // The method is a private header, as real receiver methods normally are.
    let headers = vec![
        header(
            struct_kind(),
            public_struct.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            function_kind(),
            method_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Private,
            &mut string_table,
        ),
    ];

    let receiver = ReceiverKey::Struct(public_struct.to_owned());
    let mut signatures = FxHashMap::default();
    signatures.insert(
        method_path.to_owned(),
        ResolvedFunctionSignature {
            receiver: Some(receiver),
            signature: receiver_signature(int_type_id),
        },
    );

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(public_struct.to_owned(), int_type_id);

    // The receiver catalog is empty on purpose: the resolved signature claims a receiver
    // but no catalog entry exists, so the invariant that the catalog owns every receiver
    // method must fire.
    let result = build_table(
        headers,
        signatures,
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a receiver method missing from the catalog must be an internal error"
    );
}

#[test]
fn missing_active_root_function_signature_in_method_pass_is_internal_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let public_struct = path("public_struct", &mut string_table);
    let method_path = path("orphan_method", &mut string_table);
    let imported_func = path("imported_func", &mut string_table);
    let start_path = path("start", &mut string_table);

    // An imported module-root function and an active-root start function are excluded by the
    // method pass via their file role and declaration kind; they must NOT trigger the
    // missing-signature invariant. The active-root private method is a function in the active
    // module root, and AST environment construction resolves every function signature before
    // this table, so its absence from the signature table is an internal error rather than a
    // skip. This distinguishes the missing-signature invariant from normal exclusion.
    let headers = vec![
        header(
            function_kind(),
            imported_func.to_owned(),
            FileRole::ImportedModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            HeaderKind::StartFunction,
            start_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            struct_kind(),
            public_struct.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Public,
            &mut string_table,
        ),
        header(
            function_kind(),
            method_path.to_owned(),
            FileRole::ActiveModuleRoot,
            HeaderExportMode::Private,
            &mut string_table,
        ),
    ];

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(public_struct.to_owned(), int_type_id);

    // The method signature is intentionally absent from the signature table.
    let result = build_table(
        headers,
        FxHashMap::default(),
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "an active-root function missing its resolved signature in the method pass must be an internal error, not skipped"
    );
}

// ---------------------------------------------------------------------------
//  Transient trait source fact retention
// ---------------------------------------------------------------------------

/// Register a generic parameter list with one parameter whose declaration-site
/// `TraitId` bounds are supplied through `resolved_bounds_by_local`, matching how the real
/// AST environment builder registers generic parameter lists.
fn register_param_list_with_bounds(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    param_name: &str,
    bound_trait_ids: Vec<TraitId>,
) -> GenericParameterListId {
    let parameters = vec![GenericParameter {
        id: TypeParameterId(0),
        name: string_table.intern(param_name),
        location: SourceLocation::default(),
        trait_bounds: Vec::new(),
    }];
    let list = GenericParameterList { parameters };
    let mut bounds_by_local: FxHashMap<TypeParameterId, Vec<TraitId>> = FxHashMap::default();
    bounds_by_local.insert(TypeParameterId(0), bound_trait_ids);
    env.register_generic_parameter_list(&list, &bounds_by_local)
        .list_id
}

/// Register a source trait definition with the given canonical path so bound projection can
/// resolve its `TraitId` through the `TraitEnvironment`.
fn register_source_trait(
    trait_environment: &mut TraitEnvironment,
    string_table: &mut StringTable,
    trait_name: &str,
    this_type: TypeId,
) -> TraitId {
    let trait_id = trait_environment.next_trait_id();
    let definition = ResolvedTraitDefinition {
        id: trait_id,
        name: string_table.intern(trait_name),
        canonical_path: InternedPath::from_single_str(trait_name, string_table),
        source_file: InternedPath::new(),
        this_type,
        requirements: Vec::new(),
        declaration_location: SourceLocation::default(),
        visibility: TraitVisibility::Source { exported: true },
    };
    trait_environment.insert(definition);
    trait_id
}

#[test]
fn retains_source_trait_fact_for_generic_struct_bound() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let source_trait_id = register_source_trait(
        &mut trait_environment,
        &mut string_table,
        "RENDERABLE",
        int_type_id,
    );

    let list_id = register_param_list_with_bounds(
        &mut type_environment,
        &mut string_table,
        "T",
        vec![source_trait_id],
    );

    let struct_path = path("public_struct", &mut string_table);
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: Box::new([]),
        generic_parameters: Some(list_id),
        const_record: false,
    });

    let headers = vec![header(
        struct_kind(),
        struct_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(struct_path.to_owned(), struct_type_id);

    let mut struct_fields = FxHashMap::default();
    struct_fields.insert(struct_path.to_owned(), Vec::new());

    let table = build_table(
        headers,
        FxHashMap::default(),
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        struct_fields,
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    )
    .expect("a generic struct with a source trait bound should retain its root and facts");

    let source_path = InternedPath::from_single_str("RENDERABLE", &mut string_table);
    assert_eq!(
        table.trait_source_facts.get(&source_trait_id),
        Some(&ResolvedTraitSourceFact::Source(source_path)),
        "a referenced source trait bound must be retained as a Source fact with its canonical path"
    );
}

#[test]
fn retains_core_trait_fact_for_generic_struct_bound() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let mut trait_environment = TraitEnvironment::new();

    let displayable_trait_id =
        trait_environment.register_core_displayable(&mut type_environment, &mut string_table);

    let list_id = register_param_list_with_bounds(
        &mut type_environment,
        &mut string_table,
        "T",
        vec![displayable_trait_id],
    );

    let struct_path = path("public_struct", &mut string_table);
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: Box::new([]),
        generic_parameters: Some(list_id),
        const_record: false,
    });

    let headers = vec![header(
        struct_kind(),
        struct_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(struct_path.to_owned(), struct_type_id);

    let mut struct_fields = FxHashMap::default();
    struct_fields.insert(struct_path.to_owned(), Vec::new());

    let table = build_table(
        headers,
        FxHashMap::default(),
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        struct_fields,
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    )
    .expect("a generic struct with a core trait bound should retain its root and facts");

    assert_eq!(
        table.trait_source_facts.get(&displayable_trait_id),
        Some(&ResolvedTraitSourceFact::Core(CoreTraitKind::Displayable)),
        "a referenced core trait bound must be retained as a Core fact with its kind"
    );
}

#[test]
fn missing_trait_definition_for_bound_is_compiler_error() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();

    // A TraitId that is neither core nor source: no definition and no classification.
    let unknown_trait_id = TraitId(99);

    let list_id = register_param_list_with_bounds(
        &mut type_environment,
        &mut string_table,
        "T",
        vec![unknown_trait_id],
    );

    let struct_path = path("public_struct", &mut string_table);
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: Box::new([]),
        generic_parameters: Some(list_id),
        const_record: false,
    });

    let headers = vec![header(
        struct_kind(),
        struct_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(struct_path.to_owned(), struct_type_id);

    let result = build_table(
        headers,
        FxHashMap::default(),
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a bound TraitId with no trait definition and no core classification must be a CompilerError"
    );
}

#[test]
fn missing_resolved_struct_fields_is_internal_error() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let trait_environment = TraitEnvironment::new();

    let struct_path = path("public_struct", &mut string_table);

    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let headers = vec![header(
        struct_kind(),
        struct_path.to_owned(),
        FileRole::ActiveModuleRoot,
        HeaderExportMode::Public,
        &mut string_table,
    )];

    let mut nominal_ids = FxHashMap::default();
    nominal_ids.insert(struct_path.to_owned(), struct_type_id);

    // Omit the struct from resolved_struct_fields_by_path so the root-table builder
    // rejects it instead of silently producing a root with no retained declarations.
    let result = build_table(
        headers,
        FxHashMap::default(),
        nominal_ids,
        FxHashMap::default(),
        Vec::new(),
        FxHashMap::default(),
        ReceiverMethodCatalog::default(),
        &type_environment,
        &trait_environment,
        &string_table,
    );

    let CompilerError { msg, .. } = match result {
        Err(error) => error,
        Ok(_) => {
            panic!("a public struct missing resolved field declarations must be a CompilerError")
        }
    };
    assert!(
        msg.contains("no resolved field declarations"),
        "expected a missing-struct-fields diagnostic, got: {msg}"
    );
}
