//! Proves a separate crate can describe a builder through public SDK names only.

use moth::builder::{
    BuildArtifacts, BuildContext, BuildProfile, BuildRequest, BuilderDefinition,
    BuilderDefinitionDraft, CompiledProject, Error, ErrorKind, HostPackage, ProjectBuilder,
    SourcePackage, build_project,
};
use moth::host::{HostFunction, HostType};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

/// The smallest page that compiles without diagnostics.
const PAGE_SOURCE: &str = "[:<h1>About</h1>]\n";

/// A record literal that is never closed, so the frontend rejects the project before any builder
/// runs.
const UNTERMINATED_RECORD_PAGE_SOURCE: &str = "stats #= (a = 1\n";

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

/// Accepts any project and returns empty artifacts, so the tests observe only the package facade
/// that the driver hands to `build`.
struct PackageFacadeBuilder;

impl ProjectBuilder for PackageFacadeBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game").finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        project.require_package()?;
        Ok(BuildArtifacts::new())
    }
}

/// Writes a one-page project whose page holds `page_source` and, when requested, its
/// `@package.moth` facade beside `config.moth`.
fn write_project(project_root: &Path, page_source: &str, with_facade: bool) {
    fs::write(
        project_root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\n",
    )
    .expect("should write config");
    fs::write(project_root.join("@entry.moth"), page_source).expect("should write page");
    if with_facade {
        fs::write(
            project_root.join("@package.moth"),
            "$export\nlabel #= \"facade\"\n",
        )
        .expect("should write package facade");
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

    let built = build_project(&PackageFacadeBuilder, BuildRequest::new(project_dir.path()))
        .expect("project with a package facade should build");

    assert!(built.warnings().is_empty());
}

#[test]
fn external_builder_without_facade_returns_build_error() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path(), PAGE_SOURCE, false);

    let error = build_project(&PackageFacadeBuilder, BuildRequest::new(project_dir.path()))
        .expect_err("project without a package facade should fail the build");

    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(error.to_string().contains("+<name>.moth"), "{error}");
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
    assert!(
        built.warnings().is_empty(),
        "the nested template fixture should compile without warnings: {:?}",
        built.warnings()
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

/// Writes a paired `@ui` source package whose external surface is its `+` root.
fn write_ui_paired_package(package_root: &Path) {
    fs::write(
        package_root.join("@mod.moth"),
        "normal_value #= \"normal\"\n",
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

/// Writes a `@ui` package root whose only module exports `greet`.
fn write_ui_package(package_root: &Path) {
    fs::write(
        package_root.join("@mod.moth"),
        "$export\ngreet || -> I32:\n    return 1\n;\n",
    )
    .expect("should write package module");
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
