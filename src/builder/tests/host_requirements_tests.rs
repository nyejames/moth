//! Host import and export requirement production tests.
//!
//! WHAT: compiles real temporary Moth projects through the SDK definition path and checks the
//! `moth::host` requirements the compiler derives for the project package assembly, then runs the
//! shared preflight against implementations a native runtime would declare.
//! WHY: requirement production joins three compiler-owned facts, namely facade reachability, the
//! per-module external package registries and the facade's public semantic interface. Only a real
//! project compilation, with real host-package registration, proves those facts join correctly.

use crate::builder::host_requirements::host_requirements;
use crate::builder::{
    BuildArtifacts, BuildContext, BuildRequest, BuilderDefinition, CompiledProject, Error,
    ErrorKind, HostPackage, ProjectBuilder, SourcePackage, build_project,
};
use moth_host::{
    HostBindingIdentity, HostExportRequirement, HostExportSelection, HostExports, HostFunction,
    HostImplementation, HostRequirements, HostSignature, HostType, PreflightError, preflight,
};
use std::cell::RefCell;
use std::fs;
use std::path::Path;

/// The mandatory project configuration record.
const CONFIG_SOURCE: &str = "project #= (\n    name = \"docs\",\n)\n";

/// The facade a native host import test compiles. `log` and `has_prefix` are both host bindings.
const IMPORTING_FACADE_SOURCE: &str = "\
@platform has_prefix, log

