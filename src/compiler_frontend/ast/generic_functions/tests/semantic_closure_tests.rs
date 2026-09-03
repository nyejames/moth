//! Focused test for the frozen semantic closure of a generic module.
//!
//! WHAT: proves a retained local alias whose target handle cannot be projected fails at that
//! alias declaration rather than at the start of the file.
//! WHY: the AST environment makes a malformed present alias row unrepresentable, but a generated
//! sidecar is built from retained facts and must still refuse to freeze a fact it cannot project.
//! Alias-target reach into nominal blueprints is owned by the cross-module integration cases,
//! because that is where a missing blueprint is observable.

use super::ModuleMaterialisationPreparation;
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;

struct PreparedModule {
    preparation: ModuleMaterialisationPreparation,
    public_interface: PublicSemanticInterface,
    string_table: StringTable,
}

fn prepared_module(source: &str, package: &str) -> PreparedModule {
    let (mut build_result, string_table) =
        parse_single_file_ast_build_result(source).expect("generic source should build");
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(package),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );

    // Declaration identities are installed by module compilation, so this fixture assigns the
    // one identity its single template needs to reach the freeze boundary.
    let template = build_result
        .materialisation_context
        .generic_function_templates_mut()
        .values_mut()
        .next()
        .expect("the source should retain one generic template");
    template.declaration_identity = Some(GeneratedDeclarationIdentity::ModulePrivate(
        ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            "probe".to_owned(),
            None,
        ),
    ));

    let mut preparation = build_result
        .materialisation_context
        .finish_preparation()
        .expect("generic template identity index should build");
    preparation.module_origin = Some(module_origin.clone());

    PreparedModule {
        preparation,
        public_interface: PublicSemanticInterface {
            module_origin,
            export_bindings: Vec::new(),
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        },
        string_table,
    }
}

const ALIAS_SOURCE: &str = concat!(
    "Count as Int\n",
    "\n",
    "probe type T |value T| -> T:\n",
    "    seen Count = 1\n",
    "    return value\n",
    ";\n",
    "\n",
    "probe(1)\n",
);

#[test]
fn unprojectable_retained_alias_target_fails_at_the_alias_declaration() {
    let control = prepared_module(ALIAS_SOURCE, "closure-alias-control");
    control
        .preparation
        .freeze(&control.public_interface, &ModuleResourceTable::new())
        .expect("a completed alias target must freeze")
        .expect("the retained generic should produce a materialisation context");

    let mut prepared = prepared_module(ALIAS_SOURCE, "closure-alias-tests");
    // The alias row is complete by construction, so the only way to reach the generated-side
    // defence is to replace its handle with one the type environment never interned.
    let alias_path = prepared
        .preparation
        .resolved_type_aliases_by_path
        .keys()
        .find(|path| path.name_str(&prepared.string_table) == Some("Count"))
        .cloned()
        .expect("the module should retain its Count alias");
    let alias = prepared
        .preparation
        .resolved_type_aliases_by_path
        .get_mut(&alias_path)
        .expect("the alias row should be present");
    alias.target_type_id = TypeId(u32::MAX);

    let freeze_result = prepared
        .preparation
        .freeze(&prepared.public_interface, &ModuleResourceTable::new());
    let Err(mut error) = freeze_result else {
        panic!("an unprojectable alias target must not freeze");
    };

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("Count") && error.msg.contains("completed-target invariant"),
        "unexpected alias freeze error: {error:?}"
    );
    // `Count as Int` is the first authored line and the name starts the line, so the reported
    // location is the alias declaration rather than a default file-start location.
    assert_eq!(
        (
            error.location.start_pos.line_number,
            error.location.start_pos.char_column
        ),
        (0, 1)
    );
    assert!(
        error.take_render_context().is_some(),
        "the alias declaration location must survive transport with its own string table"
    );
}
