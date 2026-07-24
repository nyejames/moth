//! Focused hidden-invariant tests for the directly-defined public export identity component.
//!
//! WHAT: exercises the invariants of [`DefinedPublicExportOrigins`] that integration output
//!      cannot inspect: direct export bindings cover exactly the public declarations authored in
//!      the active module root, category distinctions are exact, receiver methods attach to their
//!      receiver surface rather than free namespace bindings, generic receiver methods attach to
//!      their generic nominal base origin, private declarations and the implicit start function are
//!      excluded, and ordering is deterministic independent of declaration scheduling.
//! WHY: these are construction invariants owned by `compiler_frontend::defined_public_export_origins`,
//!      so they own a focused test beside the module rather than an end-to-end case.

use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::{ReceiverMethodCatalog, ReceiverMethodEntry};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::defined_public_export_origins::{
    build_defined_public_export_origin_draft, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
use crate::compiler_frontend::headers::module_symbols::{
    ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::parse_single_file_headers_with_table;
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::prepare_single_file;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, FileRole, Header, HeaderKind,
};
use crate::compiler_frontend::semantic_identity::{
    DefinedPublicExportOrigins, ExportBinding, FunctionOriginKind, ModuleRootRole,
    OriginDeclarationId, OriginTraitId, OriginTypeCategory, OriginTypeId,
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::path::PathBuf;

/// Build the component for one active-root source using a deterministic synthetic module origin.
///
/// The receiver-method catalog defaults to empty, which exercises the free-binding projection
/// and confirms a module with no receiver methods records no receiver surfaces.
fn build_origins(source: &str) -> DefinedPublicExportOrigins {
    build_origins_with_catalog(source, ReceiverMethodCatalog::default())
}

/// Build the component for one active-root source with a caller-supplied receiver-method catalog.
fn build_origins_with_catalog(
    source: &str,
    catalog: ReceiverMethodCatalog,
) -> DefinedPublicExportOrigins {
    build_origins_for_project_with_catalog(source, "test-project", catalog)
}

/// Build the component for one active-root source using a configurable project name so module
/// distinction is testable without a second discovered module.
fn build_origins_for_project_with_catalog(
    source: &str,
    project_name: &str,
    catalog: ReceiverMethodCatalog,
) -> DefinedPublicExportOrigins {
    let (mut headers, mut string_table) = parse_single_file_headers_with_table(source);
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(project_name),
        String::new(),
        ModuleRootRole::Normal,
    );

    // Build a source file table for the single synthetic test file and set the retained
    // file identity on every header so the origin projection can resolve the active root
    // from the per-file source-origin table.
    let file_path = PathBuf::from("src/#page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the synthetic test file");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("the synthetic test file should be in the source file table")
        .file_id;
    for header in &mut headers.headers {
        header.tokens.file_id = Some(file_id);
    }

    let source_module_origins =
        SourceModuleOriginTable::from_synthetic_origin(&source_files, &module_origin);

    let draft = build_defined_public_export_origin_draft(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &string_table,
    )
    .expect("defined public export origin draft must build for valid headers");
    draft
        .finalize(&catalog, &string_table)
        .expect("receiver surface origins must finalize for a valid catalog")
}

fn binding_for<'a>(
    origins: &'a DefinedPublicExportOrigins,
    public_name: &str,
) -> &'a ExportBinding {
    origins
        .export_bindings()
        .iter()
        .find(|binding| binding.public_name() == public_name)
        .unwrap_or_else(|| panic!("no export binding for public name `{public_name}`"))
}

fn binding_names(origins: &DefinedPublicExportOrigins) -> Vec<&str> {
    origins
        .export_bindings()
        .iter()
        .map(|binding| binding.public_name())
        .collect()
}

/// Find the canonical declaration path of a directly-defined public nominal type by name.
///
/// The receiver-surface projection matches catalog entries by canonical declaration path, so the
/// test catalog must carry the same path the header parser assigned to the type declaration.
fn nominal_type_path(
    headers: &BoundModuleHeaders,
    type_name: &str,
    string_table: &StringTable,
) -> InternedPath {
    for header in &headers.headers {
        if header.kind.is_authored_public_export_declaration()
            && header.tokens.src_path.name_str(string_table) == Some(type_name)
        {
            return header.tokens.src_path.clone();
        }
    }
    panic!("no public nominal type header named `{type_name}` in test source")
}

/// Build a receiver-method catalog entry for a method on a nominal receiver type.
fn receiver_method_entry(
    method_name: &str,
    receiver: ReceiverKey,
    string_table: &mut StringTable,
) -> (InternedPath, ReceiverMethodEntry) {
    let function_path = InternedPath::from_single_str(method_name, string_table);
    let entry = ReceiverMethodEntry {
        function_path: function_path.clone(),
        receiver,
        source_file: InternedPath::new(),
        receiver_mutable: false,
        signature: FunctionSignature::default(),
    };
    (function_path, entry)
}

