//! Proves a separate crate can describe a builder through public SDK names only.

use moth::builder::{
    BuildArtifacts, BuildContext, BuildProfile, BuildRequest, BuilderDefinition,
    BuilderDefinitionDraft, CompiledProject, Error, ErrorKind, HostPackage, ProjectBuilder,
    Settings, SourcePackage, build_project,
};
use moth::host::{HostFunction, HostType};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};

/// The smallest page that compiles without diagnostics.
const PAGE_SOURCE: &str = "[:<h1>About</h1>]\n";

/// A record literal that is never closed, so the frontend rejects the project before any builder
/// runs.
const UNTERMINATED_RECORD_PAGE_SOURCE: &str = "stats #= (a = 1\n";

/// Repeated literal patterns that make later arms unreachable. The frontend reports three warnings
/// and the project still builds.
const DUPLICATE_LITERAL_PAGE_SOURCE: &str = r#"value ~= "hello"
result ~= "unset"

if value is:
    "hello" => ~result = "one"
    "hello" => ~result = "two"
    "hello" => ~result = "three"
    "hello" => ~result = "four"
    else => ~result = "other"
;

~[:pattern_unreachable_after_duplicate_literal_warning result=[result]]
"#;

/// Imports `greet` from the `@ui` package that only one test builder registers.
const UI_IMPORT_PAGE_SOURCE: &str = "@ui greet\nvalue = greet()\n";

/// Imports a shared helper constant and a shared nominal with a receiver method from `@ui`.
const UI_SUPPORT_HELPER_IMPORT_PAGE_SOURCE: &str =
    "@ui value, Message\nmessage = Message(\"hello\")\nio.line([: [value] [message.label()]])\n";

/// A `Byte` value passed to a host function that takes `Byte`.
const HOST_BYTE_PAGE_SOURCE: &str = "@platform takes_byte\noctet Byte = 42\ntakes_byte(octet)\n";

/// A `U8` value passed where the host function takes `Byte`.
const HOST_U8_PAGE_SOURCE: &str = "@platform takes_byte\noctet U8 = 42\ntakes_byte(octet)\n";

#[test]
fn external_crate_defines_source_and_host_packages() {
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    let definition = BuilderDefinition::new("game")
        .source_package(SourcePackage::new("@ui").from_dir(ui_dir.path()))
        .host_package(
            HostPackage::new("@platform").function(
                HostFunction::new("has_prefix")
                    .parameter(HostType::String)
                    .parameter(HostType::String)
                    .returns(HostType::Bool),
            ),
        )
        .markdown()
        .finish()
        .expect("public definition should finish without the standard template prelude");

    assert_eq!(definition.identity(), "game");
}

#[test]
fn external_crate_cannot_replace_official_core_source_packages() {
    let package_dir = tempfile::tempdir().expect("should create package directory");

    for (case, path) in [
        ("html", "@html"),
        ("html nested", "@html/docs"),
        ("styles", "@styles"),
        ("styles nested", "@styles/docs"),
    ] {
        let error = BuilderDefinition::new("game")
            .source_package(SourcePackage::new(path).from_dir(package_dir.path()))
            .finish()
            .expect_err("SDK builders cannot replace an official Core source package");
        assert_eq!(error.kind(), ErrorKind::InvalidDefinition, "{case}");
        assert!(
            error
                .to_string()
                .contains("overlaps registered source package"),
            "{case}: {error}"
        );
    }
}

#[test]
fn external_crate_receives_typed_definition_errors() {
    let draft: BuilderDefinitionDraft =
        BuilderDefinition::new("game").host_package(HostPackage::new("@core/game"));
    let error: Error = draft
        .finish()
        .expect_err("builders cannot claim the Core namespace");

    assert_eq!(error.kind(), ErrorKind::InvalidDefinition);
    assert!(error.to_string().contains("@core/game"), "{error}");
}

/// Accepts any project, records that `build` ran and requires the package facade.
struct PackageFacadeBuilder {
    build_ran: Cell<bool>,
}

impl ProjectBuilder for PackageFacadeBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.build_ran.set(true);
        project.require_package()?;
        Ok(BuildArtifacts::new())
    }
}

/// Writes the project config shared by the SDK fixtures.
fn write_config(project_root: &Path) {
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\n",
    )
    .expect("should write config");
}

/// Writes the project-root `@package.moth` facade.
fn write_package_facade(project_root: &Path) {
    fs::write(
        project_root.join("@package.moth"),
        "$export\nlabel #= \"facade\"\n",
    )
    .expect("should write package facade");
}

/// Writes a one-page project whose page holds `page_source` and, when requested, its
/// `@package.moth` facade beside `@config.moth`.
fn write_project(project_root: &Path, page_source: &str, with_facade: bool) {
    write_config(project_root);
    fs::write(project_root.join("@entry.moth"), page_source).expect("should write page");
    if with_facade {
        write_package_facade(project_root);
    }
}

/// Records the profile observed when the public driver reaches `build`.
///
/// WHAT: stores the profile provided through `BuildContext`.
/// WHY: the external SDK tests verify request selection crosses the build adapter.
struct ProfileRecordingBuilder {
    observed_profile: Cell<Option<BuildProfile>>,
}

impl ProjectBuilder for ProfileRecordingBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.observed_profile.set(Some(context.profile()));
        Ok(BuildArtifacts::new())
    }
}

#[test]
fn external_builder_observes_requested_release_profile() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ProfileRecordingBuilder {
        observed_profile: Cell::new(None),
    };

    let request = BuildRequest::new(project_dir.path()).profile(BuildProfile::Release);
    build_project(&builder, request).expect("release profile project should build");

    assert_eq!(builder.observed_profile.get(), Some(BuildProfile::Release));
}

#[test]
fn external_builder_observes_default_dev_profile() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ProfileRecordingBuilder {
        observed_profile: Cell::new(None),
    };

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("default profile project should build");

    assert_eq!(builder.observed_profile.get(), Some(BuildProfile::Dev));
}

