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
const PAGE_SOURCE: &str = "#[:<h1>About</h1>]\n";

/// A record literal that is never closed, so the frontend rejects the project before any builder
/// runs.
const UNTERMINATED_RECORD_PAGE_SOURCE: &str = "stats #= (a = 1\n";

/// Imports `greet` from the `@ui` package that only one test builder registers.
const UI_IMPORT_PAGE_SOURCE: &str = "@ui greet\nvalue = greet()\n";

/// A `Byte` value passed to a host function that takes `Byte`.
const HOST_BYTE_PAGE_SOURCE: &str = "@platform takes_byte\noctet Byte = 42\ntakes_byte(octet)\n";

/// A `U8` value passed where the host function takes `Byte`.
const HOST_U8_PAGE_SOURCE: &str = "@platform takes_byte\noctet U8 = 42\ntakes_byte(octet)\n";

#[test]
fn external_crate_defines_source_and_host_packages() {
    let ui_dir = tempfile::tempdir().expect("should create package directory");
    let definition = BuilderDefinition::new("game")
        .source_package(
            SourcePackage::new("@ui")
                .from_dir(ui_dir.path())
                .available_in_templates(),
        )
        .host_package(
            HostPackage::new("@platform").function(
                HostFunction::new("has_prefix")
                    .parameter(HostType::String)
                    .parameter(HostType::String)
                    .returns(HostType::Bool),
            ),
        )
        .moth_templates()
        .markdown()
        .finish()
        .expect("public definition should finish");

    assert_eq!(definition.identity(), "game");
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
/// `+package.moth` facade beside `config.moth`.
fn write_project(project_root: &Path, page_source: &str, with_facade: bool) {
    let src = project_root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        project_root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), page_source).expect("should write page");
    if with_facade {
        fs::write(
            project_root.join("+package.moth"),
            "export:\n    label #= \"facade\"\n;\n",
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

/// Writes a `@ui` package root whose only module exports `greet`.
fn write_ui_package(package_root: &Path) {
    fs::write(
        package_root.join("@mod.moth"),
        "export:\n    greet || -> I32:\n        return 1\n    ;\n;\n",
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
    assert!(rendered_lines.contains("@page.moth"), "{rendered_lines}");
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