/// A public surface exercising every directly-defined public declaration category.
const ALL_CATEGORIES_SOURCE: &str = "\
export:\n\
    render |button Button| -> String:\n\
        return button.label\n\
    ;\n\
    Button = | label String |\n\
    Status :: Ready,\n\
    ;\n\
    Shape as Int\n\
    count #= 1\n\
    DISPLAYABLE must:\n\
        show |This| -> String\n\
    ;\n\
;\n";

#[test]
fn directly_defined_public_exports_get_export_bindings_with_exact_category() {
    let origins = build_origins(ALL_CATEGORIES_SOURCE);

    // Every public declaration category is admitted with its exact origin category.
    assert!(
        matches!(
            binding_for(&origins, "render").origin(),
            OriginDeclarationId::Function(function)
                if matches!(function.kind(), FunctionOriginKind::Free)
        ),
        "a public free function must produce a free-function origin"
    );
    assert!(
        matches!(
            binding_for(&origins, "Button").origin(),
            OriginDeclarationId::Type(type_id)
                if type_id.category() == OriginTypeCategory::Struct
        ),
        "a public struct must produce a struct type origin"
    );
    assert!(
        matches!(
            binding_for(&origins, "Status").origin(),
            OriginDeclarationId::Type(type_id)
                if type_id.category() == OriginTypeCategory::Choice
        ),
        "a public choice must produce a choice type origin"
    );
    assert!(
        matches!(
            binding_for(&origins, "Shape").origin(),
            OriginDeclarationId::Type(type_id)
                if type_id.category() == OriginTypeCategory::TransparentAlias
        ),
        "a public transparent alias must produce a transparent-alias type origin"
    );
    assert!(
        matches!(
            binding_for(&origins, "count").origin(),
            OriginDeclarationId::Constant(_)
        ),
        "a public constant must produce a constant origin"
    );
    assert!(
        matches!(
            binding_for(&origins, "DISPLAYABLE").origin(),
            OriginDeclarationId::Trait(_)
        ),
        "a public trait must produce a trait origin"
    );
}

#[test]
fn receiver_methods_attach_to_receiver_surface_not_free_bindings() {
    // `Counter` is a public struct; `tick` is a private receiver method on `Counter`. Receiver
    // methods cannot be exported directly, so `tick` is never a free namespace binding.
    let source = "\
export:\n\
    Counter = | value Int |\n\
;\n\
tick |this Counter| -> Int:\n\
    return this.value\n\
;\n";

    let (headers, mut string_table) = parse_single_file_headers_with_table(source);
    let counter_path = nominal_type_path(&headers, "Counter", &string_table);
    let (function_path, entry) =
        receiver_method_entry("tick", ReceiverKey::Struct(counter_path), &mut string_table);

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(function_path, entry);

    let origins = build_origins_with_catalog(source, catalog);

    assert!(
        !origins
            .export_bindings()
            .iter()
            .any(|binding| binding.public_name() == "tick"),
        "a receiver method must not become a free namespace export binding"
    );

    let counter_surface = origins
        .receiver_surfaces()
        .iter()
        .find(|surface| surface.receiver().defining_name() == "Counter")
        .expect("a public struct with a receiver method must own a receiver surface");

    assert_eq!(
        counter_surface.receiver().category(),
        OriginTypeCategory::Struct,
        "the receiver surface must carry the receiver's stable type origin"
    );

    let method_names: Vec<&str> = counter_surface
        .methods()
        .iter()
        .map(|method| method.defining_name())
        .collect();
    assert_eq!(
        method_names,
        vec!["tick"],
        "the receiver method must be attached to its receiver surface"
    );

    let tick = &counter_surface.methods()[0];
    assert!(
        matches!(tick.kind(), FunctionOriginKind::Receiver(receiver)
            if receiver.defining_name() == "Counter"
            && receiver.category() == OriginTypeCategory::Struct),
        "the method origin must be built with new_receiver using the stable receiver type origin"
    );
}