#[test]
fn external_builder_receives_package_facade() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, true);
    let builder = PackageFacadeBuilder {
        build_ran: Cell::new(false),
    };

    let built = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("project with a package facade should build");

    assert!(
        builder.build_ran.get(),
        "a package facade project must reach ProjectBuilder::build"
    );
    assert!(
        built
            .into_warnings()
            .expect("warning finalisation")
            .is_empty()
    );
}

#[test]
fn external_builder_builds_facade_only_project_without_entry() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_config(project_dir.path());
    write_package_facade(project_dir.path());
    let builder = PackageFacadeBuilder {
        build_ran: Cell::new(false),
    };

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("a project with only a package facade should build through the SDK");

    assert!(
        builder.build_ran.get(),
        "an entryless package build must reach ProjectBuilder::build"
    );
}

#[test]
fn external_builder_without_facade_returns_build_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_config(project_dir.path());
    let builder = PackageFacadeBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("project without a package facade should fail the build");

    assert!(
        builder.build_ran.get(),
        "a project without a facade or entry must still reach ProjectBuilder::build"
    );
    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains("@package.moth"), "{error}");
}

#[test]
fn external_builder_requires_a_package_facade_for_entry_projects() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = PackageFacadeBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("an entry-only project should fail require_package");

    assert!(
        builder.build_ran.get(),
        "an entry project without a facade must still reach ProjectBuilder::build"
    );
    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains("@package.moth"), "{error}");
}

#[test]
fn entryless_package_facade_source_error_returns_compilation_error_without_building() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_config(project_dir.path());
    fs::write(
        project_dir.path().join("@package.moth"),
        "stats #= (a = 1\n",
    )
    .expect("should write the failing package facade");
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a source error in an entryless package facade should fail compilation");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        !builder.build_ran.get(),
        "the builder must not run when compilation fails"
    );
    let rendered_lines = error.messages().join("\n");
    assert!(
        rendered_lines.contains("@package.moth"),
        "the failure must come from the compiled package facade: {rendered_lines}"
    );
    assert!(
        !rendered_lines.contains("No executable entry root"),
        "the facade must be compiled instead of rejected for lacking an entry: {rendered_lines}"
    );
}

/// Records whether `build` ran, so a compilation failure can prove it never did.
struct RecordingBuildBuilder {
    build_ran: Cell<bool>,
}

impl ProjectBuilder for RecordingBuildBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.build_ran.set(true);
        Ok(BuildArtifacts::new())
    }
}

/// Registers the `@ui` source package installed in `ui_root`.
struct UiPackageBuilder {
    ui_root: PathBuf,
}

impl ProjectBuilder for UiPackageBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@ui").from_dir(&self.ui_root))
            .finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        Ok(BuildArtifacts::new())
    }
}

/// Registers no packages, so a project importing `@ui` cannot resolve it.
struct NoPackageBuilder;

impl ProjectBuilder for NoPackageBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        Ok(BuildArtifacts::new())
    }
}

/// Imports official Core source packages without any builder-selected web package.
const CORE_SOURCE_IMPORT_PAGE: &str = "@html p\n@styles center\nvalue = [p, center: Hello]\n";

#[test]
fn minimal_sdk_builder_imports_official_html_and_styles_without_web_packages() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), CORE_SOURCE_IMPORT_PAGE, false);
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("official @html and @styles must import without @web/canvas");
    assert!(
        builder.build_ran.get(),
        "compilation of the official Core imports must reach the builder"
    );
}

/// A page holding an explicit `.mtf` file value and a qualified `content` constant reference.
const MTF_FILE_VALUE_PAGE: &str =
    "intro #= @intro.mtf\nbody = @intro content\n~[: [intro] [body] ]\n";

/// A template body that imports a nested `.mtf` and uses a bare `@html` prelude name.
const INTRO_TEMPLATE_SOURCE: &str = "[: intro [p: nested [@nested.mtf] ] ]\n";

/// The nested template body reached from `intro.mtf`.
const NESTED_TEMPLATE_SOURCE: &str = "nested template body\n";

/// Proves the public SDK build path compiles `.mtf` content when the builder selects no optional
/// source kinds. The exact folded text is asserted by the crate-internal
/// `compiler_supported_mtf_file_value_and_qualified_content_fold_identically` test, which reads
/// the published constant values.
#[test]
fn minimal_sdk_builder_compiles_mtf_file_values_without_kind_selection() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), MTF_FILE_VALUE_PAGE, false);
    fs::write(project_dir.path().join("intro.mtf"), INTRO_TEMPLATE_SOURCE)
        .expect("should write intro template");
    fs::write(
        project_dir.path().join("nested.mtf"),
        NESTED_TEMPLATE_SOURCE,
    )
    .expect("should write nested template");
    // The minimal SDK builder selects no optional source kinds, so this project proves `.mtf`
    // support is compiler-owned rather than builder-registered.
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    let built = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("`.mtf` file values must compile under a builder that selects no source kinds");
    assert!(
        builder.build_ran.get(),
        "a successful template compilation must reach the builder"
    );
    let warnings = built
        .into_warnings()
        .expect("warning finalisation should succeed");
    assert!(
        warnings.is_empty(),
        "the nested template fixture should compile without warnings: {warnings:?}"
    );
}

/// Exposes one host function that takes a `Byte`.
struct ByteParameterBuilder;

impl ProjectBuilder for ByteParameterBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game")
            .host_package(
                HostPackage::new("@platform")
                    .function(HostFunction::new("takes_byte").parameter(HostType::Byte)),
            )
            .finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        Ok(BuildArtifacts::new())
    }
}

/// Writes a paired `@ui` source package whose external surface is its support root.
fn write_ui_paired_package(package_root: &Path) {
    fs::write(
        package_root.join("@mod.moth"),
        "$export\nnormal_value #= \"normal\"\n",
    )
    .expect("should write normal package root");
    fs::write(
        package_root.join("@package.moth"),
        "$export\n@impl greeting\n",
    )
    .expect("should write support package facade");
    fs::write(package_root.join("impl.moth"), "greeting #= \"support\"\n")
        .expect("should write shared package helper");
}

