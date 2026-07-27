//! Focused hidden-invariant tests for validated generic-template body artefact extraction.
//!
//! WHAT: exercises the total extraction/join owner in
//! [`extract_validated_generic_template_artefacts`]: stable-origin retention, private and
//! non-generic exclusion, and missing/duplicate/mismatch failure. These are side-table facts
//! that integration output cannot inspect.
//! WHY: the store is compiler metadata for the future generated sidecar worklist (R5D-R5G), not
//! user-visible behaviour. The invariants are owned by
//! `compiler_frontend::validated_generic_template_metadata`.

use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::canonical_type_identity::{
    ExportedGenericParameterIdentity, GenericDeclarationOrigin,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
use crate::compiler_frontend::public_interface::{
    CallableSeed, CallableSeedKind, PublicGenericParameterSurface,
};
use crate::compiler_frontend::public_interface::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicFunctionCategory,
    PublicFunctionSemantics, PublicGenericTemplateDescriptor, PublicInterfaceDraft,
    PublicReceiverMethodCategory, PublicReceiverMethodSemantics, PublicStructSemantics,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, OriginDeclarationId, OriginFunctionId, OriginTypeCategory, OriginTypeId,
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::validated_generic_template_metadata::extract_validated_generic_template_artefacts;

use rustc_hash::FxHashMap;

fn module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    )
}

fn free_function_origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module_origin(), name.to_owned())
}

fn function_record(name: &str, is_generic: bool) -> PublicDeclarationRecord {
    let category = if is_generic {
        PublicFunctionCategory::GenericTemplate(PublicGenericTemplateDescriptor {
            generic_parameters: vec![PublicGenericParameterSurface {
                identity: ExportedGenericParameterIdentity::new(
                    GenericDeclarationOrigin::free_function(free_function_origin(name))
                        .expect("free function is a valid generic owner"),
                    0,
                    "T".to_owned(),
                ),
                bounds: vec![],
            }],
        })
    } else {
        PublicFunctionCategory::ConcreteLocal
    };

    PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(free_function_origin(name)),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category,
            parameters: vec![],
            returns: vec![],
            error_return: None,
        }),
    }
}

fn empty_draft(records: Vec<PublicDeclarationRecord>) -> PublicInterfaceDraft {
    PublicInterfaceDraft {
        module_origin: module_origin(),
        export_bindings: vec![],
        declarations: records,
        reusable_evidence: vec![],
    }
}

fn template_for(
    name: &str,
    string_table: &mut StringTable,
) -> (InternedPath, GenericFunctionTemplate) {
    let path = InternedPath::from_single_str(name, string_table);
    let template = GenericFunctionTemplate {
        function_path: path.to_owned(),
        source_file: InternedPath::new(),
        generic_parameter_list_id: GenericParameterListId(0),
        signature: FunctionSignature::default(),
        body_tokens: FileTokens::new(path.to_owned(), vec![]),
        declaration_location: SourceLocation::default(),
    };
    (path, template)
}

fn templates_map(
    names: &[&str],
    string_table: &mut StringTable,
) -> FxHashMap<InternedPath, GenericFunctionTemplate> {
    let mut map = FxHashMap::default();
    for name in names {
        let (path, template) = template_for(name, string_table);
        map.insert(path, template);
    }
    map
}

fn callable_seed(
    name: &str,
    generic_template: bool,
    string_table: &mut StringTable,
) -> CallableSeed {
    CallableSeed {
        path: InternedPath::from_single_str(name, string_table),
        origin: free_function_origin(name),
        generic_template,
        kind: CallableSeedKind::FreeFunction,
    }
}

fn receiver_origin(receiver_name: &str, method_name: &str) -> OriginFunctionId {
    OriginFunctionId::new_receiver(
        module_origin(),
        method_name.to_owned(),
        receiver_type_origin(receiver_name),
    )
}

fn receiver_type_origin(receiver_name: &str) -> OriginTypeId {
    OriginTypeId::new(
        module_origin(),
        receiver_name.to_owned(),
        OriginTypeCategory::Struct,
    )
}

fn receiver_record(
    receiver_name: &str,
    method_origin: OriginFunctionId,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(OriginTypeId::new(
            module_origin(),
            receiver_name.to_owned(),
            OriginTypeCategory::Struct,
        )),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: vec![],
            fields: vec![],
            receiver_methods: vec![PublicReceiverMethodSemantics {
                method_origin,
                category: PublicReceiverMethodCategory::GenericTemplate,
                parameters: vec![],
                returns: vec![],
                error_return: None,
            }],
        }),
    }
}

fn template_at_path(path: InternedPath) -> (InternedPath, GenericFunctionTemplate) {
    let template = GenericFunctionTemplate {
        function_path: path.to_owned(),
        source_file: InternedPath::new(),
        generic_parameter_list_id: GenericParameterListId(0),
        signature: FunctionSignature::default(),
        body_tokens: FileTokens::new(path.to_owned(), vec![]),
        declaration_location: SourceLocation::default(),
    };
    (path, template)
}