#[test]
fn generic_struct_receiver_method_attaches_to_generic_base_struct_origin() {
    // `Box` is a public generic struct; `get` is a receiver method whose resolved receiver key is
    // the generic nominal base `Box`, not the instantiated `Box of A`. The resolved catalog
    // carries the base ReceiverKey, so the method attaches to the `Box` stable struct origin.
    let source = "\
export:\n\
    Box type A = |\n\
        value A,\n\
    |\n\
;\n\
get type A |this Box of A| -> A:\n\
    return this.value\n\
;\n";

    let (headers, mut string_table) = parse_single_file_headers_with_table(source);
    let box_path = nominal_type_path(&headers, "Box", &string_table);
    let (function_path, entry) =
        receiver_method_entry("get", ReceiverKey::Struct(box_path), &mut string_table);

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(function_path, entry);

    let origins = build_origins_with_catalog(source, catalog);

    let box_surface = origins
        .receiver_surfaces()
        .iter()
        .find(|surface| surface.receiver().defining_name() == "Box")
        .expect("a public generic struct with a receiver method must own a receiver surface");

    assert_eq!(
        box_surface.receiver().category(),
        OriginTypeCategory::Struct,
        "the generic receiver method must attach to the generic base struct origin"
    );

    let method_names: Vec<&str> = box_surface
        .methods()
        .iter()
        .map(|method| method.defining_name())
        .collect();
    assert_eq!(
        method_names,
        vec!["get"],
        "the generic receiver method must be attached to its receiver surface"
    );
}

#[test]
fn generic_choice_receiver_method_attaches_to_generic_base_choice_origin() {
    // `Maybe` is a public generic choice; `label` is a receiver method whose resolved receiver key
    // is the generic nominal base `Maybe`, not the instantiated `Maybe of A`. The resolved catalog
    // carries the base ReceiverKey, so the method attaches to the `Maybe` stable choice origin.
    let source = "\
export:\n\
    Maybe type A ::\n\
        Some | value A |,\n\
        Missing,\n\
    ;\n\
;\n\
label type A |this Maybe of A| -> String:\n\
    return \"maybe\"\n\
;\n";

    let (headers, mut string_table) = parse_single_file_headers_with_table(source);
    let maybe_path = nominal_type_path(&headers, "Maybe", &string_table);
    let (function_path, entry) =
        receiver_method_entry("label", ReceiverKey::Choice(maybe_path), &mut string_table);

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(function_path, entry);

    let origins = build_origins_with_catalog(source, catalog);

    let maybe_surface = origins
        .receiver_surfaces()
        .iter()
        .find(|surface| surface.receiver().defining_name() == "Maybe")
        .expect("a public generic choice with a receiver method must own a receiver surface");

    assert_eq!(
        maybe_surface.receiver().category(),
        OriginTypeCategory::Choice,
        "the generic receiver method must attach to the generic base choice origin"
    );

    let method_names: Vec<&str> = maybe_surface
        .methods()
        .iter()
        .map(|method| method.defining_name())
        .collect();
    assert_eq!(
        method_names,
        vec!["label"],
        "the generic choice receiver method must be attached to its receiver surface"
    );
}

#[test]
fn receiver_methods_on_private_types_are_excluded_from_the_public_surface() {
    // `Hidden` is private, so its receiver method is not part of any public surface.
    let source = "\
Hidden = | x Int |\n\
poke |this Hidden| -> Int:\n\
    return this.x\n\
;\n";

    let (headers, mut string_table) = parse_single_file_headers_with_table(source);
    // `Hidden` is not public, so nominal_type_path would not find it. Build the receiver path
    // from the private header directly.
    let hidden_path = headers
        .headers
        .iter()
        .find(|header| {
            matches!(header.kind, HeaderKind::Struct { .. })
                && header.tokens.src_path.name_str(&string_table) == Some("Hidden")
        })
        .map(|header| header.tokens.src_path.clone())
        .expect("private struct Hidden must be in the parsed headers");

    let (function_path, entry) =
        receiver_method_entry("poke", ReceiverKey::Struct(hidden_path), &mut string_table);

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(function_path, entry);

    let origins = build_origins_with_catalog(source, catalog);

    assert!(
        origins.export_bindings().is_empty(),
        "a module with no public exports must record no free bindings"
    );
    assert!(
        origins.receiver_surfaces().is_empty(),
        "a receiver method on a private type must not attach to a public receiver surface"
    );
}

#[test]
fn private_declarations_and_implicit_start_are_excluded() {
    // `helper` and `Inner` are private; the implicit start function is always present for an
    // active module root. Only the public `public_fn` must be recorded.
    let source = "\
helper |value Int| -> Int:\n\
    return value\n\
;\n\
Inner = | x Int |\n\
export:\n\
    public_fn |x Int| -> Int:\n\
        return x\n\
    ;\n\
;\n";

    let origins = build_origins(source);

    assert_eq!(
        binding_names(&origins),
        vec!["public_fn"],
        "private declarations and the implicit start function must be excluded"
    );
    assert!(
        origins.receiver_surfaces().is_empty(),
        "a module with no public receiver types must record no receiver surfaces"
    );
}

