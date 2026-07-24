//! String-ID remapping tests for flat header metadata types.
//!
//! WHAT: verifies that `TopLevelConstFragment` and `FileImport`
//!      can be remapped from local string tables into a merged global table.
//! WHY: per-file frontend preparation produces these flat metadata structures using local
//!      string tables; remapping must preserve all paths, names, aliases, and source locations.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, NameNamespace, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::choice::{
    ChoiceVariantPayloadSyntax, ChoiceVariantSyntax,
};
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, InitializerReference,
};
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionSignatureSyntax, SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareError, FileFrontendPrepareOutput, FileImport, FileRole, Header,
    HeaderExportMode, HeaderKind, LocalDeclarationOrderingHint, TopLevelConstFragment,
};
use crate::compiler_frontend::paths::const_paths::StructuralProviderReference;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;
use std::collections::HashSet;

fn make_location(path_name: &str, string_table: &mut StringTable) -> SourceLocation {
    let path = InternedPath::from_single_str(path_name, string_table);
    SourceLocation::new(path, CharPosition::default(), CharPosition::default())
}

fn assert_location_resolves_to(
    location: &SourceLocation,
    expected: &str,
    string_table: &StringTable,
) {
    let scope_components = location
        .scope
        .as_components()
        .iter()
        .map(|id| string_table.resolve(*id))
        .collect::<Vec<_>>();

    assert_eq!(scope_components, vec![expected]);
}

fn make_signature_member(name: &str, string_table: &mut StringTable) -> SignatureMemberSyntax {
    let location = make_location("test.moth", string_table);

    SignatureMemberSyntax {
        id: InternedPath::from_single_str(name, string_table),
        value_mode: ValueMode::ImmutableOwned,
        is_reactive: false,
        type_annotation: ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        },
        default_tokens: vec![],
        location,
    }
}

fn make_generic_parameter_list(name: &str, string_table: &mut StringTable) -> GenericParameterList {
    GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: string_table.intern(name),
            location: make_location("test.moth", string_table),
            trait_bounds: Vec::new(),
        }],
    }
}

fn make_file_tokens(symbol_name: &str, string_table: &mut StringTable) -> FileTokens {
    let src_path = InternedPath::from_single_str("test.moth", string_table);
    let token = Token::new(
        TokenKind::Symbol(string_table.intern(symbol_name)),
        make_location("test.moth", string_table),
    );
    FileTokens::new(src_path, vec![token])
}

#[test]
fn top_level_const_fragment_remaps_path_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let header_path = InternedPath::from_single_str("src/#page.moth", &mut local);
    let location = make_location("src/#page.moth", &mut local);

    let mut fragment = TopLevelConstFragment {
        runtime_insertion_index: 3,
        header_path,
        location,
    };

    let remap = global.merge_from(&local);
    fragment.remap_string_ids(&remap);

    assert_eq!(fragment.runtime_insertion_index, 3);
    assert_eq!(
        fragment.header_path.to_portable_string(&global),
        "src/#page.moth"
    );
    assert_location_resolves_to(&fragment.location, "src/#page.moth", &global);
}

#[test]
fn file_import_remaps_all_fields_without_alias() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let header_path = InternedPath::from_single_str("@html/head", &mut local);
    let location = make_location("test.moth", &mut local);
    let path_location = make_location("test.moth", &mut local);

    let mut import = FileImport {
        provider: StructuralProviderReference {
            path: header_path,
            path_location,
        },
        alias: None,
        location,
        alias_location: None,
        from_grouped: false,
        export_mode: HeaderExportMode::Private,
    };

    let remap = global.merge_from(&local);
    import.remap_string_ids(&remap);

    assert_eq!(
        import.provider.path.to_portable_string(&global),
        "@html/head"
    );
    assert!(import.alias.is_none());
    assert_location_resolves_to(&import.location, "test.moth", &global);
    assert_location_resolves_to(&import.provider.path_location, "test.moth", &global);
    assert!(import.alias_location.is_none());
    assert_eq!(import.export_mode, HeaderExportMode::Private);
}