/// Writes a support-only `@ui` package root that re-exports shared helper declarations.
fn write_ui_support_package(package_root: &Path) {
    fs::write(
        package_root.join("@package.moth"),
        "$export\n@impl value, Message\n",
    )
    .expect("should write support package facade");
    fs::write(
        package_root.join("impl.moth"),
        "value #= 7\n\nMessage = | text String |\n\nlabel |this Message| -> String:\n    return this.text\n;\n",
    )
    .expect("should write shared package helpers");
}

/// Writes a `@ui` package root whose only module publishes `greet` through its support facade.
fn write_ui_package(package_root: &Path) {
    fs::write(
        package_root.join("@package.moth"),
        "$export\ngreet || -> I32:\n    return 1\n;\n",
    )
    .expect("should write package module");
}

/// Writes a `@ui` package root that publishes only a normal `@mod.moth` facade.
///
/// A registered Moth-source package must publish its external API through `@package.moth`, so the
/// normal root alone is insufficient even though ordinary modules may use it.
fn write_ui_normal_only_package(package_root: &Path) {
    fs::write(
        package_root.join("@mod.moth"),
        "$export\ngreet || -> I32:\n    return 1\n;\n",
    )
    .expect("should write normal-only package root");
}

#[test]
fn invalid_source_returns_compilation_error_without_building() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), UNTERMINATED_RECORD_PAGE_SOURCE, false);
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("source with diagnostics should fail the build");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        !builder.build_ran.get(),
        "the builder must not run when compilation fails"
    );
    let rendered_lines = error.messages().join("\n");
    assert!(rendered_lines.contains("@entry.moth"), "{rendered_lines}");
}

#[test]
fn independent_definitions_do_not_share_source_packages() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), UI_IMPORT_PAGE_SOURCE, false);
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    write_ui_package(ui_dir.path());
    let ui_builder = UiPackageBuilder {
        ui_root: ui_dir.path().to_path_buf(),
    };
    let no_package_builder = NoPackageBuilder;

    build_project(&ui_builder, BuildRequest::new(project_dir.path()))
        .expect("the registered @ui package should satisfy the import");
    let error = build_project(&no_package_builder, BuildRequest::new(project_dir.path()))
        .expect_err("a builder without @ui must not resolve the import");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    build_project(&ui_builder, BuildRequest::new(project_dir.path()))
        .expect("the first builder should still resolve @ui after the failing build");
}

#[test]
fn builder_registered_support_only_source_package_publishes_its_support_interface() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(
        project_dir.path(),
        UI_SUPPORT_HELPER_IMPORT_PAGE_SOURCE,
        false,
    );
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    write_ui_support_package(ui_dir.path());
    let builder = UiPackageBuilder {
        ui_root: ui_dir.path().to_path_buf(),
    };

    build_project(&builder, BuildRequest::new(project_dir.path())).expect(
        "a builder-registered support-only package should publish its support interface and import",
    );
}

#[test]
fn builder_registered_paired_source_package_publishes_its_support_interface() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(
        project_dir.path(),
        "@ui greeting\nio.line([: [greeting]])\n",
        false,
    );
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    write_ui_paired_package(ui_dir.path());
    let builder = UiPackageBuilder {
        ui_root: ui_dir.path().to_path_buf(),
    };

    build_project(&builder, BuildRequest::new(project_dir.path())).expect(
        "external consumers must receive the paired package support interface, not its normal root",
    );
}

#[test]
fn builder_registered_normal_only_source_package_is_rejected() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), UI_IMPORT_PAGE_SOURCE, false);
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    write_ui_normal_only_package(ui_dir.path());
    let builder = UiPackageBuilder {
        ui_root: ui_dir.path().to_path_buf(),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a registered package with only @mod.moth must not publish a package surface");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    let messages = error.messages().join("\n");
    assert!(
        messages.contains("@package.moth"),
        "the missing-facade diagnostic must name @package.moth: {messages}"
    );
}

#[test]
fn builder_registered_paired_source_package_does_not_expose_its_normal_root() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(
        project_dir.path(),
        "@ui normal_value\nvalue = normal_value\n",
        false,
    );
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    write_ui_paired_package(ui_dir.path());
    let builder = UiPackageBuilder {
        ui_root: ui_dir.path().to_path_buf(),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("external consumers must never observe a paired package's normal root");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    let messages = error.messages().join("\n");
    assert!(
        messages.contains("normal_value"),
        "the diagnostic must name the unexposed declaration: {messages}"
    );
}

#[test]
fn project_local_support_folder_is_visible_without_registration() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(
        project_dir.path(),
        "@markup greeting\nvalue = greeting()\n",
        false,
    );
    // A project copying a package folder gains ordinary project-local support visibility. It is
    // not registered as a Builder or Core package and needs no builder definition entry.
    let markup_root = project_dir.path().join("markup");
    fs::create_dir_all(&markup_root).expect("should create copied support folder");
    fs::write(
        markup_root.join("@package.moth"),
        "$export\n@impl greeting\n",
    )
    .expect("should write copied support facade");
    fs::write(
        markup_root.join("impl.moth"),
        "greeting || -> String:\n    return \"local\"\n;\n",
    )
    .expect("should write copied support helper");

    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };
    build_project(&builder, BuildRequest::new(project_dir.path())).expect(
        "a copied project-local support folder must resolve as an ordinary support package",
    );
    assert!(
        builder.build_ran.get(),
        "a project-local support import must not prevent the build step"
    );
}

#[test]
fn registered_package_private_helper_stays_private() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), "@ui hidden\nvalue = hidden\n", false);
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    fs::write(
        ui_dir.path().join("@package.moth"),
        "$export\n@impl greeting\n",
    )
    .expect("should write support facade");
    fs::write(
        ui_dir.path().join("impl.moth"),
        "hidden #= \"private\"\n\ngreeting #= hidden\n",
    )
    .expect("should write referenced private helper");
    let builder = UiPackageBuilder {
        ui_root: ui_dir.path().to_path_buf(),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a package's private helper must not join its public facade");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    let messages = error.messages().join("\n");
    assert!(
        messages.contains("hidden"),
        "the unexported-helper diagnostic must name the private declaration: {messages}"
    );
}