#[test]
fn ordering_is_deterministic_independent_of_declaration_scheduling() {
    let order_one = "\
export:\n\
    zebra #= 1\n\
    alpha #= 2\n\
;\n";
    let order_two = "\
export:\n\
    alpha #= 2\n\
    zebra #= 1\n\
;\n";

    let first = build_origins(order_one);
    let second = build_origins(order_two);

    assert_eq!(
        binding_names(&first),
        vec!["alpha", "zebra"],
        "export bindings must be sorted by public name"
    );
    assert_eq!(
        first, second,
        "the component must be identical regardless of declaration scheduling"
    );
}

#[test]
fn active_origin_missing_from_table_fails_internally() {
    // Hidden invariant: when the active root's FileId maps to no owning module origin, the
    // projection must fail through CompilerError rather than silently using a fallback origin.
    let (mut headers, mut string_table) =
        parse_single_file_headers_with_table("export:\n    alpha #= 1\n;\n");

    let file_path = PathBuf::from("src/#page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .file_id;
    for header in &mut headers.headers {
        header.tokens.file_id = Some(file_id);
    }

    // Build a table where every file maps to None (simulating a source-package file outside the
    // project module graph that somehow became an active root).
    let empty_lookup = rustc_hash::FxHashMap::default();
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &empty_lookup);

    let result = build_defined_public_export_origin_draft(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("an active root with no owning module origin must fail"),
    };
    assert!(
        error.msg.contains("no owning module origin"),
        "the error must state the missing-origin violation, got: {}",
        error.msg
    );
}