$export
update |text String| -> Bool:
    log(text)
    return has_prefix(text, \"pre\")
;
";

/// The facade a preflight test compiles. `load` declares the builtin `Error!` channel.
const FALLIBLE_IMPORT_FACADE_SOURCE: &str = "\
@platform has_prefix, load

$export
update |text String| -> I32, Error!:
    if has_prefix(text, \"pre\"):
        return load()!
    ;
    return 0
;
";

/// The facade an export test compiles. `secret` and `label` are never selected.
const SELECTION_FACADE_SOURCE: &str = "\
$export
update || -> I32:
    return 1
;

$export
secret || -> I32:
    return 2
;

$export
label #= \"facade\"
";

/// The facade a multi-slot export test compiles.
const MULTI_SLOT_FACADE_SOURCE: &str = "\
$export
pair |value I32, flag Bool| -> I32, String, Error!:
    return value, \"ok\"
;
";

/// The facade a rejection test compiles. It exposes one declaration of each rejected shape.
const REJECTION_FACADE_SOURCE: &str = "\
$export
label #= \"facade\"

$export
identity type T |value T| -> T:
    return value
;

$export
Failure = | text String |

$export
risky || -> I32, Failure!:
    return 1
;

$export
Point = | x I32 |

$export
take_point |point Point| -> I32:
    return point.x
;

$export
make_point || -> Point:
    return Point(1)
;
";

/// The facade a Core-binding test compiles. `io.line` is a compiler Core binding, not a native
/// host binding, so only `has_prefix` may become a required host import.
const CORE_BINDING_FACADE_SOURCE: &str = "\
@platform has_prefix

$export
update |text String| -> Bool:
    io.line(\"checking\")
    return has_prefix(text, \"pre\")
;
";

/// A facade whose selected export takes a `Wire` parameter. The language accepts the contract,
/// but the V1 host ABI cannot carry a binding identity.
const WIRE_PARAMETER_FACADE_SOURCE: &str = "\
$export
update |value Wire of String| -> String:
    return value
;
";

/// A facade whose selected export takes a zero-input `Route` parameter.
const ROUTE_PARAMETER_FACADE_SOURCE: &str = "\
$export
Message ::
    Delete,
;

$export
update |on_delete Route of Message| -> I32:
    return 0
;
";

/// A facade whose selected export takes a mutable parameter.
const MUTABLE_PARAMETER_FACADE_SOURCE: &str = "\
$export
update |value ~String| -> I32:
    return 0
;
";

/// The child module whose exported function the project package facade re-exports.
const REEXPORTED_IMPORT_MODULE_SOURCE: &str = "\
@platform has_prefix

$export
update |text String| -> Bool:
    return has_prefix(text, \"pre\")
;
";

/// The project package facade that re-exports the child module's function without a wrapper.
const REEXPORTED_IMPORT_FACADE_SOURCE: &str = "\
$export
@api update
";

fn write_project(project_root: &Path, files: &[(&str, &str)]) {
    fs::write(project_root.join("@config.moth"), CONFIG_SOURCE).expect("should write config");
    for (name, source) in files {
        let path = project_root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("should create the fixture directory");
        }
        fs::write(path, source).expect("should write the fixture file");
    }
}

fn definition(host_packages: &[HostPackage]) -> BuilderDefinition {
    let mut draft = BuilderDefinition::new("game");
    for package in host_packages {
        draft = draft.host_package(package.clone());
    }
    draft.finish().expect("the test definition should finish")
}

fn selection(selected: &[(&str, &str)]) -> HostExportSelection {
    let mut exports = HostExports::new();
    for (facade_name, export_name) in selected {
        exports = exports.export(*facade_name, *export_name);
    }
    exports.finish().expect("the test selection should finish")
}

/// A builder that derives the host requirements of its compiled project and records the result.
///
/// The requirements stay compiler-internal, so the test harness reads them through this builder
/// instead of a public accessor. `build` always succeeds, which keeps a requirement failure
/// observable beside a successful compilation.
struct RequirementBuilder {
    definition: BuilderDefinition,
    selection: HostExportSelection,
    observed: RefCell<Option<Result<HostRequirements, Error>>>,
}

impl RequirementBuilder {
    fn new(definition: BuilderDefinition, selection: HostExportSelection) -> Self {
        Self {
            definition,
            selection,
            observed: RefCell::new(None),
        }
    }

    /// Compiles `files` as a project and returns what requirement production reported.
    fn requirements(&self, files: &[(&str, &str)]) -> Result<HostRequirements, Error> {
        let project_dir = tempfile::tempdir().expect("should create project directory");
        write_project(project_dir.path(), files);
        build_project(self, BuildRequest::new(project_dir.path()))
            .expect("the requirement harness project should compile");
        self.observed
            .borrow_mut()
            .take()
            .expect("the build step should record the requirements")
    }
}

impl ProjectBuilder for RequirementBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        Ok(self.definition.clone())
    }

    fn build(
        &self,
        project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        let requirements = host_requirements(&project.compilation, &self.selection);
        *self.observed.borrow_mut() = Some(requirements);
        Ok(BuildArtifacts::new())
    }
}

/// The host package the import fixtures register.
fn platform_package() -> HostPackage {
    HostPackage::new("@platform")
        .function(
            HostFunction::new("has_prefix")
                .parameter(HostType::String)
                .parameter(HostType::String)
                .returns(HostType::Bool),
        )
        .function(HostFunction::new("log").parameter(HostType::String))
}

/// The host package the fallible import fixture registers.
fn fallible_platform_package() -> HostPackage {
    platform_package().function(HostFunction::new("load").returns(HostType::I32).fallible())
}

fn has_prefix_implementation() -> HostImplementation {
    HostImplementation::new(
        HostBindingIdentity::new("@platform", ["has_prefix"]),
        HostSignature::new()
            .parameter(HostType::String)
            .parameter(HostType::String)
            .returns(HostType::Bool),
    )
}

fn log_implementation() -> HostImplementation {
    HostImplementation::new(
        HostBindingIdentity::new("@platform", ["log"]),
        HostSignature::new().parameter(HostType::String),
    )
}

fn load_implementation() -> HostImplementation {
    HostImplementation::new(
        HostBindingIdentity::new("@platform", ["load"]),
        HostSignature::new().returns(HostType::I32).fallible(),
    )
}

fn import_identities(requirements: &HostRequirements) -> Vec<String> {
    requirements
        .imports()
        .iter()
        .map(|import| import.identity().to_string())
        .collect()
}

fn build_error(result: Result<HostRequirements, Error>) -> Error {
    let error = result.expect_err("the export selection should be rejected");
    assert_eq!(error.kind(), ErrorKind::Build);
    error
}

#[test]
fn accepted_host_import_appears_with_canonical_identity_and_signature() {
    let builder = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("update", "update_export")]),
    );

    let requirements = builder
        .requirements(&[("@package.moth", IMPORTING_FACADE_SOURCE)])
        .expect("registered host imports should produce requirements");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix", "@platform#log"],
        "reachable host bindings appear once each in canonical order"
    );
    assert_eq!(
        requirements.exports(),
        [HostExportRequirement::new(
            "update",
            "update_export",
            HostSignature::new()
                .parameter(HostType::String)
                .returns(HostType::Bool),
        )],
        "the selected export carries the facade function's semantic signature"
    );

    let has_prefix = &requirements.imports()[0];
    assert_eq!(has_prefix.identity().package_path(), "@platform");
    assert_eq!(
        has_prefix.identity().symbol_path(),
        ["has_prefix".to_owned()].as_slice(),
        "the canonical identity keeps the package and symbol components structured"
    );
    assert_eq!(
        has_prefix.signature(),
        &HostSignature::new()
            .parameter(HostType::String)
            .parameter(HostType::String)
            .returns(HostType::Bool)
    );
    assert_eq!(
        requirements.imports()[1].signature(),
        &HostSignature::new().parameter(HostType::String)
    );

    assert_eq!(
        preflight(
            &requirements,
            &[has_prefix_implementation(), log_implementation()]
        ),
        Ok(())
    );
}