#[test]
fn file_import_remaps_all_fields_with_alias() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let alias_name = local.intern("h");
    let header_path = InternedPath::from_single_str("@html/head", &mut local);
    let location = make_location("test.moth", &mut local);
    let path_location = make_location("test.moth", &mut local);
    let alias_location = Some(make_location("test.moth", &mut local));

    let mut import = FileImport {
        provider: StructuralProviderReference {
            path: header_path,
            path_location,
        },
        alias: Some(alias_name),
        location,
        alias_location,
        from_grouped: false,
        export_mode: HeaderExportMode::Public,
    };

    let remap = global.merge_from(&local);
    import.remap_string_ids(&remap);

    assert_eq!(
        import.provider.path.to_portable_string(&global),
        "@html/head"
    );
    assert_eq!(global.resolve(import.alias.unwrap()), "h");
    assert_location_resolves_to(&import.location, "test.moth", &global);
    assert_location_resolves_to(&import.provider.path_location, "test.moth", &global);
    assert_location_resolves_to(&import.alias_location.unwrap(), "test.moth", &global);
    assert_eq!(import.export_mode, HeaderExportMode::Public);
}

#[test]
fn remap_preserves_correct_ids_when_global_has_preexisting_strings() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_a");
    global.intern("preexisting_b");

    let alias_name = local.intern("my_alias");
    let header_path = InternedPath::from_single_str("@utils/helpers", &mut local);
    let location = make_location("file.moth", &mut local);
    let path_location = make_location("file.moth", &mut local);
    let alias_location = Some(make_location("file.moth", &mut local));

    let mut import = FileImport {
        provider: StructuralProviderReference {
            path: header_path,
            path_location,
        },
        alias: Some(alias_name),
        location,
        alias_location,
        from_grouped: false,
        export_mode: HeaderExportMode::Public,
    };

    let mut fragment = TopLevelConstFragment {
        runtime_insertion_index: 7,
        header_path: InternedPath::from_single_str("file.moth", &mut local),
        location: make_location("file.moth", &mut local),
    };

    let remap = global.merge_from(&local);
    import.remap_string_ids(&remap);
    fragment.remap_string_ids(&remap);

    // Verify the alias resolves to the correct string in the global table.
    assert_eq!(global.resolve(import.alias.unwrap()), "my_alias");

    // Verify the path resolves correctly.
    assert_eq!(
        import.provider.path.to_portable_string(&global),
        "@utils/helpers"
    );

    // Verify fragment path resolves correctly.
    assert_eq!(
        fragment.header_path.to_portable_string(&global),
        "file.moth"
    );

    // Verify all locations still resolve.
    assert_location_resolves_to(&import.location, "file.moth", &global);
    assert_location_resolves_to(&import.provider.path_location, "file.moth", &global);
    assert_location_resolves_to(&import.alias_location.unwrap(), "file.moth", &global);
    assert_location_resolves_to(&fragment.location, "file.moth", &global);
}

// -----------------------------------------------------------
//  HeaderKind remapping tests
// -----------------------------------------------------------

#[test]
fn header_kind_function_remaps_generic_parameters_and_signature() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let signature = FunctionSignatureSyntax::default();

    let mut kind = HeaderKind::Function {
        generic_parameters,
        signature,
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Function {
        generic_parameters, ..
    } = kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_location_resolves_to(
        &generic_parameters.parameters[0].location,
        "test.moth",
        &global,
    );
}

#[test]
fn header_kind_constant_remaps_declaration() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let type_name = local.intern("MyType");
    let declaration = DeclarationSyntax {
        binding_mode: BindingMode::ImmutableRuntime,
        type_annotation: ParsedTypeRef::Named {
            name: type_name,
            location: make_location("test.moth", &mut local),
        },
        initializer_tokens: vec![],
        initializer_references: vec![],
        location: make_location("test.moth", &mut local),
    };

    let mut kind = HeaderKind::Constant { declaration };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Constant { declaration, .. } = kind else {
        panic!("expected Constant kind");
    };

    let ParsedTypeRef::Named { name, location } = &declaration.type_annotation else {
        panic!("expected Named type annotation");
    };
    assert_eq!(global.resolve(*name), "MyType");
    assert_location_resolves_to(location, "test.moth", &global);
    assert_location_resolves_to(&declaration.location, "test.moth", &global);
}