#[test]
fn out_of_range_active_root_file_id_fails_internally() {
    // Hidden invariant: an out-of-range FileId is an internal CompilerError, not a silent None.
    let (mut headers, mut string_table) =
        parse_single_file_headers_with_table("export:\n    alpha #= 1\n;\n");

    let file_path = PathBuf::from("src/#page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .file_id;
    for header in &mut headers.headers {
        header.tokens.file_id = Some(file_id);
    }

    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let source_module_origins =
        SourceModuleOriginTable::from_synthetic_origin(&source_files, &module_origin);

    let result = build_defined_public_export_origin_draft(
        &source_module_origins,
        FileId(999),
        &headers.headers,
        &headers.module_symbols,
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("an out-of-range active root FileId must fail"),
    };
    assert!(
        error.msg.contains("out-of-range"),
        "the error must state the out-of-range violation, got: {}",
        error.msg
    );
}

#[test]
fn conflicting_public_header_ownership_fails_internally() {
    // Hidden invariant: when two directly-defined public headers resolve to different owning
    // module origins, the projection must fail rather than picking one silently.
    let (mut headers, mut string_table) =
        parse_single_file_headers_with_table("export:\n    alpha #= 1\n    beta #= 2\n;\n");

    let file_path = PathBuf::from("src/#page.moth");
    let second_path = PathBuf::from("src/other.moth");
    let source_files = SourceFileTable::build(
        [file_path.clone(), second_path.clone()],
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build with two files");
    let active_file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root should be in the source file table")
        .file_id;
    let other_file_id = source_files
        .get_by_canonical_path(&second_path)
        .expect("second file should be in the source file table")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let other_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "other".to_string(),
        ModuleRootRole::Normal,
    );

    let mut lookup = rustc_hash::FxHashMap::default();
    lookup.insert(file_path.clone(), active_origin.clone());
    lookup.insert(second_path.clone(), other_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &lookup);

    // Set one header to the active file and the other to the other file so their origins differ.
    for (index, header) in headers.headers.iter_mut().enumerate() {
        header.tokens.file_id = Some(if index == 0 {
            active_file_id
        } else {
            other_file_id
        });
    }

    let result = build_defined_public_export_origin_draft(
        &source_module_origins,
        active_file_id,
        &headers.headers,
        &headers.module_symbols,
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("conflicting public header ownership must fail"),
    };
    assert!(
        error.msg.contains("does not match the active root origin"),
        "the error must state the ownership conflict, got: {}",
        error.msg
    );
}

#[test]
fn zero_public_exports_still_validates_active_origin() {
    // Hidden invariant: the active root origin is validated from the table even when the module
    // has zero directly-defined public exports. An in-range active root whose table entry is
    // None must still fail, proving lookup and validation run before any header is inspected.
    let (headers, mut string_table) = parse_single_file_headers_with_table("#page\n");

    let file_path = PathBuf::from("src/#page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .file_id;

    // Build a table where the in-range active file maps to None, simulating an unowned active
    // root. With zero public exports the header loop never runs, so only the active-root lookup
    // and validation can fail — proving they execute before any header is inspected.
    let empty_lookup = rustc_hash::FxHashMap::default();
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &empty_lookup);
    let result = build_defined_public_export_origin_draft(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!(
            "an active root with no owning module origin must fail even with zero public exports"
        ),
    };
    assert!(
        error.msg.contains("no owning module origin"),
        "the error must state the missing-origin violation, got: {}",
        error.msg
    );
}

#[test]
fn receiver_surfaces_are_ordered_by_receiver_name_with_methods_ordered_by_name() {
    // Two public structs each with two receiver methods. Surfaces sort by receiver name; methods
    // sort by method name, independent of source order.
    let source = "\
export:\n\
    Alpha = | v Int |\n\
    Beta = | v Int |\n\
;\n\
zap |this Beta| -> Int:\n\
    return this.v\n\
;\n\
beta_first |this Beta| -> Int:\n\
    return this.v\n\
;\n\
zoom |this Alpha| -> Int:\n\
    return this.v\n\
;\n\
alpha_first |this Alpha| -> Int:\n\
    return this.v\n\
;\n";

    let (headers, mut string_table) = parse_single_file_headers_with_table(source);
    let alpha_path = nominal_type_path(&headers, "Alpha", &string_table);
    let beta_path = nominal_type_path(&headers, "Beta", &string_table);

    let mut catalog = ReceiverMethodCatalog::default();
    for (method_name, receiver) in [
        ("zap", ReceiverKey::Struct(beta_path.clone())),
        ("beta_first", ReceiverKey::Struct(beta_path.clone())),
        ("zoom", ReceiverKey::Struct(alpha_path.clone())),
        ("alpha_first", ReceiverKey::Struct(alpha_path.clone())),
    ] {
        let (function_path, entry) =
            receiver_method_entry(method_name, receiver, &mut string_table);
        catalog.by_function_path.insert(function_path, entry);
    }

    let origins = build_origins_with_catalog(source, catalog);

    let receiver_names: Vec<&str> = origins
        .receiver_surfaces()
        .iter()
        .map(|surface| surface.receiver().defining_name())
        .collect();
    assert_eq!(
        receiver_names,
        vec!["Alpha", "Beta"],
        "receiver surfaces must be ordered by receiver defining name"
    );

    let alpha = &origins.receiver_surfaces()[0];
    let method_names: Vec<&str> = alpha
        .methods()
        .iter()
        .map(|method| method.defining_name())
        .collect();
    assert_eq!(
        method_names,
        vec!["alpha_first", "zoom"],
        "methods within a receiver surface must be ordered by method defining name"
    );
}

// ---------------------------------------------------------------------------
//  Transient expanded public source-nominal origin index (graph-derived origins)
// ---------------------------------------------------------------------------

/// Find the canonical declaration path of a struct header by defining name.
fn struct_header_path(headers: &[Header], name: &str, string_table: &StringTable) -> InternedPath {
    for header in headers {
        if matches!(header.kind, HeaderKind::Struct { .. })
            && header.tokens.src_path.name_str(string_table) == Some(name)
        {
            return header.tokens.src_path.clone();
        }
    }
    panic!("no public struct header named `{name}` in test headers")
}

/// Find the canonical declaration path of a trait header by defining name.
fn trait_header_path(headers: &[Header], name: &str, string_table: &StringTable) -> InternedPath {
    for header in headers {
        if matches!(header.kind, HeaderKind::Trait { .. })
            && header.tokens.src_path.name_str(string_table) == Some(name)
        {
            return header.tokens.src_path.clone();
        }
    }
    panic!("no trait header named `{name}` in test headers")
}

/// Build a `ModuleSymbols` whose retained module-root public exports target the given source
/// paths, modelling the header-built public export maps without standing up a full project path
/// resolver.
///
/// The origin index membership check is key-agnostic (it scans every retained entry's target), so
/// all entries live under one representative module-root key. Each export name is the target
/// path's defining name, matching the direct-public-declaration export shape produced by
/// `headers::public_exports` pass 1.
fn module_symbols_with_module_root_export_targets(
    targets: &[InternedPath],
    string_table: &mut StringTable,
) -> ModuleSymbols {
    let module_root_key = InternedPath::from_single_str("test-module-root", string_table);
    let entries: rustc_hash::FxHashSet<PublicExportEntry> = targets
        .iter()
        .map(|target| PublicExportEntry {
            export_name: target
                .name()
                .expect("an export target path must carry a defining name"),
            target: PublicExportTarget::Source(target.clone()),
        })
        .collect();
    let mut module_symbols = ModuleSymbols::empty();
    module_symbols
        .module_root_public_exports
        .insert(module_root_key, entries);
    module_symbols
}

/// Add a retained source-package public export entry targeting the given source path, modelling a
/// source-backed package public surface without a full project path resolver.
fn add_source_package_export_target(
    module_symbols: &mut ModuleSymbols,
    package_prefix: &str,
    target: &InternedPath,
) {
    let entry = PublicExportEntry {
        export_name: target
            .name()
            .expect("an export target path must carry a defining name"),
        target: PublicExportTarget::Source(target.clone()),
    };
    module_symbols
        .source_package_public_exports
        .entry(package_prefix.to_owned())
        .or_default()
        .insert(entry);
}

#[test]
fn public_source_nominal_origin_index_includes_imported_provider_origin() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let imported_path = PathBuf::from("src/#mod.moth");

    // The active root is the entry file; the imported root is a hash-root file compiled only to
    // validate its public declaration surface.
    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let imported_output = prepare_single_file(
        "export:\n    Imported = | value Int |\n;\n",
        &imported_path,
        &active_path,
        &mut string_table,
    );

    assert_eq!(active_output.file_role, FileRole::ActiveModuleRoot);
    assert_eq!(imported_output.file_role, FileRole::ImportedModuleRoot);

    let source_files = SourceFileTable::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the two module-root files");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let imported_file_id = source_files
        .get_by_canonical_path(&imported_path)
        .expect("the imported root file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        header.tokens.file_id = Some(imported_file_id);
        headers.push(header);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "imported".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(imported_path.clone(), provider_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    // Each module root's retained public export targets its own public nominal's source path:
    // the active root exports `Local` and the imported root exports `Imported`. The index admits
    // a nominal when a retained public export entry targets its canonical source path, mirroring
    // the AST `source_path_is_public_from_root_file` nameability owner.
    let local_path = struct_header_path(&headers, "Local", &string_table);
    let imported_path_decl = struct_header_path(&headers, "Imported", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[local_path.clone(), imported_path_decl.clone()],
        &mut string_table,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the expanded nominal origin index should build for active plus imported roots");

    // The active-root public nominal resolves to the active module origin.
    assert_eq!(
        index.get(&local_path),
        Some(&OriginTypeId::new(
            active_origin.clone(),
            "Local".to_owned(),
            OriginTypeCategory::Struct
        )),
        "an active-root public struct must resolve to the active module origin"
    );

    // The imported-root public nominal resolves to its defining provider module origin, not the
    // active origin.
    assert_eq!(
        index.get(&imported_path_decl),
        Some(&OriginTypeId::new(
            provider_origin.clone(),
            "Imported".to_owned(),
            OriginTypeCategory::Struct
        )),
        "an imported public struct must resolve to its provider module origin, not the active \
         module origin"
    );
}

#[test]
fn public_source_nominal_origin_index_rejects_missing_file_id() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let imported_path = PathBuf::from("src/#mod.moth");

    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let imported_output = prepare_single_file(
        "export:\n    Imported = | value Int |\n;\n",
        &imported_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;

    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "imported".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(imported_path.clone(), provider_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    // The active root keeps its file id; the imported root's headers deliberately carry no file id.
    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for header in imported_output.headers {
        // Imported headers keep file_id = None (unprepared identity), which must be rejected
        // explicitly rather than silently skipped.
        let mut header = header;
        header.tokens.file_id = None;
        headers.push(header);
    }

    // `Imported` is targeted by a retained module-root public export entry, so the index admits
    // it; its missing retained FileId is then an internal invariant violation rather than a
    // silent skip.
    let imported_path_decl = struct_header_path(&headers, "Imported", &string_table);
    let module_symbols =
        module_symbols_with_module_root_export_targets(&[imported_path_decl], &mut string_table);

    let result = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    );
    assert!(
        result.is_err(),
        "a public export-targeted nominal header with no retained FileId must be a CompilerError"
    );
}

#[test]
fn public_source_nominal_origin_index_skips_unowned_source_package_nominal() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let package_path = PathBuf::from("src/#pkg.moth");

    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    // A source-package module root not owned by the project graph: deliberately absent from the
    // origin map, so its table entry is None.
    let package_output = prepare_single_file(
        "export:\n    Pkg = | value Int |\n;\n",
        &package_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), package_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;
    let package_file_id = source_files
        .get_by_canonical_path(&package_path)
        .expect("package root file should be present")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    // Only the active root is owned; the package root maps to None.
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in package_output.headers {
        header.tokens.file_id = Some(package_file_id);
        headers.push(header);
    }

    // `Local` is targeted by a retained module-root public export (admitted, owned, present).
    // `Pkg` is targeted by a retained source-package public export (admitted, but its file maps
    // to None ownership, so it is skipped rather than given a fabricated origin).
    let local_path = struct_header_path(&headers, "Local", &string_table);
    let pkg_path = struct_header_path(&headers, "Pkg", &string_table);
    let mut module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&local_path),
        &mut string_table,
    );
    add_source_package_export_target(&mut module_symbols, "pkg", &pkg_path);

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build; the unowned package nominal is skipped, not an error");

    // The active nominal is present; the unowned package nominal is deliberately absent.
    assert!(
        index.contains_key(&local_path),
        "the active-root public nominal must be in the index"
    );
    assert!(
        !index.contains_key(&pkg_path),
        "a source-package nominal with no project-module owner (None) must be absent from the \
         index, not assigned a fabricated origin"
    );
}