#[test]
fn preflight_rejects_a_missing_implementation() {
    let builder = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("update", "update_export")]),
    );
    let requirements = builder
        .requirements(&[("@package.moth", IMPORTING_FACADE_SOURCE)])
        .expect("registered host imports should produce requirements");

    let errors = preflight(&requirements, &[has_prefix_implementation()])
        .expect_err("the log import has no implementation");

    assert_eq!(
        errors.errors(),
        [PreflightError::MissingImplementation {
            import: HostBindingIdentity::new("@platform", ["log"]),
        }]
    );
}

#[test]
fn preflight_rejects_a_wrong_parameter_type() {
    let builder = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("update", "update_export")]),
    );
    let requirements = builder
        .requirements(&[("@package.moth", IMPORTING_FACADE_SOURCE)])
        .expect("registered host imports should produce requirements");

    let wrong_parameter = HostImplementation::new(
        HostBindingIdentity::new("@platform", ["has_prefix"]),
        HostSignature::new()
            .parameter(HostType::U8)
            .parameter(HostType::String)
            .returns(HostType::Bool),
    );
    let errors = preflight(&requirements, &[wrong_parameter, log_implementation()])
        .expect_err("a parameter type mismatch should be rejected");

    assert_eq!(
        errors.errors(),
        [PreflightError::ParameterTypeMismatch {
            import: HostBindingIdentity::new("@platform", ["has_prefix"]),
            index: 0,
            required: HostType::String,
            declared: HostType::U8,
        }]
    );
}

#[test]
fn preflight_rejects_a_wrong_parameter_count() {
    let builder = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("update", "update_export")]),
    );
    let requirements = builder
        .requirements(&[("@package.moth", IMPORTING_FACADE_SOURCE)])
        .expect("registered host imports should produce requirements");

    let wrong_arity = HostImplementation::new(
        HostBindingIdentity::new("@platform", ["has_prefix"]),
        HostSignature::new()
            .parameter(HostType::String)
            .returns(HostType::Bool),
    );
    let errors = preflight(&requirements, &[wrong_arity, log_implementation()])
        .expect_err("a parameter count mismatch should be rejected");

    assert_eq!(
        errors.errors(),
        [PreflightError::ParameterCountMismatch {
            import: HostBindingIdentity::new("@platform", ["has_prefix"]),
            required: 2,
            declared: 1,
        }]
    );
}