/// Build one map entry whose key path differs from the template's own `function_path`.
fn mismatched_template(
    key_name: &str,
    function_path_name: &str,
    string_table: &mut StringTable,
) -> (InternedPath, GenericFunctionTemplate) {
    let key = InternedPath::from_single_str(key_name, string_table);
    let function_path = InternedPath::from_single_str(function_path_name, string_table);
    let template = GenericFunctionTemplate {
        function_path: function_path.to_owned(),
        source_file: InternedPath::new(),
        generic_parameter_list_id: GenericParameterListId(0),
        signature: FunctionSignature::default(),
        body_tokens: FileTokens::new(function_path.to_owned(), vec![]),
        declaration_location: SourceLocation::default(),
    };
    (key, template)
}

#[test]
fn exported_generic_free_function_retains_one_artefact_keyed_by_origin() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("identity", true)]);
    let templates = templates_map(&["identity"], &mut string_table);

    let seed = callable_seed("identity", true, &mut string_table);
    let store = extract_validated_generic_template_artefacts(&draft, &[seed], templates)
        .expect("one exported generic function with a matching template extracts one artefact");

    assert_eq!(store.len(), 1);
    let artefact = &store.artefacts()[0];
    assert_eq!(
        artefact.origin,
        free_function_origin("identity"),
        "the artefact is keyed by the exact OriginFunctionId"
    );
    assert_eq!(
        artefact.template.function_path,
        InternedPath::from_single_str("identity", &mut StringTable::new()),
        "the artefact retains the one existing template body payload"
    );
}

#[test]
fn same_named_generic_receiver_methods_retain_distinct_exact_origins() {
    let mut string_table = StringTable::new();
    let first_method = receiver_origin("First", "map");
    let second_method = receiver_origin("Second", "map");
    let first_receiver = receiver_type_origin("First");
    let second_receiver = receiver_type_origin("Second");
    let draft = empty_draft(vec![
        receiver_record("First", first_method.clone()),
        receiver_record("Second", second_method.clone()),
    ]);

    let first_path = InternedPath::from_single_str("First", &mut string_table)
        .join_str("map", &mut string_table);
    let second_path = InternedPath::from_single_str("Second", &mut string_table)
        .join_str("map", &mut string_table);
    let (first_path, first_template) = template_at_path(first_path);
    let (second_path, second_template) = template_at_path(second_path);
    let mut templates = FxHashMap::default();
    templates.insert(first_path.clone(), first_template);
    templates.insert(second_path.clone(), second_template);
    let seeds = [
        CallableSeed {
            path: first_path,
            origin: first_method.clone(),
            generic_template: true,
            kind: CallableSeedKind::ReceiverMethod {
                receiver_origin: first_receiver,
                method_index: 0,
            },
        },
        CallableSeed {
            path: second_path,
            origin: second_method.clone(),
            generic_template: true,
            kind: CallableSeedKind::ReceiverMethod {
                receiver_origin: second_receiver,
                method_index: 1,
            },
        },
    ];

    let store = extract_validated_generic_template_artefacts(&draft, &seeds, templates)
        .expect("same-named receiver methods on distinct receivers join by exact path");

    assert_eq!(store.len(), 2);
    let origins: Vec<OriginFunctionId> = store
        .artefacts()
        .iter()
        .map(|artefact| artefact.origin.clone())
        .collect();
    let mut expected = vec![first_method, second_method];
    expected.sort();
    assert_eq!(origins, expected);
}

#[test]
fn non_generic_exported_function_produces_no_artefact() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("render", false)]);
    let templates = templates_map(&[], &mut string_table);

    let seed = callable_seed("render", false, &mut string_table);
    let store = extract_validated_generic_template_artefacts(&draft, &[seed], templates)
        .expect("a non-generic exported function with no templates produces an empty store");

    assert!(
        store.is_empty(),
        "a non-generic exported function must produce no artefact"
    );
}

#[test]
fn private_generic_function_is_intentionally_excluded() {
    let mut string_table = StringTable::new();
    // The draft exports no callables. The template map contains one private generic function
    // whose exact declaration path does not match any exported callable seed.
    let draft = empty_draft(vec![]);
    let templates = templates_map(&["private_helper"], &mut string_table);

    let store = extract_validated_generic_template_artefacts(&draft, &[], templates)
        .expect("a private generic template is an intentional exclusion, not an error");

    assert!(
        store.is_empty(),
        "a private generic function must not produce an artefact"
    );
}

#[test]
fn missing_template_for_exported_generic_function_is_compiler_error() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("identity", true)]);
    let templates = templates_map(&[], &mut string_table);

    let seed = callable_seed("identity", true, &mut string_table);
    let result = extract_validated_generic_template_artefacts(&draft, &[seed], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "an exported generic function with no matching template must fail as CompilerError"
    );
}

#[test]
fn missing_public_callable_seed_is_compiler_error() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("identity", true)]);
    let templates = templates_map(&["identity"], &mut string_table);

    let result = extract_validated_generic_template_artefacts(&draft, &[], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "an exported callable without its exact transient seed must fail as CompilerError"
    );
}