/// A privately-authored nominal in a normal file is included when a module-root public export
/// (an alias or re-export) targets its canonical source path, resolving to the normal file's
/// graph-derived module origin.
#[test]
fn public_source_nominal_origin_index_includes_alias_targeted_normal_file_nominal() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active module root carries an unrelated public constant; `Counter` is authored as a
    // private struct in the normal file `impl.moth` and has no public export of its own. A
    // module-root public alias (`PublicCounter as Counter`) re-exports it, so the retained
    // module-root public export entry targets `Counter`'s canonical source path.
    let active_output = prepare_single_file(
        "export:\n    placeholder #= 1\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let impl_output = prepare_single_file(
        "Counter = | count Int |\n",
        &impl_path,
        &active_path,
        &mut string_table,
    );
    assert_eq!(active_output.file_role, FileRole::ActiveModuleRoot);
    assert_eq!(impl_output.file_role, FileRole::Normal);

    let source_files = SourceFileTable::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the active root plus normal file");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens.file_id = Some(impl_file_id);
        headers.push(header);
    }

    // The normal file inherits its nearest owning module origin from the project graph, the same
    // module origin as the active root, so the alias-targeted nominal resolves to that origin.
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), module_origin.clone());
    origin_by_canonical_path.insert(impl_path.clone(), module_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let counter_path = struct_header_path(&headers, "Counter", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&counter_path),
        &mut string_table,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build; the alias-targeted normal-file nominal is included");

    assert_eq!(
        index.get(&counter_path),
        Some(&OriginTypeId::new(
            module_origin.clone(),
            "Counter".to_owned(),
            OriginTypeCategory::Struct
        )),
        "a privately-authored nominal exposed through a module-root public alias must resolve to its normal file's graph-derived module origin"
    );
}