#[test]
fn preflight_rejects_a_wrong_return_channel() {
    let builder = RequirementBuilder::new(
        definition(&[fallible_platform_package()]),
        selection(&[("update", "update_export")]),
    );
    let requirements = builder
        .requirements(&[("@package.moth", FALLIBLE_IMPORT_FACADE_SOURCE)])
        .expect("registered host imports should produce requirements");

    assert!(
        requirements.imports()[1].signature().is_fallible(),
        "the registered load binding declares the builtin Error! channel"
    );
    assert_eq!(
        preflight(
            &requirements,
            &[has_prefix_implementation(), load_implementation()]
        ),
        Ok(())
    );

    let infallible_load = HostImplementation::new(
        HostBindingIdentity::new("@platform", ["load"]),
        HostSignature::new().returns(HostType::I32),
    );
    let errors = preflight(
        &requirements,
        &[has_prefix_implementation(), infallible_load],
    )
    .expect_err("an error-channel mismatch should be rejected");

    assert_eq!(
        errors.errors(),
        [PreflightError::ErrorChannelMismatch {
            import: HostBindingIdentity::new("@platform", ["load"]),
            required: true,
            declared: false,
        }]
    );
}

#[test]
fn an_explicit_export_appears_and_unselected_declarations_do_not() {
    let builder = RequirementBuilder::new(definition(&[]), selection(&[("update", "game_update")]));

    let requirements = builder
        .requirements(&[("@package.moth", SELECTION_FACADE_SOURCE)])
        .expect("a concrete export selection should produce requirements");

    assert!(requirements.imports().is_empty());
    assert_eq!(
        requirements.exports(),
        [HostExportRequirement::new(
            "update",
            "game_update",
            HostSignature::new().returns(HostType::I32),
        )]
    );
}

#[test]
fn a_selected_export_carries_ordered_success_values_and_the_error_channel() {
    let builder = RequirementBuilder::new(definition(&[]), selection(&[("pair", "pair_export")]));

    let requirements = builder
        .requirements(&[("@package.moth", MULTI_SLOT_FACADE_SOURCE)])
        .expect("several ordered success values are expressible on an export");

    assert_eq!(
        requirements.exports(),
        [HostExportRequirement::new(
            "pair",
            "pair_export",
            HostSignature::new()
                .parameter(HostType::I32)
                .parameter(HostType::Bool)
                .returns(HostType::I32)
                .returns(HostType::String)
                .fallible(),
        )]
    );
}

#[test]
fn a_selection_naming_a_missing_export_is_a_build_error() {
    let builder = RequirementBuilder::new(definition(&[]), selection(&[("absent", "host_name")]));

    let error = build_error(builder.requirements(&[("@package.moth", SELECTION_FACADE_SOURCE)]));
    let message = error.to_string();

    assert!(
        message.contains("'absent'")
            && message.contains("'host_name'")
            && message.contains("exports no declaration with that name"),
        "the error must name the missing facade export rather than only the host export name: \
         {message}"
    );
}

#[test]
fn a_selection_naming_a_non_function_is_a_build_error() {
    let builder = RequirementBuilder::new(definition(&[]), selection(&[("label", "label_export")]));

    let error = build_error(builder.requirements(&[("@package.moth", SELECTION_FACADE_SOURCE)]));

    assert!(error.to_string().contains("label_export"), "{error}");
    assert!(error.to_string().contains("constant"), "{error}");
}

#[test]
fn a_selection_naming_a_generic_function_is_a_build_error() {
    let builder = RequirementBuilder::new(
        definition(&[]),
        selection(&[("identity", "identity_export")]),
    );

    let error = build_error(builder.requirements(&[("@package.moth", REJECTION_FACADE_SOURCE)]));

    assert!(error.to_string().contains("identity_export"), "{error}");
    assert!(error.to_string().contains("generic"), "{error}");
}

