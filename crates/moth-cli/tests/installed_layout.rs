//! WHAT: runs a relocated `moth` executable laid out like the release archive.
//! WHY: compiler-shipped package files (the `@core/html`, `@core/css` and `@builder/web/site` sources
//!      and the `@builder/web/*` binding assets) must come from the `packages/` directory beside an
//!      installed executable, not from the checkout that built it, and a damaged installation
//!      must say what is missing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Output;

use tempfile::tempdir;

const REPOSITORY_PACKAGES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages");

/// Only the installed copy of `@core/css` exports this name, so reading it proves which root won.
const INSTALLED_MARKER: &str = "\n$export\ninstalled_marker #= \"from installed packages\"\n";

/// Only the installed copy of `canvas.js` carries this comment, so emitting it proves the HTML
/// builder copied the shipped asset rather than the checkout's.
const INSTALLED_CANVAS_MARKER: &str = "\n// installed canvas asset\n";

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("destination directory should be created");
    for entry in fs::read_dir(from).expect("source directory should be readable") {
        let entry = entry.expect("directory entry should be readable");
        let target = to.join(entry.file_name());
        if entry
            .file_type()
            .expect("file type should be readable")
            .is_dir()
        {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("file should be copied");
        }
    }
}

/// Copies the built executable into `<root>/install`, mirroring the release archive.
fn install_executable(root: &Path) -> PathBuf {
    let built = Path::new(env!("CARGO_BIN_EXE_moth"));
    let install = root.join("install");
    fs::create_dir_all(&install).expect("install directory should be created");
    let executable = install.join(built.file_name().expect("executable has a file name"));
    fs::copy(built, &executable).expect("executable should be copied");
    executable
}

/// A directory project using an ordinary entry, explicit `@core/html`/`@core/css` imports, a
/// reachable `@builder/web/canvas` call and a `.mtf` that reads both implicit prelude providers plus
/// the installed-only marker.
fn write_project(root: &Path) -> PathBuf {
    let project = root.join("site");
    fs::create_dir_all(&project).expect("project directory should be created");
    fs::write(
        project.join("@config.moth"),
        "project #= (\n    name = \"installed\",\n)\nhtml #= ()\n",
    )
    .expect("config should be written");
    fs::write(
        project.join("@entry.moth"),
        r#"@core/html p
@core/css right
@builder/web/canvas as drawing_api
@intro content as intro

~[:[intro]]
~[p, right: explicit imports]

draw || -> String, Error!:
    element = drawing_api.get_canvas("game_canvas")!
    drawing ~= drawing_api.context_2d(element)!
    drawing_api.fill_rect(~drawing, 0.0, 0.0, 10.0, 10.0)
    return "drawn"
;

status = draw() catch:
    then "unavailable"
;

io.line([: [status]])
"#,
    )
    .expect("entry should be written");
    fs::write(
        project.join("intro.mtf"),
        "[p, center: both providers]\n\n[installed_marker]\n",
    )
    .expect("template should be written");
    project
}

fn run(executable: &Path, arguments: &[&str]) -> (Output, String) {
    let output = Command::new(executable)
        .args(arguments)
        .env_remove("MOTH_BENCH_STATUS")
        .output()
        .expect("moth subprocess should start");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output, report)
}

/// Copies the repository's packages to `<root>/install/packages`, marking the installed copies of
/// `@core/css` and `canvas.js` so outputs can only come from this installation.
fn install_marked_packages(root: &Path) -> PathBuf {
    let packages = root.join("install/packages");
    copy_tree(Path::new(REPOSITORY_PACKAGES), &packages);
    for (relative, marker) in [
        ("core/css/@package.moth", INSTALLED_MARKER),
        ("builder/web/canvas/canvas.js", INSTALLED_CANVAS_MARKER),
    ] {
        let path = packages.join(relative);
        let mut source = fs::read_to_string(&path).expect("installed file should be readable");
        source.push_str(marker);
        fs::write(&path, source).expect("installed file should be written");
    }
    packages
}

#[test]
fn installed_packages_beside_the_executable_replace_the_source_checkout() {
    let root = tempdir().expect("temporary directory should be created");
    let executable = install_executable(root.path());
    install_marked_packages(root.path());
    let project = write_project(root.path());
    let project_argument = project.to_str().expect("project path should be UTF-8");

    let (output, report) = run(&executable, &["build", project_argument]);
    assert!(output.status.success(), "installed build failed: {report}");
    let html = fs::read_to_string(project.join("_dev/index.html"))
        .expect("built document should be readable");
    // Page fragments may be mounted from escaped JavaScript strings, so match quote-free text.
    for expected in [
        "text-align: center;",
        "both providers",
        "from installed packages",
        "text-align: right;",
        "explicit imports",
    ] {
        assert!(html.contains(expected), "missing {expected:?}: {html}");
    }
    let emitted_canvas = fs::read_to_string(project.join("_dev/_moth/js/canvas.js"))
        .expect("reachable canvas calls should emit the shipped asset");
    assert!(
        emitted_canvas.contains(INSTALLED_CANVAS_MARKER.trim()),
        "the emitted canvas asset must be the installed copy"
    );
}