/// A privately-authored nominal in a normal file with no public export target remains absent
/// from the index, while a directly-defined active-root public nominal targeted by its own export
/// is present.
#[test]
fn public_source_nominal_origin_index_excludes_private_normal_file_nominal_without_target() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active root exports `Local` publicly; `Counter` is a private struct in the normal file
    // with no public export targeting it.
    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let impl_output = prepare_single_file(
        "Counter = | count Int |\n",
        &impl_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens.file_id = Some(impl_file_id);
        headers.push(header);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(impl_path.clone(), active_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    // Only `Local` is targeted by a retained module-root public export; `Counter` has no target.
    let local_path = struct_header_path(&headers, "Local", &string_table);
    let counter_path = struct_header_path(&headers, "Counter", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&local_path),
        &mut string_table,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build");

    assert!(
        index.contains_key(&local_path),
        "the directly-defined active-root public nominal must be in the index"
    );
    assert!(
        !index.contains_key(&counter_path),
        "a private normal-file nominal with no public export target must be absent from the index"
    );
}

// ---------------------------------------------------------------------------
//  Transient expanded public source-trait origin index (graph-derived origins)
// ---------------------------------------------------------------------------

#[test]
fn public_source_trait_origin_index_includes_directly_defined_trait() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/#page.moth");
    let output = prepare_single_file(
        "export:\n    RENDERABLE must:\n        show |This| -> String\n    ;\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    let mut headers: Vec<Header> = output.headers;
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root file should be present")
        .file_id;
    for header in &mut headers {
        header.tokens.file_id = Some(file_id);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path.clone(), active_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&trait_path),
        &mut string_table,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for a directly-defined public trait");

    assert_eq!(
        index.get(&trait_path),
        Some(&OriginTraitId::new(active_origin, "RENDERABLE".to_owned())),
        "a directly-defined public trait must resolve to the active module origin"
    );
}

#[test]
fn public_source_trait_origin_index_includes_imported_provider_trait() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let imported_path = PathBuf::from("src/#mod.moth");

    let active_output = prepare_single_file(
        "export:\n    RENDERABLE must:\n        show |This| -> String\n    ;\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let imported_output = prepare_single_file(
        "export:\n    IMPORTED_TRAIT must:\n        show |This| -> String\n    ;\n;\n",
        &imported_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;
    let imported_file_id = source_files
        .get_by_canonical_path(&imported_path)
        .expect("imported root file should be present")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "imported".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(imported_path.clone(), provider_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        header.tokens.file_id = Some(imported_file_id);
        headers.push(header);
    }

    let local_trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let imported_trait_path = trait_header_path(&headers, "IMPORTED_TRAIT", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[local_trait_path.clone(), imported_trait_path.clone()],
        &mut string_table,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for active plus imported roots");

    assert_eq!(
        index.get(&local_trait_path),
        Some(&OriginTraitId::new(active_origin, "RENDERABLE".to_owned())),
        "an active-root public trait must resolve to the active module origin"
    );
    assert_eq!(
        index.get(&imported_trait_path),
        Some(&OriginTraitId::new(
            provider_origin,
            "IMPORTED_TRAIT".to_owned()
        )),
        "an imported public trait must resolve to its provider module origin"
    );
}

/// A privately-authored trait in a normal file is included when a module-root public export
/// (an alias or re-export) targets its canonical source path, resolving to the normal file's
/// graph-derived module origin.
#[test]
fn public_source_trait_origin_index_includes_alias_targeted_normal_file_trait() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/#page.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active module root carries an unrelated public constant; `RENDERABLE` is authored as
    // a private trait in the normal file `impl.moth` (no `export:` of its own). A module-root
    // public re-export entry targets `RENDERABLE`'s canonical source path, so the retained
    // module-root public export admits the trait into the origin index.
    let active_output = prepare_single_file(
        "export:\n    placeholder #= 1\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let impl_output = prepare_single_file(
        "RENDERABLE must:\n    show |This| -> String\n;\n",
        &impl_path,
        &active_path,
        &mut string_table,
    );
    assert_eq!(active_output.file_role, FileRole::ActiveModuleRoot);
    assert_eq!(impl_output.file_role, FileRole::Normal);

    let source_files = SourceFileTable::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the active root plus normal file");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens.file_id = Some(impl_file_id);
        headers.push(header);
    }

    // The normal file inherits its nearest owning module origin from the project graph, the same
    // module origin as the active root, so the alias-targeted trait resolves to that origin.
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), module_origin.clone());
    origin_by_canonical_path.insert(impl_path.clone(), module_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&trait_path),
        &mut string_table,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect(
        "the trait origin index should build; the alias-targeted normal-file trait is included",
    );

    assert_eq!(
        index.get(&trait_path),
        Some(&OriginTraitId::new(module_origin, "RENDERABLE".to_owned())),
        "a privately-authored trait exposed through a module-root public re-export must resolve to its normal file's graph-derived module origin"
    );
}