#[test]
fn a_selection_naming_a_compile_time_function_is_a_build_error() {
    const CONST_FACADE_SOURCE: &str = "\
$export
tally #|| -> I32:
    return 1
;
";
    let builder = RequirementBuilder::new(definition(&[]), selection(&[("tally", "tally_export")]));

    let error = build_error(builder.requirements(&[("@package.moth", CONST_FACADE_SOURCE)]));

    assert!(error.to_string().contains("tally_export"), "{error}");
    assert!(error.to_string().contains("compile-time"), "{error}");
}

#[test]
fn a_selection_naming_a_custom_error_channel_is_a_build_error() {
    let builder = RequirementBuilder::new(definition(&[]), selection(&[("risky", "risky_export")]));

    let error = build_error(builder.requirements(&[("@package.moth", REJECTION_FACADE_SOURCE)]));

    assert!(error.to_string().contains("risky_export"), "{error}");
    assert!(error.to_string().contains("Error!"), "{error}");
}

#[test]
fn a_selection_naming_an_unsupported_parameter_or_success_type_is_a_build_error() {
    let parameter_builder = RequirementBuilder::new(
        definition(&[]),
        selection(&[("take_point", "take_point_export")]),
    );
    let parameter_error =
        build_error(parameter_builder.requirements(&[("@package.moth", REJECTION_FACADE_SOURCE)]));
    assert!(
        parameter_error.to_string().contains("take_point_export"),
        "{parameter_error}"
    );

    let success_builder = RequirementBuilder::new(
        definition(&[]),
        selection(&[("make_point", "make_point_export")]),
    );
    let success_error =
        build_error(success_builder.requirements(&[("@package.moth", REJECTION_FACADE_SOURCE)]));
    assert!(
        success_error.to_string().contains("make_point_export"),
        "{success_error}"
    );
}

#[test]
fn an_export_with_a_wire_parameter_is_a_build_error() {
    let builder =
        RequirementBuilder::new(definition(&[]), selection(&[("update", "update_export")]));

    let error =
        build_error(builder.requirements(&[("@package.moth", WIRE_PARAMETER_FACADE_SOURCE)]));

    assert!(
        error
            .to_string()
            .contains("parameter 0 is a wire, which the V1 host ABI cannot carry"),
        "{error}"
    );
}

#[test]
fn an_export_with_a_route_parameter_is_a_build_error() {
    let builder =
        RequirementBuilder::new(definition(&[]), selection(&[("update", "update_export")]));

    let error =
        build_error(builder.requirements(&[("@package.moth", ROUTE_PARAMETER_FACADE_SOURCE)]));

    assert!(
        error.to_string().contains(
            "parameter 0 is a routing constructor parameter, which the V1 host ABI cannot carry"
        ),
        "{error}"
    );
}

#[test]
fn an_export_with_a_mutable_parameter_is_a_build_error() {
    let builder =
        RequirementBuilder::new(definition(&[]), selection(&[("update", "update_export")]));

    let error =
        build_error(builder.requirements(&[("@package.moth", MUTABLE_PARAMETER_FACADE_SOURCE)]));

    assert!(
        error.to_string().contains(
            "parameter 0 takes a mutable value, but the V1 host ABI passes shared reads only"
        ),
        "{error}"
    );
}