#[test]
fn project_local_support_folder_private_helper_stays_private() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(
        project_dir.path(),
        "@markup hidden\nvalue = hidden\n",
        false,
    );
    let markup_root = project_dir.path().join("markup");
    fs::create_dir_all(&markup_root).expect("should create copied support folder");
    fs::write(
        markup_root.join("@package.moth"),
        "$export\n@impl greeting\n",
    )
    .expect("should write copied support facade");
    fs::write(
        markup_root.join("impl.moth"),
        "hidden #= \"private\"\n\ngreeting #= hidden\n",
    )
    .expect("should write referenced copied private helper");
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a copied support folder must not expose private helpers");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    let messages = error.messages().join("\n");
    assert!(
        messages.contains("hidden"),
        "the unexported-helper diagnostic must name the private declaration: {messages}"
    );
}

#[test]
fn host_byte_parameter_accepts_byte_and_rejects_u8() {
    let byte_dir = tempfile::tempdir().expect("should create project directory");
    write_project(byte_dir.path(), HOST_BYTE_PAGE_SOURCE, false);
    let builder = ByteParameterBuilder;

    build_project(&builder, BuildRequest::new(byte_dir.path()))
        .expect("a Byte value should satisfy a Byte parameter");

    let u8_dir = tempfile::tempdir().expect("should create project directory");
    write_project(u8_dir.path(), HOST_U8_PAGE_SOURCE, false);

    let error = build_project(&builder, BuildRequest::new(u8_dir.path()))
        .expect_err("a U8 value must not convert to Byte");

    assert_eq!(error.kind(), ErrorKind::Compilation);
}

/// The values one builder reads through `CompiledProject::settings`.
#[derive(Debug, Default, PartialEq, Eq)]
struct ObservedSettings {
    asset_root: Option<String>,
    debug_overlay: Option<bool>,
    dev_output: Option<String>,
    release_output: Option<String>,
    asset_root_as_bool: Option<bool>,
    debug_overlay_as_string: Option<String>,
}

/// Reads the declared section fields and the reserved output roots through one settings view.
///
/// The wrong-accessor reads pin that a declared field is only visible through its declared type.
fn observe_settings(project: &CompiledProject) -> ObservedSettings {
    let settings: Settings<'_> = project.settings();
    ObservedSettings {
        asset_root: settings.string("asset_root").map(str::to_owned),
        debug_overlay: settings.bool("debug_overlay"),
        dev_output: settings.string("dev_output").map(str::to_owned),
        release_output: settings.string("release_output").map(str::to_owned),
        asset_root_as_bool: settings.bool("asset_root"),
        debug_overlay_as_string: settings.string("debug_overlay").map(str::to_owned),
    }
}

/// Declares the `game` section and records what `build` reads from it.
#[derive(Default)]
struct SettingsRecordingBuilder {
    observed: RefCell<Option<ObservedSettings>>,
}

impl ProjectBuilder for SettingsRecordingBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game")
            .config("game", |config| {
                config.string("asset_root").default("assets");
                config.bool("debug_overlay").default(true);
            })
            .finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.observed.replace(Some(observe_settings(project)));
        Ok(BuildArtifacts::new())
    }
}

/// Declares no config section and records what `build` reads.
#[derive(Default)]
struct SectionlessSettingsBuilder {
    observed: RefCell<Option<ObservedSettings>>,
}

impl ProjectBuilder for SectionlessSettingsBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.observed.replace(Some(observe_settings(project)));
        Ok(BuildArtifacts::new())
    }
}

/// Declares a required `game` field and records whether `build` ran.
struct RequiredSectionBuilder {
    build_ran: Cell<bool>,
}

impl ProjectBuilder for RequiredSectionBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game")
            .config("game", |config| {
                config.string("mode").required();
            })
            .finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.build_ran.set(true);
        Ok(BuildArtifacts::new())
    }
}

/// Writes a one-page project whose config authors the builder section `section_name`.
fn write_project_with_builder_section(project_root: &Path, section_name: &str, fields: &str) {
    write_project_with_builder_section_and_source(project_root, section_name, fields, PAGE_SOURCE);
}

fn write_project_with_builder_section_and_source(
    project_root: &Path,
    section_name: &str,
    fields: &str,
    page_source: &str,
) {
    fs::write(
        project_root.join("@config.moth"),
        format!("project #= (\n    name = \"docs\",\n)\n{section_name} #= (\n{fields})\n"),
    )
    .expect("should write config");
    fs::write(project_root.join("@entry.moth"), page_source).expect("should write page");
}

#[test]
fn builder_section_defaults_apply_when_the_project_omits_the_section() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = SettingsRecordingBuilder::default();

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("an omitted optional section should build with its declared defaults");

    assert_eq!(
        builder.observed.borrow().as_ref(),
        Some(&ObservedSettings {
            asset_root: Some("assets".to_owned()),
            debug_overlay: Some(true),
            dev_output: None,
            release_output: None,
            asset_root_as_bool: None,
            debug_overlay_as_string: None,
        })
    );
}

#[test]
fn builder_reads_authored_section_values_and_declared_defaults() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(project_dir.path(), "game", "    debug_overlay = false,\n");
    let builder = SettingsRecordingBuilder::default();

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("an authored section should build");

    let observed = builder.observed.borrow();
    let observed = observed
        .as_ref()
        .expect("the builder should record its settings");
    assert_eq!(
        observed.asset_root_as_bool, None,
        "a string field must not read through the bool accessor"
    );
    assert_eq!(
        observed.debug_overlay_as_string, None,
        "a bool field must not read through the string accessor"
    );

    assert_eq!(
        builder.observed.borrow().as_ref(),
        Some(&ObservedSettings {
            asset_root: Some("assets".to_owned()),
            debug_overlay: Some(false),
            dev_output: None,
            release_output: None,
            asset_root_as_bool: None,
            debug_overlay_as_string: None,
        })
    );
}