#[test]
fn header_kind_struct_remaps_generic_parameters_and_fields() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let field = make_signature_member("field", &mut local);

    let mut kind = HeaderKind::Struct {
        generic_parameters,
        fields: vec![field],
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Struct {
        generic_parameters,
        fields,
    } = kind
    else {
        panic!("expected Struct kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_location_resolves_to(&fields[0].location, "test.moth", &global);
}

#[test]
fn header_kind_choice_remaps_generic_parameters_and_variants() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let variant = ChoiceVariantSyntax {
        id: local.intern("SomeVariant"),
        payload: ChoiceVariantPayloadSyntax::Unit,
        location: make_location("test.moth", &mut local),
    };

    let mut kind = HeaderKind::Choice {
        generic_parameters,
        variants: vec![variant],
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Choice {
        generic_parameters,
        variants,
    } = kind
    else {
        panic!("expected Choice kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_eq!(global.resolve(variants[0].id), "SomeVariant");
    assert_location_resolves_to(&variants[0].location, "test.moth", &global);
}

#[test]
fn header_kind_type_alias_remaps_target() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let target = ParsedTypeRef::Named {
        name: local.intern("TargetType"),
        location: make_location("test.moth", &mut local),
    };

    let mut kind = HeaderKind::TypeAlias { target };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::TypeAlias { target } = kind else {
        panic!("expected TypeAlias kind");
    };

    let ParsedTypeRef::Named { name, location } = target else {
        panic!("expected Named target");
    };
    assert_eq!(global.resolve(name), "TargetType");
    assert_location_resolves_to(&location, "test.moth", &global);
}

#[test]
fn header_kind_const_template_remaps_condition_references() {
    let mut global = StringTable::new();
    let mut local = StringTable::new();
    let show_banner = local.intern("show_banner");

    let mut kind = HeaderKind::ConstTemplate {
        condition_references: vec![InitializerReference {
            name: show_banner,
            dot_member: None,
            location: make_location("test.moth", &mut local),
            followed_by_call: false,
            followed_by_choice_namespace: false,
        }],
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::ConstTemplate {
        condition_references,
        ..
    } = kind
    else {
        panic!("expected ConstTemplate kind");
    };
    assert_eq!(global.resolve(condition_references[0].name), "show_banner");
    assert_location_resolves_to(&condition_references[0].location, "test.moth", &global);
}

#[test]
fn header_kind_start_function_is_no_op() {
    let mut kind = HeaderKind::StartFunction;
    let identity_remap = {
        let mut global = StringTable::new();
        global.merge_from(&StringTable::new())
    };
    kind.remap_string_ids(&identity_remap);
    // No panic and no fields to assert.
}

// -----------------------------------------------------------
//  Header container remapping tests
// -----------------------------------------------------------

#[test]
fn header_remaps_kind_dependencies_locations_tokens_source_file_and_imports() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let mut dependencies = HashSet::new();
    dependencies.insert(LocalDeclarationOrderingHint::new(
        InternedPath::from_single_str("@core/prelude", &mut local),
    ));

    let mut header = Header {
        kind: HeaderKind::Function {
            generic_parameters,
            signature: FunctionSignatureSyntax::default(),
        },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: dependencies,
        name_location: make_location("test.moth", &mut local),
        tokens: make_file_tokens("my_symbol", &mut local),
        source_file: InternedPath::from_single_str("test.moth", &mut local),
        capacity_references: Vec::new(),
    };

    let remap = global.merge_from(&local);
    header.remap_string_ids(&remap);

    // Verify kind remapped.
    let HeaderKind::Function {
        generic_parameters, ..
    } = &header.kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");

    // Verify dependencies remapped.
    assert_eq!(header.local_ordering_hints.len(), 1);
    let dep = header.local_ordering_hints.iter().next().unwrap();
    assert_eq!(dep.path().to_portable_string(&global), "@core/prelude");

    // Verify name location remapped.
    assert_location_resolves_to(&header.name_location, "test.moth", &global);

    // Verify tokens remapped.
    assert_eq!(
        header.tokens.src_path.to_portable_string(&global),
        "test.moth"
    );
    let token_kind = &header.tokens.tokens[0].kind;
    let TokenKind::Symbol(symbol_id) = token_kind else {
        panic!("expected Symbol token");
    };
    assert_eq!(global.resolve(*symbol_id), "my_symbol");

    // Verify source file remapped.
    assert_eq!(header.source_file.to_portable_string(&global), "test.moth");
}

#[test]
fn header_remap_preserves_correct_ids_when_global_has_preexisting_strings() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_a");
    global.intern("preexisting_b");

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let mut dependencies = HashSet::new();
    dependencies.insert(LocalDeclarationOrderingHint::new(
        InternedPath::from_single_str("@core/prelude", &mut local),
    ));

    let mut header = Header {
        kind: HeaderKind::Function {
            generic_parameters,
            signature: FunctionSignatureSyntax::default(),
        },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Public,
        local_ordering_hints: dependencies,
        name_location: make_location("test.moth", &mut local),
        tokens: make_file_tokens("my_symbol", &mut local),
        source_file: InternedPath::from_single_str("test.moth", &mut local),
        capacity_references: Vec::new(),
    };

    let remap = global.merge_from(&local);
    header.remap_string_ids(&remap);

    // Verify generic parameter name resolves correctly after non-identity merge.
    let HeaderKind::Function {
        generic_parameters, ..
    } = &header.kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");

    // Verify dependency resolves correctly.
    assert_eq!(
        header
            .local_ordering_hints
            .iter()
            .next()
            .unwrap()
            .path()
            .to_portable_string(&global),
        "@core/prelude"
    );

    // Verify token symbol resolves correctly.
    let TokenKind::Symbol(symbol_id) = &header.tokens.tokens[0].kind else {
        panic!("expected Symbol token");
    };
    assert_eq!(global.resolve(*symbol_id), "my_symbol");
}

// -----------------------------------------------------------
//  FileFrontendPrepareOutput remapping tests
// -----------------------------------------------------------

fn make_unknown_name_diagnostic(name: &str, string_table: &mut StringTable) -> CompilerDiagnostic {
    let name_id = string_table.intern(name);
    let location = make_location("test.moth", string_table);
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        location,
        DiagnosticPayload::UnknownName {
            name: name_id,
            namespace: NameNamespace::Value,
        },
    )
}

#[test]
fn file_frontend_prepare_output_remaps_all_string_id_fields() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_a");
    global.intern("preexisting_b");

    let source_file = InternedPath::from_single_str("src/main.moth", &mut local);

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let mut dependencies = HashSet::new();
    dependencies.insert(LocalDeclarationOrderingHint::new(
        InternedPath::from_single_str("@core/prelude", &mut local),
    ));

    let header = Header {
        kind: HeaderKind::Function {
            generic_parameters,
            signature: FunctionSignatureSyntax::default(),
        },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: dependencies,
        name_location: make_location("test.moth", &mut local),
        tokens: make_file_tokens("my_func", &mut local),
        source_file: InternedPath::from_single_str("test.moth", &mut local),
        capacity_references: Vec::new(),
    };

    let fragment = TopLevelConstFragment {
        runtime_insertion_index: 2,
        header_path: InternedPath::from_single_str("src/#page.moth", &mut local),
        location: make_location("src/#page.moth", &mut local),
    };

    let warning = make_unknown_name_diagnostic("warn_name", &mut local);

    let import = FileImport {
        provider: StructuralProviderReference {
            path: InternedPath::from_single_str("@html/head", &mut local),
            path_location: make_location("test.moth", &mut local),
        },
        alias: Some(local.intern("h")),
        location: make_location("test.moth", &mut local),
        alias_location: Some(make_location("test.moth", &mut local)),
        from_grouped: false,
        export_mode: HeaderExportMode::Public,
    };

    let mut output = FileFrontendPrepareOutput {
        source_file,
        file_id: None,
        token_count: 12,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_imports: vec![import],
        canonical_os_path: None,
        headers: vec![header],
        top_level_const_fragments: vec![fragment],
        const_template_count: 5,
        runtime_fragment_count: 3,
        has_non_trivial_root_body: false,
        warnings: vec![warning],
    };

    let remap = global.merge_from(&local);
    output.remap_string_ids(&remap);

    // source_file remapped.
    assert_eq!(
        output.source_file.to_portable_string(&global),
        "src/main.moth"
    );

    // file_id unchanged.
    assert!(output.file_id.is_none());

    // Header nested fields remapped.
    assert_eq!(output.headers.len(), 1);
    let header = &output.headers[0];
    let HeaderKind::Function {
        generic_parameters, ..
    } = &header.kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_eq!(
        header
            .local_ordering_hints
            .iter()
            .next()
            .unwrap()
            .path()
            .to_portable_string(&global),
        "@core/prelude"
    );
    assert_location_resolves_to(&header.name_location, "test.moth", &global);
    let TokenKind::Symbol(symbol_id) = &header.tokens.tokens[0].kind else {
        panic!("expected Symbol token");
    };
    assert_eq!(global.resolve(*symbol_id), "my_func");
    assert_eq!(header.source_file.to_portable_string(&global), "test.moth");

    // Per-file imports remapped.
    assert_eq!(output.file_imports.len(), 1);
    let import = &output.file_imports[0];
    assert_eq!(
        import.provider.path.to_portable_string(&global),
        "@html/head"
    );
    assert_eq!(global.resolve(import.alias.unwrap()), "h");
    assert_eq!(import.export_mode, HeaderExportMode::Public);

    // Const fragment remapped.
    assert_eq!(output.top_level_const_fragments.len(), 1);
    let fragment = &output.top_level_const_fragments[0];
    assert_eq!(fragment.runtime_insertion_index, 2);
    assert_eq!(
        fragment.header_path.to_portable_string(&global),
        "src/#page.moth"
    );
    assert_location_resolves_to(&fragment.location, "src/#page.moth", &global);

    // Counters unchanged.
    assert_eq!(output.const_template_count, 5);
    assert_eq!(output.runtime_fragment_count, 3);

    // Warnings remapped.
    assert_eq!(output.warnings.len(), 1);
    let warning = &output.warnings[0];
    let DiagnosticPayload::UnknownName { name, .. } = &warning.payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name), "warn_name");
    assert_location_resolves_to(&warning.primary_location, "test.moth", &global);
}