#[test]
fn canonical_identity_and_order_do_not_depend_on_declaration_order() {
    let alpha_package = HostPackage::new("@alpha").function(HostFunction::new("ping"));
    let shuffled_package = HostPackage::new("@platform")
        .function(HostFunction::new("log").parameter(HostType::String))
        .function(
            HostFunction::new("has_prefix")
                .parameter(HostType::String)
                .parameter(HostType::String)
                .returns(HostType::Bool),
        );
    let first_builder = RequirementBuilder::new(
        definition(&[platform_package(), alpha_package.clone()]),
        selection(&[("update", "update_export")]),
    );
    let second_builder = RequirementBuilder::new(
        definition(&[alpha_package, shuffled_package]),
        selection(&[("update", "update_export")]),
    );

    const FIRST_FACADE: &str = "\
@platform has_prefix, log
@alpha ping

$export
update |text String| -> Bool:
    log(text)
    ping()
    return has_prefix(text, \"pre\")
;
";
    const SECOND_FACADE: &str = "\
@alpha ping
@platform log, has_prefix

$export
update |text String| -> Bool:
    ping()
    matched = has_prefix(text, \"pre\")
    log(text)
    return matched
;
";

    let first = first_builder
        .requirements(&[("@package.moth", FIRST_FACADE)])
        .expect("the first project should produce requirements");
    let second = second_builder
        .requirements(&[("@package.moth", SECOND_FACADE)])
        .expect("the second project should produce requirements");

    assert_eq!(first, second);
    assert_eq!(
        import_identities(&first),
        ["@alpha#ping", "@platform#has_prefix", "@platform#log"],
        "canonical identity orders imports across host packages whatever their registration order"
    );
}

#[test]
fn an_unreachable_registered_host_function_is_absent() {
    let package = platform_package().function(
        HostFunction::new("unused")
            .parameter(HostType::I64)
            .returns(HostType::String),
    );
    let builder = RequirementBuilder::new(
        definition(&[package]),
        selection(&[("update", "update_export")]),
    );

    let requirements = builder
        .requirements(&[("@package.moth", IMPORTING_FACADE_SOURCE)])
        .expect("registered host imports should produce requirements");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix", "@platform#log"]
    );
}

#[test]
fn capability_isolation_names_only_the_definitions_own_host_packages() {
    let alpha_package = HostPackage::new("@alpha").function(HostFunction::new("alpha_only"));
    let beta_package = HostPackage::new("@beta").function(HostFunction::new("beta_only"));
    let alpha_builder = RequirementBuilder::new(
        definition(&[alpha_package]),
        selection(&[("update", "update_export")]),
    );
    let beta_builder = RequirementBuilder::new(
        definition(&[beta_package]),
        selection(&[("update", "update_export")]),
    );

    let alpha_requirements = alpha_builder
        .requirements(&[(
            "@package.moth",
            "@alpha alpha_only\n\n$export\nupdate ||:\n    alpha_only()\n;\n",
        )])
        .expect("the alpha project should compile against its own definition");
    let beta_requirements = beta_builder
        .requirements(&[(
            "@package.moth",
            "@beta beta_only\n\n$export\nupdate ||:\n    beta_only()\n;\n",
        )])
        .expect("the beta project should compile against its own definition");

    assert_eq!(
        import_identities(&alpha_requirements),
        ["@alpha#alpha_only"]
    );
    assert_eq!(import_identities(&beta_requirements), ["@beta#beta_only"]);
}

#[test]
fn an_optional_host_type_crosses_both_conversion_directions() {
    let package = HostPackage::new("@platform").function(
        HostFunction::new("find")
            .parameter(HostType::optional(HostType::String))
            .returns(HostType::optional(HostType::U32)),
    );
    let builder = RequirementBuilder::new(
        definition(&[package]),
        selection(&[("lookup", "lookup_export")]),
    );

    let requirements = builder
        .requirements(&[(
            "@package.moth",
            "@platform find\n\n$export\nlookup |name String| -> U32?:\n    return find(name)\n;\n",
        )])
        .expect("the registered and public optional types should both convert");

    assert_eq!(
        requirements.imports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::optional(HostType::String))
            .returns(HostType::optional(HostType::U32))
    );
    assert_eq!(
        requirements.exports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::String)
            .returns(HostType::optional(HostType::U32))
    );
}