#[test]
fn builder_section_accepts_the_builder_own_output_roots() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(
        project_dir.path(),
        "game",
        "    dev_output = \"_custom/dev\",\n    release_output = \"_custom/release\",\n    asset_root = \"content\",\n",
    );
    let builder = SettingsRecordingBuilder::default();

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("a section should accept the automatic output roots");

    assert_eq!(
        builder.observed.borrow().as_ref(),
        Some(&ObservedSettings {
            asset_root: Some("content".to_owned()),
            debug_overlay: Some(true),
            dev_output: None,
            release_output: None,
            asset_root_as_bool: None,
            debug_overlay_as_string: None,
        })
    );
}

#[test]
fn builder_without_a_section_reads_no_settings() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = SectionlessSettingsBuilder::default();

    build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("a builder without a section should build");

    assert_eq!(
        builder.observed.borrow().as_ref(),
        Some(&ObservedSettings::default()),
        "a builder without a section has no readable fields"
    );
}

#[test]
fn builder_section_missing_required_field_is_a_compilation_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(project_dir.path(), "game", "");
    let builder = RequiredSectionBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("an authored section without its required field should fail compilation");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        !builder.build_ran.get(),
        "the builder must not run when config validation fails"
    );
    let rendered = error.messages().join("\n");
    assert!(rendered.contains("mode"), "{error}");
    assert!(
        rendered.contains("game section"),
        "the diagnostic must name the authored section: {error}"
    );
}

#[test]
fn omitted_required_builder_section_is_a_compilation_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = RequiredSectionBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a project that omits a required section should fail compilation");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        !builder.build_ran.get(),
        "the builder must not run when config validation fails"
    );
    let rendered = error.messages().join("\n");
    assert!(
        rendered.contains("requires the active builder section 'game'"),
        "{error}"
    );
    assert!(
        !rendered.contains("even when it is empty"),
        "an empty record does not satisfy a section with a required field: {error}"
    );
}

#[test]
fn builder_section_unknown_field_is_a_compilation_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(project_dir.path(), "game", "    font = \"serif\",\n");
    let builder = SettingsRecordingBuilder::default();

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("an undeclared section field should fail compilation");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        builder.observed.borrow().is_none(),
        "the builder must not run when config validation fails"
    );
    let rendered = error.messages().join("\n");
    assert!(rendered.contains("font"), "{error}");
    assert!(
        rendered.contains("game section"),
        "the diagnostic must name the authored section: {error}"
    );
}

#[test]
fn builder_section_wrong_value_type_is_a_compilation_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(project_dir.path(), "game", "    asset_root = 3,\n");
    let builder = SettingsRecordingBuilder::default();

    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a non-string value for a string field should fail compilation");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        builder.observed.borrow().is_none(),
        "the builder must not run when config validation fails"
    );
    assert!(
        error.messages().join("\n").contains("asset_root"),
        "{error}"
    );
}

/// Requests the native Wasm capability, records the failure and publishes a fallback artefact.
struct NativeWasmProbeBuilder {
    observed: RefCell<Option<Error>>,
}

impl ProjectBuilder for NativeWasmProbeBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        let package = project.require_package()?;
        let error = context
            .compile_wasm(package)
            .expect_err("the test builder expected native Wasm compilation to be unavailable");
        self.observed.replace(Some(error));

        // A missing capability is not a failed build, so the builder still publishes its own
        // artefact.
        let mut artifacts = BuildArtifacts::new();
        artifacts.add("game.report", "native wasm unavailable")?;
        Ok(artifacts)
    }
}

#[test]
fn compile_wasm_reports_the_unavailable_capability_and_the_build_still_publishes() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, true);
    let builder = NativeWasmProbeBuilder {
        observed: RefCell::new(None),
    };

    let built = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the project should still build when native Wasm is unavailable");
    built
        .write_outputs()
        .expect("a build that handled the capability error should still publish");

    let error = builder
        .observed
        .borrow()
        .clone()
        .expect("build must reach compile_wasm");
    assert_eq!(error.kind(), ErrorKind::UnsupportedCapability);
    let rendered = error.to_string();
    assert!(
        rendered.contains("Wasm compilation of a project package"),
        "the error must name the missing capability: {rendered}"
    );
    assert!(
        rendered.contains("not available in this compiler yet"),
        "the error must state the capability is unavailable: {rendered}"
    );
}

/// The artefacts one SDK test build records and whether its `build` step fails.
#[derive(Default)]
struct ArtefactPlan {
    artefacts: Vec<(String, Vec<u8>)>,
    fail_build: bool,
}

/// Publishes a test-selected artefact set through `BuildArtifacts`.
struct ArtefactBuilder {
    plan: RefCell<ArtefactPlan>,
}

impl ArtefactBuilder {
    fn new(artefacts: &[(&str, &[u8])]) -> Self {
        let builder = Self {
            plan: RefCell::new(ArtefactPlan::default()),
        };
        builder.set(artefacts);
        builder
    }

    fn set(&self, artefacts: &[(&str, &[u8])]) {
        self.plan.borrow_mut().artefacts = artefacts
            .iter()
            .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
            .collect();
    }

    fn fail_build(&self) {
        self.plan.borrow_mut().fail_build = true;
    }
}

impl ProjectBuilder for ArtefactBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game")
            .config("game", |config| {
                config.string("asset_root").default("assets");
            })
            .finish()
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        let plan = self.plan.borrow();
        if plan.fail_build {
            return Err(Error::build("The test builder was told to fail."));
        }

        let mut artifacts = BuildArtifacts::new();
        for (path, bytes) in &plan.artefacts {
            artifacts.add(path, bytes.clone())?;
        }
        Ok(artifacts)
    }
}