#[test]
fn public_callable_seed_generic_flag_mismatch_is_compiler_error() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("identity", true)]);
    let templates = templates_map(&[], &mut string_table);
    let seed = callable_seed("identity", false, &mut string_table);

    let result = extract_validated_generic_template_artefacts(&draft, &[seed], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a callable seed whose generic flag disagrees with the draft must fail as CompilerError"
    );
}

#[test]
fn generic_non_generic_mismatch_is_compiler_error() {
    let mut string_table = StringTable::new();
    // The draft marks "identity" as non-generic, but a generic template exists for it.
    let draft = empty_draft(vec![function_record("identity", false)]);
    let templates = templates_map(&["identity"], &mut string_table);

    let seed = callable_seed("identity", false, &mut string_table);
    let result = extract_validated_generic_template_artefacts(&draft, &[seed], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a template for an exported non-generic function is a mismatch CompilerError"
    );
}

#[test]
fn multiple_exported_generic_functions_retain_one_artefact_each_in_deterministic_order() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![
        function_record("zebra", true),
        function_record("alpha", true),
        function_record("mid", true),
    ]);
    // Insert in non-sorted order to prove the store sorts by full stable origin.
    let templates = templates_map(&["mid", "zebra", "alpha"], &mut string_table);
    let seeds = [
        callable_seed("zebra", true, &mut string_table),
        callable_seed("alpha", true, &mut string_table),
        callable_seed("mid", true, &mut string_table),
    ];

    let store = extract_validated_generic_template_artefacts(&draft, &seeds, templates)
        .expect("three exported generic functions with matching templates extract three artefacts");

    assert_eq!(store.len(), 3);
    let origins: Vec<OriginFunctionId> = store
        .artefacts()
        .iter()
        .map(|artefact| artefact.origin.clone())
        .collect();
    let mut expected = vec![
        free_function_origin("zebra"),
        free_function_origin("alpha"),
        free_function_origin("mid"),
    ];
    expected.sort();
    assert_eq!(
        origins, expected,
        "artefacts are sorted by full stable origin for deterministic iteration"
    );
}

#[test]
fn mixed_exported_and_private_generic_functions_retain_only_exported() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("public_generic", true)]);
    let templates = templates_map(&["public_generic", "private_generic"], &mut string_table);
    let seed = callable_seed("public_generic", true, &mut string_table);

    let store = extract_validated_generic_template_artefacts(&draft, &[seed], templates)
        .expect("a private generic template alongside an exported one is an intentional exclusion");

    assert_eq!(store.len(), 1);
    assert_eq!(
        store.artefacts()[0].origin,
        free_function_origin("public_generic"),
        "only the exported generic function retains an artefact"
    );
}

#[test]
fn duplicate_draft_origin_is_compiler_error() {
    let mut string_table = StringTable::new();
    // Two draft records carry the same exported generic free-function origin.
    let draft = empty_draft(vec![
        function_record("identity", true),
        function_record("identity", true),
    ]);
    let templates = templates_map(&["identity"], &mut string_table);

    let seed = callable_seed("identity", true, &mut string_table);
    let result = extract_validated_generic_template_artefacts(&draft, &[seed], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a duplicate generic free-function origin in the draft must fail as CompilerError"
    );
}

#[test]
fn duplicate_public_callable_seed_path_is_compiler_error() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("identity", true)]);
    let templates = templates_map(&["identity"], &mut string_table);
    let seed = callable_seed("identity", true, &mut string_table);

    let result =
        extract_validated_generic_template_artefacts(&draft, &[seed.clone(), seed], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a duplicate exact public callable path must fail as CompilerError"
    );
}

#[test]
fn duplicate_public_callable_seed_origin_is_compiler_error() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![
        function_record("identity", true),
        function_record("render", true),
    ]);
    let identity_seed = callable_seed("identity", true, &mut string_table);
    let duplicate_origin_seed = CallableSeed {
        path: InternedPath::from_single_str("render", &mut string_table),
        origin: identity_seed.origin.clone(),
        generic_template: true,
        kind: CallableSeedKind::FreeFunction,
    };

    let result = extract_validated_generic_template_artefacts(
        &draft,
        &[identity_seed, duplicate_origin_seed],
        FxHashMap::default(),
    );

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "duplicate stable public callable origins must fail as CompilerError"
    );
}

#[test]
fn map_key_template_function_path_mismatch_is_compiler_error() {
    let mut string_table = StringTable::new();
    let draft = empty_draft(vec![function_record("identity", true)]);
    // The map key is "identity" but the template's own function_path is "mismatch".
    let mut templates = FxHashMap::default();
    let (key, template) = mismatched_template("identity", "mismatch", &mut string_table);
    templates.insert(key, template);

    let seed = callable_seed("identity", true, &mut string_table);
    let result = extract_validated_generic_template_artefacts(&draft, &[seed], templates);

    assert!(
        matches!(result, Err(CompilerError { .. })),
        "a map key that does not equal the template's own function_path must fail as CompilerError"
    );
}