#[test]
fn a_host_import_reached_through_a_linked_source_package_module_is_included() {
    let package_dir = tempfile::tempdir().expect("should create package directory");
    fs::write(
        package_dir.path().join("@package.moth"),
        "@platform has_prefix\n\n$export\nscore || -> Bool:\n    return has_prefix(\"pre\", \"prefix\")\n;\n",
    )
    .expect("should write the source package module");
    let builder = RequirementBuilder::new(
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@game").from_dir(package_dir.path()))
            .host_package(platform_package())
            .finish()
            .expect("the source and host package definition should finish"),
        selection(&[("update", "update_export")]),
    );

    let requirements = builder
        .requirements(&[(
            "@package.moth",
            "@game score\n\n$export\nupdate || -> Bool:\n    return score()\n;\n",
        )])
        .expect("a linked source-package module may reach a host binding");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix"],
        "reachable external calls of every linked module are resolved through that module's registry"
    );
}

#[test]
fn a_reachable_core_binding_is_not_a_native_host_import() {
    let builder = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("update", "update_export")]),
    );

    let requirements = builder
        .requirements(&[("@package.moth", CORE_BINDING_FACADE_SOURCE)])
        .expect("a facade may call compiler Core bindings beside a host binding");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix"],
        "a Core binding is compiler-owned and is not a registered native host import"
    );
}

#[test]
fn an_empty_selection_requires_no_host_imports() {
    let builder = RequirementBuilder::new(definition(&[platform_package()]), selection(&[]));

    let requirements = builder
        .requirements(&[("@package.moth", IMPORTING_FACADE_SOURCE)])
        .expect("an empty selection still produces requirements");

    assert!(
        requirements.imports().is_empty(),
        "an artefact with no selected export executes no facade function, so it reaches no host binding"
    );
    assert!(requirements.exports().is_empty());
}

#[test]
fn a_host_binding_is_required_only_once_a_calling_export_is_selected() {
    const TWO_EXPORTS: &str = "\
@platform has_prefix, log

$export
selected |text String| -> Bool:
    return has_prefix(text, \"pre\")
;

$export
unselected |text String|:
    log(text)
;
";
    let only_selected = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("selected", "selected_export")]),
    )
    .requirements(&[("@package.moth", TWO_EXPORTS)])
    .expect("the selected export reaches its own host binding");
    assert_eq!(
        import_identities(&only_selected),
        ["@platform#has_prefix"],
        "a binding only an unselected public function reaches is not required"
    );

    let both_selected = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[
            ("selected", "selected_export"),
            ("unselected", "unselected_export"),
        ]),
    )
    .requirements(&[("@package.moth", TWO_EXPORTS)])
    .expect("both selected exports reach their host bindings");
    assert_eq!(
        import_identities(&both_selected),
        ["@platform#has_prefix", "@platform#log"],
        "selecting the second export adds exactly the binding it reaches"
    );
}

#[test]
fn a_binding_reached_from_two_selected_exports_appears_once() {
    const SHARED_BINDING: &str = "\
@platform has_prefix

$export
first |text String| -> Bool:
    return has_prefix(text, \"a\")
;

$export
second |text String| -> Bool:
    return has_prefix(text, \"b\")
;
";
    let requirements = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("first", "first_export"), ("second", "second_export")]),
    )
    .requirements(&[("@package.moth", SHARED_BINDING)])
    .expect("both selected exports reach the same binding");

    assert_eq!(import_identities(&requirements), ["@platform#has_prefix"]);
    assert_eq!(
        requirements.imports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::String)
            .parameter(HostType::String)
            .returns(HostType::Bool)
    );
}