/// The compiler's output manifest filename, observed as an ordinary emitted file by an SDK
/// consumer that has no access to the crate-private constant.
const MANIFEST_FILENAME: &str = ".moth_manifest";

fn read_manifest(output_root: &Path) -> String {
    fs::read_to_string(output_root.join(MANIFEST_FILENAME)).expect("manifest should exist")
}

fn assert_path_missing(path: &Path) {
    assert!(!path.exists(), "{} should not exist", path.display());
}

/// Replaces the project root's authored and canonical spellings with `<project>`.
///
/// Temporary directories can have two spellings for one location: macOS resolves `/var` through
/// `/private/var`, and Windows renders the same path with backslashes. Normalise both spellings
/// and both separators so the diagnostic comparison covers the location and message parts, not
/// the host's spelling. Longer spellings are replaced first so a canonical prefix cannot be
/// shortened into a broken partial match.
fn normalize_project_paths(rendered: &str, project_root: &Path) -> String {
    // Both sides drop Windows' verbatim prefix so a canonical `\\?\C:\...` spelling and an
    // authored `C:\...` spelling compare as the same location.
    let mut normalized = rendered.replace('\\', "/").replace("//?/", "");
    let mut spellings = vec![project_root.to_path_buf()];
    if let Ok(canonical) = fs::canonicalize(project_root) {
        spellings.push(canonical);
    }
    spellings.sort_by_key(|spelling| std::cmp::Reverse(spelling.as_os_str().len()));
    for spelling in spellings {
        let spelling = spelling
            .to_string_lossy()
            .replace('\\', "/")
            .replace("//?/", "");
        normalized = normalized.replace(&spelling, "<project>");
    }
    normalized
}

/// Renders the ownership-conflict error for a project whose builder authors `_custom/dev`.
///
/// The first publication creates the manifest under the authored root, then a foreign builder
/// manifest forces the second publication to fail against that retained output.
fn ownership_conflict_message(page_source: &str) -> String {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section_and_source(
        project_dir.path(),
        "game",
        "    dev_output = \"_custom/dev\",\n",
        page_source,
    );
    let builder = ArtefactBuilder::new(&[("game.wasm", b"bytes")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_custom/dev");
    fs::write(
        output_root.join(MANIFEST_FILENAME),
        "# moth-manifest v4\n# builder: html\n# profile: dev\n# managed_extensions: .html,.js,.wasm\nindex.html\n",
    )
    .expect("should write a foreign builder manifest");

    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the second build should compile");
    let error = second
        .write_outputs()
        .expect_err("a foreign manifest owner should fail publication");

    assert_eq!(error.kind(), ErrorKind::Build, "{error}");
    let rendered = error.messages().join("\n");
    normalize_project_paths(&rendered, project_dir.path())
}

#[test]
fn compiler_warnings_return_from_successful_publication() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), DUPLICATE_LITERAL_PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("game.wasm", b"bytes")]);

    let built = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("warning-only project should build");
    let outputs = built.write_outputs().expect("the build should publish");

    assert_eq!(
        outputs.warnings().len(),
        3,
        "a successful publication must return the frozen compiler warnings"
    );
    assert!(
        outputs.warnings().iter().all(|warning| {
            warning.starts_with("W|MOTH-RULE-0022|")
                && warning.split('|').nth(2) == Some("@entry.moth")
        }),
        "frozen warnings must retain their stable code and authored source path"
    );
    assert!(project_dir.path().join("_dev/game.wasm").is_file());
}

#[test]
fn into_warnings_returns_warnings_without_writing_output() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), DUPLICATE_LITERAL_PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("game.wasm", b"published")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_dev");
    let published = fs::read(output_root.join("game.wasm")).expect("artefact should exist");
    let manifest = read_manifest(&output_root);

    builder.set(&[("new.wasm", b"new")]);
    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the second build should compile");
    let warnings = second
        .into_warnings()
        .expect("warning finalisation should succeed");

    assert_eq!(warnings.len(), 3);
    assert!(warnings.iter().all(|warning| {
        warning.starts_with("W|MOTH-RULE-0022|") && warning.split('|').nth(2) == Some("@entry.moth")
    }));
    assert_path_missing(&output_root.join("new.wasm"));
    assert_eq!(
        fs::read(output_root.join("game.wasm")).expect("previous artefact should remain"),
        published
    );
    assert_eq!(read_manifest(&output_root), manifest);
}

#[test]
fn located_output_diagnostics_match_with_and_without_compiler_warnings() {
    let without_warnings = ownership_conflict_message(PAGE_SOURCE);
    let with_warnings = ownership_conflict_message(DUPLICATE_LITERAL_PAGE_SOURCE);

    assert_eq!(
        with_warnings, without_warnings,
        "staged compiler warnings must not change the located output diagnostic"
    );
    assert!(
        with_warnings.contains("@config.moth"),
        "the late output diagnostic must retain its authored source location: {with_warnings}"
    );
    assert!(
        with_warnings.contains("_custom/dev"),
        "the late output diagnostic must name the authored output root: {with_warnings}"
    );
    assert!(
        !with_warnings.contains("MOTH-RULE-0022"),
        "a failed publication must discard staged compiler warnings: {with_warnings}"
    );
}

#[test]
fn external_builder_publishes_artefacts_under_the_default_dev_root() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[
        ("game.wasm", b"\0asm-test-bytes"),
        ("a/b.bin", b"nested-bytes"),
    ]);

    let built = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("a project with an artefact should build");
    let outputs = built.write_outputs().expect("outputs should publish");
    assert!(outputs.warnings().is_empty());

    let output_root = project_dir.path().join("_dev");
    assert_eq!(outputs.output_root(), output_root);
    assert_eq!(
        outputs.written_paths(),
        &[PathBuf::from("game.wasm"), PathBuf::from("a/b.bin")],
        "written paths are relative to the output root, including nested artefacts"
    );
    assert_eq!(
        fs::read(output_root.join("game.wasm")).expect("artefact should be written"),
        b"\0asm-test-bytes".to_vec()
    );
    assert_eq!(
        fs::read(output_root.join("a/b.bin")).expect("nested artefact should be written"),
        b"nested-bytes".to_vec()
    );
    assert!(outputs.output_warnings().is_empty());
}