#[test]
fn file_frontend_prepare_output_identity_remap_preserves_payload() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let source_file = InternedPath::from_single_str("src/main.moth", &mut local);
    let warning = make_unknown_name_diagnostic("warn_name", &mut local);

    let mut output = FileFrontendPrepareOutput {
        source_file,
        file_id: None,
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_imports: Vec::new(),
        canonical_os_path: None,
        headers: Vec::new(),
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: vec![warning],
    };

    let remap = global.merge_from(&local);
    assert!(remap.is_identity());

    output.remap_string_ids(&remap);

    assert_eq!(
        output.source_file.to_portable_string(&global),
        "src/main.moth"
    );

    let DiagnosticPayload::UnknownName { name, .. } = &output.warnings[0].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name), "warn_name");
    assert_location_resolves_to(&output.warnings[0].primary_location, "test.moth", &global);
}

// -----------------------------------------------------------
//  FileFrontendPrepareError remapping tests
// -----------------------------------------------------------

#[test]
fn file_frontend_prepare_error_remaps_warnings_and_diagnostic() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_x");
    global.intern("preexisting_y");

    let warning_a = make_unknown_name_diagnostic("warn_a", &mut local);
    let warning_b = make_unknown_name_diagnostic("warn_b", &mut local);
    let diagnostic = make_unknown_name_diagnostic("error_name", &mut local);

    let mut error = FileFrontendPrepareError {
        warnings: vec![warning_a, warning_b],
        diagnostic: Box::new(diagnostic),
    };

    let remap = global.merge_from(&local);
    error.remap_string_ids(&remap);

    // Warnings remapped.
    assert_eq!(error.warnings.len(), 2);

    let DiagnosticPayload::UnknownName { name: name_a, .. } = &error.warnings[0].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name_a), "warn_a");
    assert_location_resolves_to(&error.warnings[0].primary_location, "test.moth", &global);

    let DiagnosticPayload::UnknownName { name: name_b, .. } = &error.warnings[1].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name_b), "warn_b");
    assert_location_resolves_to(&error.warnings[1].primary_location, "test.moth", &global);

    // Primary diagnostic remapped.
    let DiagnosticPayload::UnknownName {
        name: error_name, ..
    } = &error.diagnostic.payload
    else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*error_name), "error_name");
    assert_location_resolves_to(&error.diagnostic.primary_location, "test.moth", &global);
}

#[test]
fn file_frontend_prepare_error_identity_remap_preserves_payload() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let warning = make_unknown_name_diagnostic("warn_name", &mut local);
    let diagnostic = make_unknown_name_diagnostic("error_name", &mut local);

    let mut error = FileFrontendPrepareError {
        warnings: vec![warning],
        diagnostic: Box::new(diagnostic),
    };

    let remap = global.merge_from(&local);
    assert!(remap.is_identity());

    error.remap_string_ids(&remap);

    let DiagnosticPayload::UnknownName { name, .. } = &error.warnings[0].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name), "warn_name");
    assert_location_resolves_to(&error.warnings[0].primary_location, "test.moth", &global);

    let DiagnosticPayload::UnknownName {
        name: error_name, ..
    } = &error.diagnostic.payload
    else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*error_name), "error_name");
    assert_location_resolves_to(&error.diagnostic.primary_location, "test.moth", &global);
}