#[test]
fn public_source_trait_origin_index_excludes_unexported_private_trait() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/#page.moth");
    let output = prepare_single_file(
        "RENDERABLE must:\n    show |This| -> String\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    let mut headers: Vec<Header> = output.headers;
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root file should be present")
        .file_id;
    for header in &mut headers {
        header.tokens.file_id = Some(file_id);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path.clone(), active_origin);
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    // No public export targets the trait path, so it is unexported.
    let module_symbols = ModuleSymbols::empty();

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for an unexported trait");

    assert!(
        !index.contains_key(&trait_path),
        "a private unexported trait must be absent from the trait origin index"
    );
}

#[test]
fn public_source_trait_origin_index_skips_unowned_source_package_trait() {
    let mut string_table = StringTable::new();
    let package_path = PathBuf::from("src/#pkg.moth");

    let output = prepare_single_file(
        "export:\n    PKG_TRAIT must:\n        show |This| -> String\n    ;\n;\n",
        &package_path,
        &package_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        std::iter::once(package_path.clone()),
        &package_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&package_path)
        .expect("package file should be present")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in output.headers {
        header.tokens.file_id = Some(file_id);
        headers.push(header);
    }

    // The package file has an explicit None owning origin (no project-module owner).
    let source_module_origins = SourceModuleOriginTable::from_graph_ownership(
        &source_files,
        &rustc_hash::FxHashMap::default(),
    );

    let trait_path = trait_header_path(&headers, "PKG_TRAIT", &string_table);
    let mut module_symbols = ModuleSymbols::empty();
    add_source_package_export_target(&mut module_symbols, "pkg", &trait_path);

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for an unowned source-package trait");

    assert!(
        !index.contains_key(&trait_path),
        "an unowned source-package trait must be absent from the trait origin index"
    );
}

#[test]
fn public_source_trait_origin_index_rejects_missing_file_id() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/#page.moth");
    let output = prepare_single_file(
        "export:\n    RENDERABLE must:\n        show |This| -> String\n    ;\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    let headers: Vec<Header> = output.headers;
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    // Deliberately keep file_id = None on all headers.
    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path, active_origin);
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let module_symbols =
        module_symbols_with_module_root_export_targets(&[trait_path], &mut string_table);

    let result = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    );
    assert!(
        result.is_err(),
        "a public export-targeted trait header with no retained FileId must be a CompilerError"
    );
}