#[test]
fn external_builder_publishes_under_the_release_root_for_a_release_request() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("game.wasm", b"release-bytes")]);

    let request = BuildRequest::new(project_dir.path()).profile(BuildProfile::Release);
    let built = build_project(&builder, request).expect("release project should build");
    let outputs = built
        .write_outputs()
        .expect("release outputs should publish");

    let output_root = project_dir.path().join("_release");
    assert_eq!(outputs.output_root(), output_root);
    assert_eq!(
        fs::read(output_root.join("game.wasm")).expect("artefact should be written"),
        b"release-bytes".to_vec()
    );
    assert_path_missing(&project_dir.path().join("_dev"));
}

#[test]
fn builder_authored_output_root_is_honoured() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(
        project_dir.path(),
        "game",
        "    dev_output = \"_custom/dev\",\n",
    );
    let builder = ArtefactBuilder::new(&[("game.wasm", b"custom-root-bytes")]);

    let built = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("a project with a custom output root should build");
    let outputs = built.write_outputs().expect("outputs should publish");

    let output_root = project_dir.path().join("_custom/dev");
    assert_eq!(outputs.output_root(), output_root);
    assert_eq!(
        fs::read(output_root.join("game.wasm")).expect("artefact should be written"),
        b"custom-root-bytes".to_vec()
    );
    assert_path_missing(&project_dir.path().join("_dev"));
}

#[test]
fn failed_compilation_and_failed_build_leave_published_outputs_untouched() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("game.wasm", b"first-build")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_dev");
    let published = fs::read(output_root.join("game.wasm")).expect("artefact should exist");
    let manifest = read_manifest(&output_root);

    fs::write(
        project_dir.path().join("@entry.moth"),
        UNTERMINATED_RECORD_PAGE_SOURCE,
    )
    .expect("should write invalid source");
    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("invalid source should fail compilation");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert_eq!(
        fs::read(output_root.join("game.wasm")).expect("artefact should remain"),
        published
    );
    assert_eq!(read_manifest(&output_root), manifest);

    write_project(project_dir.path(), PAGE_SOURCE, false);
    builder.fail_build();
    let error = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect_err("a failing builder should fail the build");
    assert_eq!(error.kind(), ErrorKind::Build);
    assert_eq!(
        fs::read(output_root.join("game.wasm")).expect("artefact should remain"),
        published
    );
    assert_eq!(read_manifest(&output_root), manifest);
}

#[test]
fn artefact_paths_are_validated_when_recorded() {
    for (path, expected_fragment) in [
        ("/absolute.bin", "/absolute.bin"),
        ("../escape.bin", "../escape.bin"),
        ("", "empty"),
    ] {
        let mut artifacts = BuildArtifacts::new();
        let error = artifacts
            .add(path, b"bytes")
            .expect_err("an invalid artefact path should be rejected");

        assert_eq!(error.kind(), ErrorKind::Build, "{path:?}");
        assert!(
            error.to_string().contains(expected_fragment),
            "{path:?}: {error}"
        );
    }
}

#[test]
fn artefact_paths_cannot_claim_the_build_manifest() {
    let mut artifacts = BuildArtifacts::new();
    let error = artifacts
        .add(MANIFEST_FILENAME, b"bytes")
        .expect_err("the build manifest path is reserved");

    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains(MANIFEST_FILENAME), "{error}");
}

#[test]
fn duplicate_artefact_paths_collide_by_case_folded_identity() {
    let mut artifacts = BuildArtifacts::new();
    artifacts
        .add("game.wasm", b"first")
        .expect("the first artefact should be recorded");

    let error = artifacts
        .add("GAME.WASM", b"second")
        .expect_err("a case-only duplicate should collide");

    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains("GAME.WASM"), "{error}");
}

#[test]
fn file_and_descendant_artefacts_conflict_in_both_orders() {
    let mut file_first = BuildArtifacts::new();
    file_first
        .add("asset", b"file")
        .expect("the parent artefact should be recorded");
    let error = file_first
        .add("asset/child.bin", b"child")
        .expect_err("a descendant of a file artefact should conflict");
    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains("'asset'"), "{error}");
    assert!(error.to_string().contains("'asset/child.bin'"), "{error}");

    let mut descendant_first = BuildArtifacts::new();
    descendant_first
        .add("asset/child.bin", b"child")
        .expect("the descendant artefact should be recorded");
    let error = descendant_first
        .add("ASSET", b"file")
        .expect_err("a case-folded parent of a recorded descendant should conflict");
    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains("'ASSET'"), "{error}");
    assert!(error.to_string().contains("'asset/child.bin'"), "{error}");

    let mut nested_ancestor = BuildArtifacts::new();
    nested_ancestor
        .add("asset/icons/logo.bin", b"logo")
        .expect("the nested artefact should be recorded");
    let error = nested_ancestor
        .add("asset/icons", b"icons")
        .expect_err("a multi-component ancestor should conflict");
    assert_eq!(error.kind(), ErrorKind::Build);
}

#[test]
fn write_failure_is_reported_without_cleaning_stale_output() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("old.bin", b"old"), ("stale.bin", b"stale")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_dev");
    let old_bytes = fs::read(output_root.join("old.bin")).expect("old artefact should exist");
    let stale_bytes = fs::read(output_root.join("stale.bin")).expect("stale artefact should exist");
    let manifest = read_manifest(&output_root);

    fs::create_dir_all(output_root.join("blocked.bin"))
        .expect("should occupy the next artefact destination with a directory");
    builder.set(&[("blocked.bin", b"new")]);

    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the replacement build should compile");
    let error = second
        .write_outputs()
        .expect_err("a destination occupied by a directory should fail the write");

    assert_eq!(error.kind(), ErrorKind::Infrastructure, "{error}");
    assert_eq!(
        fs::read(output_root.join("old.bin")).expect("old artefact should remain"),
        old_bytes,
        "a preflight failure happens before any output mutation"
    );
    assert_eq!(
        fs::read(output_root.join("stale.bin")).expect("stale artefact should remain"),
        stale_bytes,
        "a preflight failure must not run stale cleanup"
    );
    assert!(!output_root.join("blocked.bin").is_file());
    assert_eq!(read_manifest(&output_root), manifest);
}

