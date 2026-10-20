//! Compiled package identity tests for SDK builds.
//!
//! WHAT: builds one project twice with the same `@ui` package installed in two different
//! directories, then compares the compiler-owned package identities observed by `build`.
//! WHY: observing compiler identities ensures the comparison detects identities that incorrectly
//! depend on the package's install directory.

use crate::builder::{
    BuildArtifacts, BuildContext, BuildRequest, BuilderDefinition, CompiledProject, Error,
    ProjectBuilder, SourcePackage, build_project,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

/// Declares only the mandatory `project` section because the test builder adds no config sections.
const CONFIG_SOURCE: &str = "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\n";

/// The project's only page, importing `greet` from the `@ui` package.
const PAGE_SOURCE: &str = "@ui greet\nmessage = greet()\n";

/// The `@ui` package's only module, exporting `greet`.
const UI_MODULE_SOURCE: &str = "export:\n    greet || -> I32:\n        return 1\n    ;\n;\n";

/// Records the compiler-owned package identities that `build` observes.
struct IdentityRecordingBuilder {
    ui_root: PathBuf,
    observed_identities: RefCell<Vec<Option<StablePackageIdentity>>>,
}

impl ProjectBuilder for IdentityRecordingBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@ui").from_dir(&self.ui_root))
            .finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        let mut identities = Vec::new();
        for entry in project.compilation.entries() {
            identities.push(entry.source_domain.cloned());
            for linked_module in entry.linked_modules {
                identities.push(linked_module.source_domain.cloned());
            }
        }
        *self.observed_identities.borrow_mut() = identities;
        Ok(BuildArtifacts::new())
    }
}

#[test]
fn compiled_package_identities_do_not_depend_on_package_install_directory() {
    let project_dir = tempfile::tempdir().expect("should create project directory");
    write_project(project_dir.path());
    let first_install = tempfile::tempdir().expect("should create first package directory");
    write_ui_package(first_install.path());
    let second_install = tempfile::tempdir().expect("should create second package directory");
    write_ui_package(second_install.path());

    let first = observe_package_identities(project_dir.path(), first_install.path());
    let second = observe_package_identities(project_dir.path(), second_install.path());

    assert!(
        first
            .iter()
            .any(|identity| { matches!(identity, Some(identity) if identity.name() == "ui") }),
        "the compiler-owned @ui package identity should be observed: {first:?}"
    );
    assert_eq!(first, second);
}

/// Builds `project_root` with `ui_root` installed as `@ui` and returns compiler-owned identities
/// in entry order, with each entry preceding its linked modules.
fn observe_package_identities(
    project_root: &Path,
    ui_root: &Path,
) -> Vec<Option<StablePackageIdentity>> {
    let builder = IdentityRecordingBuilder {
        ui_root: fs::canonicalize(ui_root).expect("package root should canonicalize"),
        observed_identities: RefCell::new(Vec::new()),
    };
    build_project(&builder, BuildRequest::new(project_root)).expect("project should build");
    builder.observed_identities.into_inner()
}

/// Writes a project whose only page imports `greet` from `@ui`.
fn write_project(project_root: &Path) {
    let src_dir = project_root.join("src");
    fs::create_dir_all(&src_dir).expect("should create source directory");
    fs::write(project_root.join("config.moth"), CONFIG_SOURCE).expect("should write config");
    fs::write(src_dir.join("@page.moth"), PAGE_SOURCE).expect("should write page");
}

/// Writes a `@ui` package root whose only module exports `greet`.
fn write_ui_package(package_root: &Path) {
    fs::write(package_root.join("@mod.moth"), UI_MODULE_SOURCE)
        .expect("should write package module");
}