fn assert_damaged_package_set(report: &str, missing: &str) {
    assert!(
        report.contains("compiler-shipped package set at")
            && report.contains("is incomplete")
            && report.contains(missing),
        "a damaged installation must name the incomplete package set and {missing:?}: {report}"
    );
}

#[test]
fn incomplete_installation_never_borrows_a_core_package_from_the_checkout() {
    // `packages/` exists, so it is the whole package set, but `@core/html` is missing from it:
    // first as a directory, then as its `@package.moth` facade. The checkout still has
    // `packages/core/html`; using it would mix two versions.
    #[derive(Clone, Copy)]
    enum InstalledHtmlState {
        Absent,
        EmptyDirectory,
        NormalRootOnly,
    }

    for state in [
        InstalledHtmlState::Absent,
        InstalledHtmlState::EmptyDirectory,
        InstalledHtmlState::NormalRootOnly,
    ] {
        let root = tempdir().expect("temporary directory should be created");
        let executable = install_executable(root.path());
        let packages = install_marked_packages(root.path());
        // Start from the complete shipped set so only `core/html` is damaged; the builder web
        // packages are part of the same selected set and must not decide this report.
        fs::remove_dir_all(packages.join("core/html")).expect("html directory should be removable");
        match state {
            InstalledHtmlState::Absent => {}
            InstalledHtmlState::EmptyDirectory => {
                fs::create_dir_all(packages.join("core/html"))
                    .expect("html directory should be created");
            }
            InstalledHtmlState::NormalRootOnly => {
                fs::create_dir_all(packages.join("core/html"))
                    .expect("html directory should be created");
                // A normal `@mod.moth` is an internal scope, never the package facade.
                fs::write(packages.join("core/html/@mod.moth"), "")
                    .expect("html normal root should be written");
            }
        }
        let project = write_project(root.path());

        let (output, report) = run(
            &executable,
            &[
                "check",
                project.to_str().expect("project path should be UTF-8"),
            ],
        );

        assert_eq!(output.status.code(), Some(1), "report: {report}");
        // The filesystem spelling may use native separators; the leaf still identifies the
        // missing Core package on every host.
        assert_damaged_package_set(&report, "html");
        if matches!(state, InstalledHtmlState::NormalRootOnly) {
            assert!(
                report.contains("@package.moth"),
                "a shipped package without a support facade must name @package.moth: {report}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn symlinked_installed_package_set_reports_its_own_normal_only_package_as_damaged() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("temporary directory should be created");
    let executable = install_executable(root.path());
    // The installed set is addressed through a symlink. The shipped registration must
    // retain its damaged-installation diagnostic after package discovery resolves the link.
    let packages = root.path().join("shipped-packages");
    fs::create_dir_all(&packages).expect("shipped package directory should be created");
    copy_tree(Path::new(REPOSITORY_PACKAGES), &packages);
    fs::remove_dir_all(packages.join("core/html")).expect("html directory should be removable");
    fs::create_dir_all(packages.join("core/html")).expect("html directory should be created");
    // A normal-only root is an internal scope, never the package facade.
    fs::write(packages.join("core/html/@mod.moth"), "")
        .expect("html normal root should be written");
    symlink(&packages, root.path().join("install/packages"))
        .expect("installed package set symlink should be created");

    let project = root.path().join("site");
    fs::create_dir_all(&project).expect("project directory should be created");
    fs::write(
        project.join("@config.moth"),
        "project #= (\n    name = \"symlinked\",\n)\nhtml #= ()\n",
    )
    .expect("config should be written");
    fs::write(project.join("@entry.moth"), "[:page]\n").expect("entry should be written");

    let (output, report) = run(
        &executable,
        &[
            "check",
            project.to_str().expect("project path should be UTF-8"),
        ],
    );

    assert_eq!(output.status.code(), Some(1), "report: {report}");
    assert_damaged_package_set(&report, "html");
    assert!(
        report.contains("@package.moth"),
        "a shipped package without a support facade must name @package.moth: {report}"
    );
}

#[test]
fn incomplete_installation_never_borrows_a_web_asset_from_the_checkout() {
    let root = tempdir().expect("temporary directory should be created");
    let executable = install_executable(root.path());
    let packages = install_marked_packages(root.path());
    fs::remove_file(packages.join("builder/web/canvas/canvas.js"))
        .expect("canvas asset should be removed");
    let project = write_project(root.path());

    let (output, report) = run(
        &executable,
        &[
            "build",
            project.to_str().expect("project path should be UTF-8"),
        ],
    );

    assert_eq!(output.status.code(), Some(1), "report: {report}");
    assert_damaged_package_set(&report, "canvas.js");
}