#[test]
fn a_binding_reached_from_the_facade_and_a_linked_module_appears_once() {
    let package_dir = tempfile::tempdir().expect("should create package directory");
    fs::write(
        package_dir.path().join("@package.moth"),
        "@platform has_prefix\n\n$export\nscore || -> Bool:\n    return has_prefix(\"pre\", \"prefix\")\n;\n",
    )
    .expect("should write the source package module");
    let builder = RequirementBuilder::new(
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@game").from_dir(package_dir.path()))
            .host_package(platform_package())
            .finish()
            .expect("the source and host package definition should finish"),
        selection(&[("update", "update_export")]),
    );

    let requirements = builder
        .requirements(&[(
            "@package.moth",
            "@game score\n@platform has_prefix\n\n$export\nupdate |text String| -> Bool:\n    if has_prefix(text, \"pre\"):\n        return score()\n    ;\n    return false\n;\n",
        )])
        .expect("the facade and a linked module may reach the same binding");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix"],
        "the same canonical binding reached from two modules names one import"
    );
    assert_eq!(
        requirements.imports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::String)
            .parameter(HostType::String)
            .returns(HostType::Bool)
    );
}

#[test]
fn a_reexported_facade_function_roots_the_declaration_origin_host_calls() {
    let builder = RequirementBuilder::new(
        definition(&[platform_package()]),
        selection(&[("update", "update_export")]),
    );

    let requirements = builder
        .requirements(&[
            ("api/@mod.moth", REEXPORTED_IMPORT_MODULE_SOURCE),
            ("@package.moth", REEXPORTED_IMPORT_FACADE_SOURCE),
        ])
        .expect("a re-exported facade function should resolve to its declaration origin");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix"],
        "the declaration origin under the facade re-export roots the module's host calls"
    );
    assert_eq!(
        requirements.imports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::String)
            .parameter(HostType::String)
            .returns(HostType::Bool),
        "the required import keeps the registered host signature"
    );
    assert_eq!(
        requirements.exports(),
        [HostExportRequirement::new(
            "update",
            "update_export",
            HostSignature::new()
                .parameter(HostType::String)
                .returns(HostType::Bool),
        )],
        "the re-exported function keeps its public signature and export name"
    );
}

#[test]
fn a_linked_module_call_joins_the_facades_host_imports() {
    let package_dir = tempfile::tempdir().expect("should create package directory");
    fs::write(
        package_dir.path().join("@package.moth"),
        "@platform has_prefix\n\n$export\nscore || -> Bool:\n    return has_prefix(\"pre\", \"prefix\")\n;\n",
    )
    .expect("should write the source package module");
    let builder = RequirementBuilder::new(
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@game").from_dir(package_dir.path()))
            .host_package(platform_package())
            .finish()
            .expect("the source and host package definition should finish"),
        selection(&[("update", "update_export")]),
    );

    // One definition snapshot serves every module of a compilation, so both modules assign the
    // same local IDs. This case proves a linked module's reachable host call joins the facade's
    // imports; registry identity is owned per module but is not observed to differ today.
    let requirements = builder
        .requirements(&[(
            "@package.moth",
            "@game score\n@platform log\n\n$export\nupdate |text String|:\n    log(text)\n    score()\n;\n",
        )])
        .expect("a linked module's host call should join the facade's imports");

    assert_eq!(
        import_identities(&requirements),
        ["@platform#has_prefix", "@platform#log"],
        "both the facade's and the linked module's host calls are required"
    );
}

#[test]
fn a_byte_host_binding_and_export_keep_their_identity() {
    let package = HostPackage::new("@platform").function(
        HostFunction::new("octet")
            .parameter(HostType::Byte)
            .returns(HostType::Byte),
    );
    let builder = RequirementBuilder::new(
        definition(&[package]),
        selection(&[("take_byte", "take_byte_export")]),
    );

    let requirements = builder
        .requirements(&[(
            "@package.moth",
            "@platform octet\n\n$export\ntake_byte |value Byte| -> Byte:\n    return octet(value)\n;\n",
        )])
        .expect("Byte stays Byte through registration, requirement production and export");

    assert_eq!(
        requirements.imports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::Byte)
            .returns(HostType::Byte)
    );
    assert_eq!(
        requirements.exports()[0].signature(),
        &HostSignature::new()
            .parameter(HostType::Byte)
            .returns(HostType::Byte)
    );
}