#[test]
fn repeated_builds_clean_removed_artefacts_and_keep_unchanged_ones() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("keep.bin", b"keep"), ("stale.bin", b"stale")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_dev");

    builder.set(&[("keep.bin", b"keep")]);
    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the second build should succeed");
    let outputs = second
        .write_outputs()
        .expect("the second build should publish");

    assert_eq!(outputs.written_paths(), &[PathBuf::from("keep.bin")]);
    assert_eq!(
        fs::read(output_root.join("keep.bin")).expect("unchanged artefact should remain"),
        b"keep".to_vec()
    );
    assert_path_missing(&output_root.join("stale.bin"));
    assert!(!read_manifest(&output_root).contains("stale.bin"));
}

#[test]
fn conflicting_manifest_owner_is_a_build_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("old.bin", b"old"), ("stale.bin", b"stale")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_dev");
    let old_bytes = fs::read(output_root.join("old.bin")).expect("old artefact should exist");
    let stale_bytes = fs::read(output_root.join("stale.bin")).expect("stale artefact should exist");

    fs::write(
        output_root.join(MANIFEST_FILENAME),
        "# moth-manifest v4\n# builder: html\n# profile: dev\n# managed_extensions: .html,.js,.wasm\nindex.html\nold.bin\nstale.bin\n",
    )
    .expect("should write a foreign builder manifest");
    let foreign_manifest = read_manifest(&output_root);

    builder.set(&[("new.bin", b"new")]);
    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the project should compile");
    let error = second
        .write_outputs()
        .expect_err("a foreign manifest owner should fail publication");

    assert_eq!(error.kind(), ErrorKind::Build, "{error}");
    assert!(
        !error.messages().is_empty(),
        "the writer's user diagnostic must be rendered: {error}"
    );
    assert_eq!(
        fs::read(output_root.join("old.bin")).expect("old artefact should remain"),
        old_bytes,
        "ownership preparation happens before any output mutation"
    );
    assert_eq!(
        fs::read(output_root.join("stale.bin")).expect("stale artefact should remain"),
        stale_bytes,
        "ownership preparation must not run stale cleanup"
    );
    assert_path_missing(&output_root.join("new.bin"));
    assert_eq!(read_manifest(&output_root), foreign_manifest);
}

#[test]
fn conflicting_manifest_owner_reports_the_authored_output_root() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project_with_builder_section(
        project_dir.path(),
        "game",
        "    dev_output = \"_custom/dev\",\n",
    );
    let builder = ArtefactBuilder::new(&[("old.bin", b"old")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_custom/dev");
    fs::write(
        output_root.join(MANIFEST_FILENAME),
        "# moth-manifest v4\n# builder: html\n# profile: dev\n# managed_extensions: .html,.js,.wasm\nindex.html\n",
    )
    .expect("should write a foreign builder manifest");

    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the project should compile");
    let error = second
        .write_outputs()
        .expect_err("a foreign manifest owner should fail publication");

    assert_eq!(error.kind(), ErrorKind::Build, "{error}");
    let rendered = error.messages().join("\n");
    assert!(rendered.contains("_custom/dev"), "{rendered}");
    assert!(rendered.contains("html"), "{rendered}");
}

#[test]
fn empty_artefact_build_cleans_the_previous_manifest_outputs() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);
    let builder = ArtefactBuilder::new(&[("game.wasm", b"bytes")]);

    let first = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("the first build should succeed");
    first
        .write_outputs()
        .expect("the first build should publish");
    let output_root = project_dir.path().join("_dev");

    builder.set(&[]);
    let second = build_project(&builder, BuildRequest::new(project_dir.path()))
        .expect("an artefact-free build should succeed");
    let outputs = second
        .write_outputs()
        .expect("an artefact-free build should still publish its manifest");

    assert!(outputs.written_paths().is_empty());
    assert_path_missing(&output_root.join("game.wasm"));
    let manifest = read_manifest(&output_root);
    assert!(!manifest.contains("game.wasm"), "{manifest}");
    assert!(
        output_root.join(MANIFEST_FILENAME).is_file(),
        "the empty build must still own its manifest"
    );
}

#[test]
fn single_file_project_root_is_a_build_error_before_building() {
    let project_dir = tempfile::tempdir().expect("should create source directory");
    let entry = project_dir.path().join("@entry.moth");
    // Invalid source proves the rejection happens before compilation: a compile-first path would
    // return a `Compilation` error instead.
    fs::write(&entry, UNTERMINATED_RECORD_PAGE_SOURCE).expect("should write single-file source");
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(&entry))
        .expect_err("the SDK accepts project directories only");

    assert_eq!(error.kind(), ErrorKind::Build, "{error}");
    assert!(
        error
            .to_string()
            .contains("Point `BuildRequest` at the project directory"),
        "{error}"
    );
    assert!(
        !builder.build_ran.get(),
        "a single-file request must not reach ProjectBuilder::build"
    );
    assert_path_missing(&project_dir.path().join("_dev"));
}

#[test]
fn missing_source_file_root_keeps_the_path_infrastructure_error() {
    let project_dir = tempfile::tempdir().expect("should create source directory");
    let missing = project_dir.path().join("missing.moth");
    let builder = RecordingBuildBuilder {
        build_ran: Cell::new(false),
    };

    let error = build_project(&builder, BuildRequest::new(&missing))
        .expect_err("a missing source file is not a buildable root");

    assert_eq!(
        error.kind(),
        ErrorKind::Infrastructure,
        "a missing root keeps the shared path-validation error: {error}"
    );
    assert!(!builder.build_ran.get());
}
