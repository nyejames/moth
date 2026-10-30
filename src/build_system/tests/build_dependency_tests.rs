//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::{
    BackendInvocationFailure, BuildResult, ProjectCompilation, build_project,
};
use crate::build_system::output::output_path_identity;
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::external_packages::{
    CanonicalBindingSymbolIdentity, ExternalAbiType, ExternalJsLowering, ExternalReturnSlot,
    ExternalSymbolCategory, ExternalSymbolId, ExternalSymbolPath,
};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::compiler_tests::test_fs::assert_path_missing;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use std::collections::{BTreeMap, HashMap};
use std::fs;

/// The registered runtime module's exact emitted path.
///
/// Tests look this up exactly rather than matching a path suffix, so a runtime module emitted
/// under an unexpected directory is a failure rather than a silent pass.
const RUNTIME_MODULE_PATH: &str = "_moth/js/runtime/moth-runtime.js";

/// The generated glue directory, anchored at the output root.
///
/// Predicates use this as a prefix rather than a substring, so a glue module emitted under some
/// other parent directory cannot satisfy an assertion about the glue directory.
const GLUE_MODULE_PREFIX: &str = "_moth/js/glue/";

/// One unique, normalized view of the artifacts a build produced.
///
/// WHAT: indexes every emitted `OutputFile` by its portable relative path and offers
///       cardinality-proving selectors.
/// WHY: `output_files.iter().find_map(..)` returns the first match, so a build that emitted
///      two glue modules, two HTML pages or a duplicate path would still satisfy an assertion
///      about "the" artifact. These selectors prove exactly-one before returning anything, and
///      `paths()` makes the whole emitted set assertable instead of merely non-empty.
struct BuiltOutputs<'a> {
    by_path: BTreeMap<String, &'a OutputFile>,
}

impl<'a> BuiltOutputs<'a> {
    /// Index a project's outputs, failing on any path the output writer would not accept and on
    /// any two artifacts that share one canonical output-path identity.
    ///
    /// Validity and collision identity come from `output_path_identity`, so these tests apply
    /// the production destination policy rather than a second, almost-equivalent one.
    #[track_caller]
    fn index(project: &'a Project) -> Self {
        let mut by_path: BTreeMap<String, &'a OutputFile> = BTreeMap::new();
        let mut spelling_by_identity = HashMap::new();

        for output in &project.output_files {
            if matches!(output.file_kind(), FileKind::NotBuilt) {
                continue;
            }

            let relative_path = output.relative_output_path();
            let identity = match output_path_identity(relative_path) {
                Ok(identity) => identity,
                Err(reason) => panic!(
                    "the build emitted {relative_path:?}, which the output writer would reject \
                     as an invalid portable destination ({reason:?})"
                ),
            };

            let spelling = portable_path_text(relative_path);
            if let Some(existing) = spelling_by_identity.insert(identity, spelling.clone()) {
                assert_ne!(
                    existing, spelling,
                    "the build emitted more than one artifact at '{spelling}'"
                );
                panic!(
                    "the build emitted '{existing}' and '{spelling}', which share one output \
                     path identity and collide on case-insensitive filesystems"
                );
            }

            // Two spellings sharing one identity were rejected above, so this never displaces.
            by_path.insert(spelling, output);
        }

        Self { by_path }
    }

    /// Every emitted artifact path, in portable sorted order.
    fn paths(&self) -> Vec<&str> {
        self.by_path.keys().map(String::as_str).collect()
    }

    /// The artifact at exactly this portable path.
    #[track_caller]
    fn at(&self, path: &str) -> &'a OutputFile {
        match self.by_path.get(path) {
            Some(output) => output,
            None => panic!(
                "expected an artifact at '{path}', but the build emitted {:?}",
                self.paths()
            ),
        }
    }

    /// The single artifact whose portable path satisfies `predicate`.
    ///
    /// Panics when zero or more than one artifact matches, so a test about "the glue module"
    /// cannot pass while a second glue module is also being emitted.
    #[track_caller]
    fn exactly_one(&self, description: &str, predicate: impl Fn(&str) -> bool) -> &'a OutputFile {
        let matches = self
            .by_path
            .iter()
            .filter(|(path, _)| predicate(path))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [(_, output)] => output,
            [] => panic!(
                "expected exactly one {description}, but the build emitted {:?}",
                self.paths()
            ),
            several => panic!(
                "expected exactly one {description}, but {} matched: {:?}",
                several.len(),
                several.iter().map(|(path, _)| path).collect::<Vec<_>>()
            ),
        }
    }

    /// The portable path of the single artifact satisfying `predicate`.
    #[track_caller]
    fn exactly_one_path(&self, description: &str, predicate: impl Fn(&str) -> bool) -> &str {
        let matches = self
            .by_path
            .keys()
            .filter(|path| predicate(path))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [path] => path.as_str(),
            [] => panic!(
                "expected exactly one {description}, but the build emitted {:?}",
                self.paths()
            ),
            several => panic!(
                "expected exactly one {description}, but {} matched: {several:?}",
                several.len()
            ),
        }
    }

    /// Assert that no emitted artifact path satisfies `predicate`.
    #[track_caller]
    fn none_matching(&self, description: &str, predicate: impl Fn(&str) -> bool) {
        let matches = self
            .by_path
            .keys()
            .filter(|path| predicate(path))
            .collect::<Vec<_>>();
        assert!(
            matches.is_empty(),
            "expected no {description}, but found {matches:?}"
        );
    }
}

/// A stable name for an artifact kind, for failure messages.
fn file_kind_name(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::NotBuilt => "not-built",
        FileKind::Wasm(_) => "wasm",
        FileKind::Bytes(_) => "bytes",
        FileKind::Js(_) => "js",
        FileKind::Html(_) => "html",
        FileKind::Directory => "directory",
    }
}

/// The HTML text of an artifact, proving its kind.
#[track_caller]
fn html_text(output: &OutputFile) -> &str {
    match output.file_kind() {
        FileKind::Html(html) => html.as_str(),
        other => panic!(
            "expected an HTML artifact at {:?}, found a {} artifact",
            output.relative_output_path(),
            file_kind_name(other)
        ),
    }
}

/// The JavaScript text of an artifact, proving its kind.
#[track_caller]
fn js_text(output: &OutputFile) -> &str {
    match output.file_kind() {
        FileKind::Js(source) => source.as_str(),
        other => panic!(
            "expected a JS artifact at {:?}, found a {} artifact",
            output.relative_output_path(),
            file_kind_name(other)
        ),
    }
}

/// The one deferred output matching the built-in canvas asset path, proving exactly one exists.
#[track_caller]
fn deferred_canvas_output_path(project: &Project) -> String {
    let mut matches: Vec<String> = project
        .deferred_resources
        .iter()
        .map(|resource| portable_path_text(&resource.relative_output_path))
        .filter(|path| path.starts_with("_moth/js/canvas.js"))
        .collect();

    assert_eq!(
        matches.len(),
        1,
        "expected exactly one deferred built-in canvas asset, got: {matches:?}"
    );
    matches.remove(0)
}

/// Read one deferred resource through the shared registry, as the central writer does.
#[track_caller]
fn deferred_resource_text(project: &mut Project, relative_path: &str) -> String {
    let source_id = project
        .deferred_resources
        .iter()
        .find(|resource| portable_path_text(&resource.relative_output_path) == relative_path)
        .map(|resource| resource.source_id)
        .unwrap_or_else(|| panic!("expected a deferred resource at '{relative_path}'"));

    let bytes = project
        .resource_inputs
        .read_source(source_id)
        .unwrap_or_else(|error| {
            panic!("deferred resource '{relative_path}' should read from its source: {error:?}")
        });

    String::from_utf8(bytes.to_vec()).expect("deferred JS resource text should be UTF-8")
}

/// Check the selected raw exports and wrapper cardinality without freezing build-local IDs.
#[track_caller]
fn assert_glue_exports(glue: &str, expected: &[(&str, &[&str])]) {
    let mut actual_imports = BTreeMap::new();
    for line in glue.lines().filter(|line| line.starts_with("import { ")) {
        let (bindings, path) = line
            .strip_prefix("import { ")
            .and_then(|line| line.split_once(" } from \""))
            .expect("glue imports should be named ES module imports");
        let path = path
            .strip_suffix("\";")
            .expect("glue import should end after its module specifier");
        let mut exports = bindings
            .split(", ")
            .map(|binding| {
                binding
                    .split_once(" as __moth_external_fn")
                    .expect("raw exports should bind generated external names")
                    .0
            })
            .collect::<Vec<_>>();
        exports.sort_unstable();
        assert!(
            actual_imports.insert(path, exports).is_none(),
            "one provider module should have only one import statement: {glue}"
        );
    }

    let expected_imports = expected
        .iter()
        .map(|(path, exports)| {
            let mut exports = exports.to_vec();
            exports.sort_unstable();
            (*path, exports)
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual_imports, expected_imports,
        "glue should import exactly the page's demanded provider exports"
    );
    assert_eq!(
        glue.matches("export function __moth_glue_fn").count(),
        expected
            .iter()
            .map(|(_, exports)| exports.len())
            .sum::<usize>(),
        "each demanded external function should have exactly one wrapper: {glue}"
    );
}

/// Resolve the page's import to the emitted module and compare its actual exported names.
#[track_caller]
fn assert_page_imports_glue(page: &str, relative_glue_path: &str, glue: &str) {
    let import_suffix = format!(" }} from \"{relative_glue_path}\";");
    let matching_lines = page
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with(&import_suffix))
        .collect::<Vec<_>>();
    assert_eq!(
        matching_lines.len(),
        1,
        "the page must import its emitted glue once"
    );
    let mut imported = matching_lines[0]
        .strip_prefix("import { ")
        .and_then(|line| line.strip_suffix(&import_suffix))
        .expect("page glue preamble should use a named import")
        .split(", ")
        .collect::<Vec<_>>();
    imported.sort_unstable();
    let mut exported = glue
        .lines()
        .filter_map(|line| line.strip_prefix("export function "))
        .map(|line| {
            line.split_once('(')
                .expect("exported wrapper should have a parameter list")
                .0
        })
        .collect::<Vec<_>>();
    exported.sort_unstable();
    assert_eq!(
        imported, exported,
        "the page's imported names must match exactly its glue's emitted wrappers"
    );
}

/// Deferred records, not eager JS outputs, own the authored provider files.
#[track_caller]
fn assert_deferred_provider_sources(project: &mut Project, expected: &[(&str, &Path)]) {
    let mut paths = project
        .deferred_resources
        .iter()
        .map(|resource| portable_path_text(&resource.relative_output_path))
        .collect::<Vec<_>>();
    paths.sort();
    let mut expected_paths = expected.iter().map(|(path, _)| *path).collect::<Vec<_>>();
    expected_paths.sort_unstable();
    assert_eq!(
        paths, expected_paths,
        "only selected provider assets should be planned, once each"
    );
    for (path, source) in expected {
        BuiltOutputs::index(project)
            .none_matching("eager provider asset", |output| output == *path);
        assert_eq!(
            deferred_resource_text(project, path),
            fs::read_to_string(source).expect("authored provider source should be readable"),
            "the whole authored module should be retained at '{path}'"
        );
    }
}

mod built_outputs_tests {
    use super::*;
    use crate::compiler_tests::test_support::assert_panics_with;

    fn project_with(files: Vec<(&str, FileKind)>) -> Project {
        Project {
            output_files: files
                .into_iter()
                .map(|(path, kind)| OutputFile::new(PathBuf::from(path), kind))
                .collect(),
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::html(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        }
    }

    #[test]
    fn index_rejects_duplicate_artifact_paths() {
        let project = project_with(vec![
            ("index.html", FileKind::Html("first".to_owned())),
            ("index.html", FileKind::Html("second".to_owned())),
        ]);

        assert_panics_with("more than one artifact at 'index.html'", || {
            BuiltOutputs::index(&project);
        });
    }

    #[test]
    fn index_rejects_artifact_paths_that_differ_only_by_ascii_case() {
        let project = project_with(vec![
            ("assets/Page.js", FileKind::Js("// first".to_owned())),
            ("assets/page.js", FileKind::Js("// second".to_owned())),
        ]);

        assert_panics_with("share one output path identity", || {
            BuiltOutputs::index(&project);
        });
    }

    #[test]
    fn index_keeps_non_ascii_case_differences_distinct() {
        // The canonical output-path identity folds ASCII case only, so these are two
        // destinations in production and must be two entries here.
        let project = project_with(vec![
            ("Å.js", FileKind::Js("// upper".to_owned())),
            ("å.js", FileKind::Js("// lower".to_owned())),
        ]);

        assert_eq!(BuiltOutputs::index(&project).paths().len(), 2);
    }

    #[test]
    fn index_rejects_paths_the_output_writer_would_reject() {
        let project = project_with(vec![(
            "assets/../page.js",
            FileKind::Js("// escaped".to_owned()),
        )]);

        assert_panics_with(
            "invalid portable destination (ParentDirectorySegment)",
            || {
                BuiltOutputs::index(&project);
            },
        );
    }

    #[test]
    fn index_skips_not_built_entries() {
        let project = project_with(vec![
            ("index.html", FileKind::NotBuilt),
            ("page.js", FileKind::Js("// page".to_owned())),
        ]);

        assert_eq!(BuiltOutputs::index(&project).paths(), vec!["page.js"]);
    }

    #[test]
    fn exactly_one_rejects_multiple_matches() {
        let project = project_with(vec![
            ("_moth/js/glue/a.js", FileKind::Js("// a".to_owned())),
            ("_moth/js/glue/b.js", FileKind::Js("// b".to_owned())),
        ]);
        let outputs = BuiltOutputs::index(&project);

        assert_panics_with("expected exactly one glue module, but 2 matched", || {
            outputs.exactly_one("glue module", |path| path.starts_with(GLUE_MODULE_PREFIX));
        });
    }

    #[test]
    fn exactly_one_rejects_no_match() {
        let project = project_with(vec![("index.html", FileKind::Html(String::new()))]);
        let outputs = BuiltOutputs::index(&project);

        assert_panics_with("expected exactly one glue module", || {
            outputs.exactly_one("glue module", |path| path.starts_with(GLUE_MODULE_PREFIX));
        });
    }

    #[test]
    fn glue_predicates_are_anchored_at_the_output_root() {
        // A nested directory that merely contains the glue path is not the glue directory.
        let project = project_with(vec![(
            "vendor/_moth/js/glue/a.js",
            FileKind::Js("// vendored".to_owned()),
        )]);
        let outputs = BuiltOutputs::index(&project);

        outputs.none_matching("generated glue module", |path| {
            path.starts_with(GLUE_MODULE_PREFIX)
        });
    }

    #[test]
    fn at_reports_the_emitted_set_when_a_path_is_missing() {
        let project = project_with(vec![("main.html", FileKind::Html(String::new()))]);
        let outputs = BuiltOutputs::index(&project);

        assert_panics_with("expected an artifact at 'index.html'", || {
            outputs.at("index.html");
        });
    }

    #[test]
    fn html_text_rejects_a_js_artifact() {
        let project = project_with(vec![("page.js", FileKind::Js("// page".to_owned()))]);
        let outputs = BuiltOutputs::index(&project);

        assert_panics_with("expected an HTML artifact", || {
            html_text(outputs.at("page.js"));
        });
    }
}

#[test]
fn build_single_file_project_includes_reachable_dependency_files() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::create_dir_all(root.join("utils")).expect("should create utils directory");
    fs::write(root.join("main.moth"), "@utils/helper greet\ngreet()\n")
        .expect("should write main file");
    fs::write(
        root.join("utils/helper.moth"),
        "greet||:\n    io.line([: [\"hello\"]])\n;\n",
    )
    .expect("should write helper file");

    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let builder = HtmlProjectBuilder::new();
        let result = successful_compilation_value(
            build_project(&builder, "main.moth", &[], &BuildConfigInputSet::new())
                .unwrap_or_else(|never| match never {}),
        );

        let outputs = BuiltOutputs::index(&result.project);
        assert_eq!(
            outputs.paths(),
            vec!["main.html"],
            "a single-file build emits exactly one page for its entry"
        );

        // The dependency is evidence only if its body was lowered into the page and the entry
        // calls it. A non-empty output list would also be satisfied by an entry that dropped
        // the import entirely.
        let page = html_text(outputs.at("main.html"));
        assert!(
            page.contains("__moth_io_line(\" hello\")"),
            "the imported helper's body should be lowered into the emitted page"
        );
        assert!(
            page.contains("__moth_private_fn_0();"),
            "the entry should call the lowered helper"
        );
    }
}

#[test]
fn build_html_project_local_js_import_emits_generated_glue() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@drawing.js draw\nvalue = draw()\n")
        .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "/**\n * @moth.sig draw || -> I32\n */\nexport function draw() { return 7; }\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let outputs = BuiltOutputs::index(&result.project);
    let html = html_text(outputs.at("index.html"));
    assert!(html.contains("<script type=\"module\">"));
    assert!(html.contains("import { __moth_glue_fn"));
    assert!(html.contains("from \"./_moth/js/glue/module-"));

    // Exactly one glue module: a second one would mean the page imports from one module while
    // this assertion inspects another.
    let glue = js_text(outputs.exactly_one("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    }));
    assert!(glue.contains("import { draw as __moth_external_fn"));

    // The page must import the glue module that was actually emitted, not merely some path
    // under the glue directory.
    let glue_path = outputs
        .exactly_one_path("generated glue module", |path| {
            path.starts_with(GLUE_MODULE_PREFIX)
        })
        .to_owned();
    assert!(
        html.contains(&format!("from \"./{glue_path}\"")),
        "the page should import the emitted glue module '{glue_path}': {html}"
    );

    // The provider module is a deferred resource output planned at its declared stable path,
    // and the glue imports that same final component.
    let provider_paths: Vec<String> = result
        .project
        .deferred_resources
        .iter()
        .map(|resource| portable_path_text(&resource.relative_output_path))
        .filter(|path| path.starts_with("_moth/js/drawing.js"))
        .collect();
    assert_eq!(
        provider_paths.len(),
        1,
        "expected exactly one deferred provider JS module, got: {provider_paths:?}"
    );
    let provider_file = provider_paths[0]
        .rsplit('/')
        .next()
        .expect("a portable path always has a final component");
    assert!(
        glue.contains(&format!("from \"../{provider_file}\"")),
        "the glue should import the deferred provider module '{provider_file}': {glue}"
    );
}

#[test]
fn build_html_project_reexport_fixture_emits_provider_and_exact_wrappers() {
    // Keep the original real-bug source shared with its executable integration owner.
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases/html_js_reexport_linking/input");
    let builder = HtmlProjectBuilder::new();
    let mut result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("fixture path should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let provider_path = "_moth/js/components/background-animation.js";
    let source = root.join("src/components/background-animation.js");
    assert_deferred_provider_sources(&mut result.project, &[(provider_path, source.as_path())]);
    let outputs = BuiltOutputs::index(&result.project);
    let glue_path = outputs.exactly_one_path("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    });
    let page = html_text(outputs.at("index.html"));
    assert_page_imports_glue(
        page,
        &format!("./{glue_path}"),
        js_text(outputs.at(glue_path)),
    );
    assert_glue_exports(
        js_text(outputs.at(glue_path)),
        &[(
            "../components/background-animation.js",
            &["animate_canvas", "observed"],
        )],
    );

    // Exercise the real fixture-to-writer handoff once. Conflict/preflight and stale-output
    // behavior remain owned by the output writer's focused tests.
    let temp = tempfile::tempdir().expect("should create output directory");
    let output_root = temp.path().join("output");
    successful_compilation_value(write_project_outputs(
        &result.project,
        &always_write_options(output_root.clone(), None),
    ));
    assert_eq!(
        fs::read(output_root.join(provider_path)).expect("provider output should exist"),
        fs::read(&source).expect("authored provider source should be readable")
    );
    assert_eq!(
        fs::read_to_string(output_root.join(glue_path)).expect("glue output should exist"),
        js_text(outputs.at(glue_path))
    );
    assert_eq!(
        fs::read_to_string(output_root.join("index.html")).expect("page output should exist"),
        page
    );
}

/// Adds real source-package boundaries while retaining the production HTML surface and backend.
struct RegisteredJsPackageBuilder {
    project_root: PathBuf,
    package_root: PathBuf,
    marker_root: PathBuf,
    reverse_registration: bool,
}

impl BackendBuilder for RegisteredJsPackageBuilder {
    type Error = std::convert::Infallible;
    fn frontend_surface(&self) -> BuilderSurface {
        let mut surface = HtmlProjectBuilder::new().frontend_surface();
        let mut packages = [
            ("campaign", self.package_root.clone()),
            ("x/y", self.marker_root.clone()),
        ];
        if self.reverse_registration {
            packages.reverse();
        }
        for (prefix, root) in packages {
            surface
                .source_packages
                .register_filesystem_root(prefix, root, PackageOrigin::Builder)
                .expect("test source root should register");
        }
        surface
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        HtmlProjectBuilder::new().frontend_style_directives()
    }

    fn validate_project_config(
        &self,
        config: &Config,
        strings: &mut DiagnosticStringBuilder<'_>,
    ) -> Result<(), ProjectConfigError> {
        HtmlProjectBuilder::new().validate_project_config(config, strings)
    }

    fn build_backend(
        &self,
        compilation: ProjectCompilation,
        config: &Config,
        build_profile: BuildProfile,
        flags: &[Flag],
        context: &mut BackendBuildContext<'_>,
    ) -> Result<Project, BackendInvocationFailure<Self::Error>> {
        let providers = [
            (
                "binding.js",
                "projectValue",
                self.project_root.join("src/binding.js"),
            ),
            (
                "providers/binding.js",
                "packageValue",
                self.package_root.join("providers/binding.js"),
            ),
        ];
        // Inspect real definitions through each module's issuing registry. Pointer equality
        // or equal local IDs alone would not prove that package and project calls agree.
        for module in compilation.modules() {
            let registry = &module.link_facts.external_package_registry;
            for (logical_path, export_name, _) in &providers {
                let identity = CanonicalBindingSymbolIdentity {
                    package: StablePackageIdentity::binding(
                        PackageOrigin::ProjectLocal,
                        &format!("@html-js/{logical_path}"),
                    ),
                    symbol_path: ExternalSymbolPath::try_from_single("provider_value")
                        .expect("the fixture symbol is a valid leaf"),
                    category: ExternalSymbolCategory::Function,
                };
                let Some(ExternalSymbolId::Function(function_id)) =
                    registry.resolve_canonical_symbol(&identity)
                else {
                    panic!("every compiled boundary must resolve {identity:?}");
                };
                assert_eq!(
                    registry.canonical_symbol_identity(ExternalSymbolId::Function(function_id)),
                    Some(identity)
                );
                let definition = registry
                    .get_function_by_id(function_id)
                    .expect("a resolved function must have a definition");
                assert_eq!(definition.name, "provider_value");
                assert!(definition.parameters.is_empty());
                assert_eq!(
                    definition.returns,
                    [ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str)]
                );
                assert!(definition.error_return_type.is_none());
                assert!(matches!(
                    &definition.lowerings.js,
                    Some(ExternalJsLowering::ExternalModuleExport { export_name: actual })
                        if actual == export_name
                ));
            }
        }
        let entries = compilation.entries();
        assert_eq!(
            entries.len(),
            1,
            "source package roots must not become pages"
        );
        assert_eq!(entries[0].external_imports.len(), providers.len());
        for (logical_path, _, source) in &providers {
            let expected_origin = StableResourceOriginId::new(
                StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
                    "html-js",
                    StablePackageIdentity::binding(
                        PackageOrigin::ProjectLocal,
                        &format!("@html-js/{logical_path}"),
                    ),
                )),
                PortableResourcePath::from_portable_spelling(format!("_moth/js/{logical_path}"))
                    .expect("the fixture output is a portable resource path"),
            );
            let imports = entries[0]
                .external_imports
                .iter()
                .filter(|import| {
                    import
                        .runtime_asset
                        .as_ref()
                        .is_some_and(|asset| asset.origin == expected_origin)
                })
                .collect::<Vec<_>>();
            let [import] = imports.as_slice() else {
                panic!("the entry must retain exactly one import for {expected_origin:?}");
            };
            let package = entries[0]
                .module
                .link_facts
                .external_package_registry
                .get_package_by_id(import.package_id)
                .expect("entry import package must resolve in its issuing registry");
            assert_eq!(package.path, format!("@html-js/{logical_path}"));
            assert_eq!(package.metadata.origin, PackageOrigin::ProjectLocal);
            assert!(import.required_runtime_imports.is_empty());
            let asset = import
                .runtime_asset
                .as_ref()
                .expect("the matched import has its provider asset");
            assert_eq!(
                asset.canonical_source_path,
                fs::canonicalize(source).expect("provider source should exist")
            );
            assert_eq!(asset.asset_kind, "js");
        }
        HtmlProjectBuilder::new().build_backend(compilation, config, build_profile, flags, context)
    }
}

#[test]
fn build_html_registered_source_package_preserves_provider_identity_and_order() {
    let temporary = tempfile::tempdir().expect("should create campaign roots");
    let mut baseline = None;
    // Isolate topologies so the nested provider root cannot survive in the flat fixture.
    // Flat package module 0 is in bounds in the project graph but has a different namespace.
    for (topology, package_root_source, provider_root_source) in [
        (
            "nested",
            "export:\n    @providers provider_value as package_value\n;\n",
            Some("export:\n    @binding.js provider_value\n;\n"),
        ),
        (
            "flat",
            "export:\n    @providers/binding.js provider_value as package_value\n;\n",
            None,
        ),
    ] {
        // Compare an identical-root repeat with reversed creation and registration order.
        for (root_index, reversed) in [(0, false), (1, true), (0, false)] {
            let root = temporary.path().join(topology).join(root_index.to_string());
            let project_root = root.join("project");
            let package_root = root.join("campaign");
            let marker_root = root.join("marker");
            let mut sources = vec![
                (
                    project_root.join("config.moth"),
                    "project #= (\n    name = \"package_campaign\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
                ),
                (
                    project_root.join("src/@page.moth"),
                    "@binding.js provider_value as project_value\n@campaign package_value\n@x/y\n[:project=[project_value()]|package=[package_value()]|[y.marker]]\n",
                ),
                (
                    project_root.join("src/binding.js"),
                    "/** @moth.sig provider_value || -> String */\nexport function projectValue() { return \"project:17\"; }\n",
                ),
                (package_root.join("@mod.moth"), package_root_source),
                (
                    package_root.join("providers/binding.js"),
                    "/** @moth.sig provider_value || -> String */\nexport function packageValue() { return \"package:29\"; }\n",
                ),
                (
                    marker_root.join("@mod.moth"),
                    "export:\n    marker #= \"marker:43\"\n;\n",
                ),
            ];
            if let Some(source) = provider_root_source {
                sources.push((package_root.join("providers/@mod.moth"), source));
            }
            if reversed {
                sources.reverse();
            }
            for (path, source) in sources {
                fs::create_dir_all(path.parent().expect("fixture source has a parent"))
                    .expect("should create fixture directories");
                fs::write(path, source).expect("should write campaign source");
            }
            let builder = RegisteredJsPackageBuilder {
                project_root: project_root.clone(),
                package_root: package_root.clone(),
                marker_root,
                reverse_registration: reversed,
            };
            let mut built = successful_compilation_value(
                build_project(
                    &builder,
                    project_root.to_str().expect("fixture path should be UTF-8"),
                    &[],
                    &BuildConfigInputSet::new(),
                )
                .unwrap_or_else(|never| match never {}),
            );
            let project_source = project_root.join("src/binding.js");
            let package_source = package_root.join("providers/binding.js");
            assert_deferred_provider_sources(
                &mut built.project,
                &[
                    ("_moth/js/binding.js", project_source.as_path()),
                    ("_moth/js/providers/binding.js", package_source.as_path()),
                ],
            );
            let outputs = BuiltOutputs::index(&built.project);
            let glue_path = outputs.exactly_one_path("generated glue module", |path| {
                path.starts_with(GLUE_MODULE_PREFIX)
            });
            let glue = js_text(outputs.at(glue_path));
            let page = html_text(outputs.at("index.html"));
            assert_eq!(outputs.paths(), vec![glue_path, "index.html"]);
            assert_page_imports_glue(page, &format!("./{glue_path}"), glue);
            assert_glue_exports(
                glue,
                &[
                    ("../binding.js", &["projectValue"]),
                    ("../providers/binding.js", &["packageValue"]),
                ],
            );
            let output_root = root.join("output");
            successful_compilation_value(write_project_outputs(
                &built.project,
                &always_write_options(output_root.clone(), None),
            ));
            for (path, source) in [
                ("_moth/js/binding.js", &project_source),
                ("_moth/js/providers/binding.js", &package_source),
            ] {
                assert_eq!(
                    fs::read(output_root.join(path)).expect("provider output must exist"),
                    fs::read(source).expect("provider source must exist")
                );
            }
            assert_eq!(
                fs::read_to_string(output_root.join(glue_path)).expect("glue output must exist"),
                glue
            );
            assert_eq!(
                fs::read_to_string(output_root.join("index.html")).expect("page output must exist"),
                page
            );
            // Keep actual import/export order but discard registry-local aliases and the
            // absolute-path-derived glue filename when comparing equivalent checkouts.
            let mut wrapper_symbols = HashMap::new();
            let mut import_order = Vec::new();
            for line in glue
                .lines()
                .filter_map(|line| line.strip_prefix("import { "))
            {
                let (bindings, path) = line
                    .split_once(" } from ")
                    .expect("glue imports have named bindings");
                let mut exports = Vec::new();
                for binding in bindings.split(", ") {
                    let (export, alias) =
                        binding.split_once(" as ").expect("raw import has an alias");
                    let suffix = alias
                        .strip_prefix("__moth_external_fn")
                        .expect("provider imports have generated local aliases");
                    wrapper_symbols.insert(
                        format!("__moth_glue_fn{suffix}"),
                        (path.to_owned(), export.to_owned()),
                    );
                    exports.push(export);
                }
                import_order.push((path.to_owned(), exports.join(",")));
            }
            assert_eq!(
                import_order.len(),
                2,
                "both provider imports must enter the order comparison"
            );
            let wrapper_order = glue
                .lines()
                .filter_map(|line| line.strip_prefix("export function "))
                .map(|line| {
                    let (name, _) = line.split_once('(').expect("wrapper has parameters");
                    wrapper_symbols
                        .remove(name)
                        .expect("every emitted wrapper must select one imported provider export")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                wrapper_order.len(),
                2,
                "both selected wrappers must enter the order comparison"
            );
            assert!(
                wrapper_symbols.is_empty(),
                "each selected export needs a wrapper"
            );
            let asset_order = built
                .project
                .deferred_resources
                .iter()
                .map(|asset| portable_path_text(&asset.relative_output_path))
                .collect::<Vec<_>>();
            let rendered =
                crate::compiler_tests::integration_test_runner::assertions::execute_html_harness_for_test(
                    &mut built,
                )
                .expect("the existing Node harness should execute both providers");
            assert_eq!(rendered.runtime_error_message(), None);
            let rendered = rendered.combined_output();
            assert_eq!(rendered, "project=project:17|package=package:29|marker:43");
            let signature = (import_order, wrapper_order, asset_order, rendered);
            if let Some(baseline) = &baseline {
                assert_eq!(
                    &signature, baseline,
                    "topology and creation/registration order must not change linkage"
                );
            } else {
                baseline = Some(signature);
            }
        }
    }
}

#[test]
fn build_html_normal_sibling_name_is_not_visible_from_child_module() {
    let temporary = tempfile::tempdir().expect("should create sibling fixtures");
    let mut baseline = None;
    for reversed in [false, true] {
        let root = temporary
            .path()
            .join(if reversed { "reverse" } else { "forward" });
        let mut sources = [
            (
                "config.moth",
                "project #= (\n    name = \"sibling_boundary\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
            ),
            ("src/@page.moth", "@consumer value\n[:[value]]\n"),
            (
                "src/consumer/@mod.moth",
                "export:\n    @provider value\n;\n",
            ),
            (
                "src/provider/@mod.moth",
                "export:\n    value #= \"must not escape through a sibling lookup\"\n;\n",
            ),
        ];
        if reversed {
            sources.reverse();
        }
        for (path, source) in sources {
            let path = root.join(path);
            fs::create_dir_all(path.parent().expect("fixture source has a parent"))
                .expect("should create fixture directories");
            fs::write(path, source).expect("should write sibling fixture");
        }
        let builder = HtmlProjectBuilder::new();
        let result = build_project(
            &builder,
            root.to_str().expect("fixture root should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {});
        let codes = diagnosed_reports(&result)
            .iter()
            .map(|(_, view)| view.descriptor().code)
            .collect::<Vec<_>>();
        assert_eq!(codes, vec!["MOTH-IMPORT-0005"]);
        if let Some(baseline) = &baseline {
            assert_eq!(&codes, baseline);
        } else {
            baseline = Some(codes);
        }
    }
}

#[test]
fn build_html_project_shared_support_provider_keeps_page_wrappers_isolated() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path();
    let src = root.join("src");
    let shared = src.join("shared");
    let about = src.join("about");
    fs::create_dir_all(&shared).expect("should create support package");
    fs::create_dir_all(&about).expect("should create second page");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"shared_pages\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@shared root_value\n[:[root_value()]]\n",
    )
    .expect("should write homepage");
    fs::write(
        about.join("@page.moth"),
        "@shared about_value\n[:[about_value()]]\n",
    )
    .expect("should write about page");
    fs::write(
        shared.join("+package.moth"),
        "export:\n    @binding.js root_value, about_value, unused_value\n;\n",
    )
    .expect("should write shared support facade");
    let source = shared.join("binding.js");
    fs::write(
        &source,
        "/** @moth.sig root_value || -> String */\nexport function rootValue() { return \"root\"; }\n/** @moth.sig about_value || -> String */\nexport function aboutValue() { return \"about\"; }\n/** @moth.sig unused_value || -> String */\nexport function unusedValue() { return \"unused\"; }\n",
    )
    .expect("should write shared provider");

    let builder = HtmlProjectBuilder::new();
    let mut result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );
    assert_deferred_provider_sources(
        &mut result.project,
        &[("_moth/js/shared/binding.js", source.as_path())],
    );

    let outputs = BuiltOutputs::index(&result.project);
    let glue_paths = outputs
        .paths()
        .into_iter()
        .filter(|path| path.starts_with(GLUE_MODULE_PREFIX))
        .collect::<Vec<_>>();
    assert_eq!(
        glue_paths.len(),
        2,
        "each page needs its own wrapper selection"
    );
    assert_eq!(
        outputs
            .paths()
            .into_iter()
            .filter(|path| path.ends_with(".html"))
            .collect::<Vec<_>>(),
        vec!["about/index.html", "index.html"]
    );
    let mut selected_paths = Vec::new();
    for (page_path, relative_prefix, export) in [
        ("index.html", "./", "rootValue"),
        ("about/index.html", "../", "aboutValue"),
    ] {
        let page = html_text(outputs.at(page_path));
        let selected = glue_paths
            .iter()
            .filter(|path| page.contains(&format!("from \"{relative_prefix}{path}\"")))
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(
            selected.len(),
            1,
            "'{page_path}' must import its actual emitted glue"
        );
        let glue_path = selected[0];
        assert_page_imports_glue(
            page,
            &format!("{relative_prefix}{glue_path}"),
            js_text(outputs.at(glue_path)),
        );
        assert_glue_exports(
            js_text(outputs.at(glue_path)),
            &[("../shared/binding.js", &[export])],
        );
        selected_paths.push(glue_path);
    }
    assert_ne!(selected_paths[0], selected_paths[1]);
}

#[test]
fn build_html_project_binding_topology_emits_only_demanded_wrappers_and_whole_assets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/cases/html_js_binding_runtime_topology/input");
    let builder = HtmlProjectBuilder::new();
    let mut result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("fixture path should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let inner_source = root.join("src/outer/inner/binding.js");
    let alternate_source = root.join("src/alternate/binding.js");
    assert_deferred_provider_sources(
        &mut result.project,
        &[
            ("_moth/js/outer/inner/binding.js", inner_source.as_path()),
            ("_moth/js/alternate/binding.js", alternate_source.as_path()),
        ],
    );
    let outputs = BuiltOutputs::index(&result.project);
    outputs.none_matching("eager dormant provider asset", |path| {
        ["private_only.js", "branch_only.js", "assertion_only.js"]
            .iter()
            .any(|name| path.ends_with(name))
    });
    outputs.none_matching("runtime helper module", |path| {
        path.starts_with("_moth/js/runtime/")
    });
    let glue_path = outputs.exactly_one_path("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    });
    let page = html_text(outputs.at("index.html"));
    assert_page_imports_glue(
        page,
        &format!("./{glue_path}"),
        js_text(outputs.at(glue_path)),
    );
    assert!(!page.contains("<script type=\"importmap\">"));
    assert_glue_exports(
        js_text(outputs.at(glue_path)),
        &[
            (
                "../outer/inner/binding.js",
                &[
                    "recordEvent",
                    "chooseOutcome",
                    "observeState",
                    "generatedLabel",
                    "readLimit",
                ],
            ),
            ("../alternate/binding.js", &["alternateValue"]),
        ],
    );
}

#[test]
fn build_html_project_fallible_js_with_runtime_helper_emits_runtime_import_map() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    let facade = src.join("drawing");
    fs::create_dir_all(&facade).expect("should create facade");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing get_number\nvalue = get_number() catch:\n    then 0\n;\n",
    )
    .expect("should write page");
    fs::write(
        facade.join("@mod.moth"),
        "export:\n    @provider.js get_number\n;\n",
    )
    .expect("should write pure re-export facade");
    fs::write(
        facade.join("provider.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> I32, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let mut result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let source = facade.join("provider.js");
    assert_deferred_provider_sources(
        &mut result.project,
        &[("_moth/js/drawing/provider.js", source.as_path())],
    );

    let outputs = BuiltOutputs::index(&result.project);
    // Exactly one runtime module at its registered path, not merely some path ending in it.
    let runtime = js_text(outputs.at(RUNTIME_MODULE_PATH));
    assert!(
        runtime.contains("export function mothOk"),
        "the registered runtime module should define the helper the provider imports"
    );

    let html = html_text(outputs.at("index.html"));
    // This artifact contract is separate from Node: the normal harness rejects import maps.
    let map = html
        .split_once("<script type=\"importmap\">")
        .and_then(|(_, remaining)| remaining.split_once("</script>"))
        .map(|(map, _)| map)
        .expect("the page should contain a runtime import map");
    let map: serde_json::Value =
        serde_json::from_str(map).expect("import map should be valid JSON");
    assert_eq!(
        map,
        serde_json::json!({ "imports": { "@moth/runtime": "./_moth/js/runtime/moth-runtime.js" } })
    );
    let glue_path = outputs.exactly_one_path("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    });
    assert_page_imports_glue(
        html,
        &format!("./{glue_path}"),
        js_text(outputs.at(glue_path)),
    );
    assert_glue_exports(
        js_text(outputs.at(glue_path)),
        &[("../drawing/provider.js", &["getNumber"])],
    );
}

#[test]
fn build_html_project_non_fallible_js_with_runtime_helper_emits_runtime_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number()\nio.line([: [value]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> I32\n */\nexport function getNumber() { return mothOk(7).value; }\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let outputs = BuiltOutputs::index(&result.project);
    outputs.at(RUNTIME_MODULE_PATH);

    let html = html_text(outputs.at("index.html"));
    assert!(html.contains("<script type=\"importmap\">"));
    assert!(html.contains("\"@moth/runtime\""));
}

#[test]
fn build_html_project_fallible_js_without_runtime_import_does_not_emit_runtime_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number() catch:\n    then 0\n;\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "/**\n * @moth.sig get_number || -> I32, Error!\n */\nexport function getNumber() { return { ok: true, value: 7 }; }\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let outputs = BuiltOutputs::index(&result.project);
    outputs.none_matching("runtime module", |path| {
        path.starts_with("_moth/js/runtime/")
    });
}

#[test]
fn build_html_project_unreachable_provider_js_import_does_not_emit_runtime_artifacts() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nunused || -> I32, Error!:\n    return get_number()!\n;\nvalue = 1\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> I32, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let outputs = BuiltOutputs::index(&result.project);
    outputs.none_matching("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    });
    outputs.none_matching("runtime module", |path| {
        path.starts_with("_moth/js/runtime/")
    });

    let html = html_text(outputs.at("index.html"));
    assert!(
        !html.contains("<script type=\"module\">"),
        "unreachable provider-created JS calls should not force a module script"
    );
    assert!(
        !html.contains("import { __moth_glue_fn"),
        "unreachable provider-created JS calls should not add a glue preamble"
    );
    assert!(
        !html.contains("<script type=\"importmap\">"),
        "unreachable provider-created JS calls should not emit an import map"
    );
}

#[test]
fn build_html_project_unreachable_html_canvas_helper_dependency_does_not_emit_runtime_artifacts() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        r#"@html canvas, get_canvas_context
#[canvas:
  [$insert("id"):unused_canvas]
  [$insert("style"):
    width: 320px;
    height: 180px;
  ]
]
"#,
    )
    .expect("should write page");

    let builder = HtmlProjectBuilder::new();
    let result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let outputs = BuiltOutputs::index(&result.project);
    outputs.none_matching("built-in canvas asset", |path| {
        path.starts_with("_moth/js/canvas.js")
    });
    outputs.none_matching("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    });
    outputs.none_matching("runtime module", |path| {
        path.starts_with("_moth/js/runtime/")
    });

    assert!(
        !result
            .project
            .deferred_resources
            .iter()
            .any(
                |resource| portable_path_text(&resource.relative_output_path)
                    .starts_with("_moth/js/canvas.js")
            ),
        "an unreachable canvas helper should not defer a canvas asset output"
    );

    let html = html_text(outputs.at("index.html"));
    assert!(html.contains("<canvas"));
    assert!(!html.contains("<script type=\"module\">"));
    assert!(!html.contains("<script type=\"importmap\">"));
}

#[test]
fn build_html_project_web_canvas_emits_builtin_js_asset_and_glue() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@web/canvas\nrun |id String| -> String, Error!:\n    canvas_ref = canvas.get_canvas(id)!\n    ctx ~= canvas.context_2d(canvas_ref)!\n    canvas.set_line_width(~ctx, 2.0)\n    gradient ~= canvas.create_linear_gradient(ctx, 0.0, 0.0, 10.0, 0.0)!\n    canvas.add_color_stop(~gradient, 0.0, \"red\")!\n    canvas.set_fill_gradient(~ctx, gradient)\n    canvas.fill_rect(~ctx, 0.0, 0.0, 10.0, 10.0)\n    return \"ok\"\n;\nresult = run(\"game\") catch:\n    then \"error\"\n;\nio.line([: [result]])\n",
    )
    .expect("should write page");

    let builder = HtmlProjectBuilder::new();
    let result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let mut result = result;
    let canvas_output_path = deferred_canvas_output_path(&result.project);
    let canvas_asset = deferred_resource_text(&mut result.project, &canvas_output_path);

    let outputs = BuiltOutputs::index(&result.project);
    assert!(
        !outputs
            .paths()
            .iter()
            .any(|path| path.starts_with("_moth/js/canvas.js")),
        "the built-in canvas asset must be a deferred resource, not an eager artifact"
    );
    assert!(canvas_asset.contains("export function getCanvas"));
    assert!(canvas_asset.contains("@moth.opaque Canvas2d"));
    assert!(canvas_asset.contains("@moth.opaque CanvasGradient"));
    assert!(canvas_asset.contains("export function createLinearGradient"));
    assert!(canvas_asset.contains("export function imageDataSetPixel"));

    let glue = js_text(outputs.exactly_one("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    }));
    assert!(glue.contains("getCanvas as __moth_external_fn"));
    assert!(glue.contains("fillRect as __moth_external_fn"));
    assert!(glue.contains("createLinearGradient as __moth_external_fn"));
    assert!(glue.contains("addColorStop as __moth_external_fn"));
    assert!(glue.contains("setFillGradient as __moth_external_fn"));
    assert!(
        glue.contains("from \"../canvas.js\""),
        "glue imports should be relative to the glue module"
    );

    let html = html_text(outputs.at("index.html"));
    assert!(
        html.contains("<script type=\"module\">"),
        "reachable @web/canvas glue should make the inline bundle a module script"
    );
    assert!(
        html.contains("import { __moth_glue_"),
        "reachable @web/canvas calls should add a glue import preamble"
    );
    assert!(
        html.contains("<script type=\"importmap\">"),
        "@web/canvas imports runtime helpers, so HTML should include an import map"
    );

    outputs.at(RUNTIME_MODULE_PATH);
}

#[test]
fn build_html_project_html_canvas_helper_emits_builtin_js_asset_and_glue() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@html get_canvas_context\ndraw || -> String, Error!:\n    context = get_canvas_context(\"game_canvas\")!\n    return \"ok\"\n;\nresult = draw() catch:\n    then \"error\"\n;\nio.line([: [result]])\n",
    )
    .expect("should write page");

    let builder = HtmlProjectBuilder::new();
    let mut result = successful_compilation_value(
        build_project(
            &builder,
            root.to_str().expect("temp dir should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    let canvas_output_path = deferred_canvas_output_path(&result.project);
    let canvas_asset = deferred_resource_text(&mut result.project, &canvas_output_path);

    let outputs = BuiltOutputs::index(&result.project);
    assert!(
        !outputs
            .paths()
            .iter()
            .any(|path| path.starts_with("_moth/js/canvas.js")),
        "the built-in canvas asset must be a deferred resource, not an eager artifact"
    );
    assert!(canvas_asset.contains("export function getCanvas"));
    assert!(canvas_asset.contains("export function context2d"));

    let glue = js_text(outputs.exactly_one("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    }));
    assert!(glue.contains("getCanvas as __moth_external_fn"));
    assert!(glue.contains("context2d as __moth_external_fn"));
    assert!(
        glue.contains("from \"../canvas.js\""),
        "glue imports should be relative to the glue module"
    );

    let html = html_text(outputs.at("index.html"));
    assert!(html.contains("<script type=\"module\">"));
    assert!(html.contains("import { __moth_glue_"));
    assert!(html.contains("<script type=\"importmap\">"));

    outputs.at(RUNTIME_MODULE_PATH);
}

#[test]
fn build_project_discards_backend_warnings_when_multimodule_diagnostics_fail() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src_dir = root.join("src");
    let docs_dir = src_dir.join("docs");
    fs::create_dir_all(&docs_dir).expect("should create docs directory");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src_dir.join("@page.moth"), "value = 1\n").expect("should write homepage");
    fs::write(docs_dir.join("@page.moth"), "value = 2\n").expect("should write docs page");

    let builder = MultiModuleDiagnosticBuilder;
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .unwrap_or_else(|never| match never {});
    let reports = diagnosed_reports(&result);

    let errors = reports
        .iter_severity(crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error)
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(
            errors[0].1.primary_site(),
            crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent
        ),
        "path-only backend diagnostics are spanless",
    );
}
#[test]
fn build_html_project_literal_const_change_reaches_next_build_without_asset() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    let helper = src.join("helper");
    fs::create_dir_all(&helper).expect("should create helper dir");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@helper as helper_ns\n#[:constant=[helper_ns.triangles_ns is 4]]\n",
    )
    .expect("should write page");
    fs::write(
        helper.join("@mod.moth"),
        "export:\n    @gl_constants.js primitive_triangles as triangles_ns\n;\n",
    )
    .expect("should write helper");
    fs::write(
        helper.join("gl_constants.js"),
        "import { mothOk } from \"@moth/runtime\";\n/** @moth.const primitive_triangles U32 */\nexport const TRIANGLES = 4;\nthrow new Error(\"constant-only modules must not execute their JS source at compile time\");\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let entry = root.to_str().expect("temp dir should be UTF-8");

    let first = successful_compilation_value(
        build_project(&builder, entry, &[], &BuildConfigInputSet::new())
            .unwrap_or_else(|never| match never {}),
    );
    let first_outputs = BuiltOutputs::index(&first.project);
    let first_html = html_text(first_outputs.at("index.html"));
    assert!(
        first_html.contains("constant=true"),
        "the first build should fold the authored literal: {first_html}"
    );
    first_outputs.none_matching("JS artifact", |path| path.ends_with(".js"));

    fs::write(
        helper.join("gl_constants.js"),
        "import { mothOk } from \"@moth/runtime\";\n/** @moth.const primitive_triangles U32 */\nexport const TRIANGLES = 5;\nthrow new Error(\"constant-only modules must not execute their JS source at compile time\");\n",
    )
    .expect("should rewrite literal");

    let second = successful_compilation_value(
        build_project(&builder, entry, &[], &BuildConfigInputSet::new())
            .unwrap_or_else(|never| match never {}),
    );
    let second_outputs = BuiltOutputs::index(&second.project);
    let second_html = html_text(second_outputs.at("index.html"));
    assert!(
        second_html.contains("constant=false"),
        "the next build should observe the rewritten literal: {second_html}"
    );
    second_outputs.none_matching("JS artifact", |path| path.ends_with(".js"));

    for project in [&first.project, &second.project] {
        assert!(
            !project.deferred_resources.iter().any(|resource| {
                resource
                    .relative_output_path
                    .extension()
                    .is_some_and(|extension| extension == "js")
            }),
            "constant-only dependencies must not emit deferred JS assets"
        );
    }
}

#[test]
fn build_html_project_mixed_const_and_function_emits_single_glue_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@gl_mixed.js primitive_triangles, u32_max, double_u32\n\ndoubled U32 = double_u32(primitive_triangles)\ndoubled_again U32 = double_u32(doubled)\n[:mixed doubled=[doubled]/[doubled_again] const=[primitive_triangles is 4]]\n",
    )
    .expect("should write page");
    fs::write(
        src.join("gl_mixed.js"),
        "/** @moth.const primitive_triangles U32 */\nexport const TRIANGLES = 4;\n/** @moth.const u32_max U32 */\nexport const U32_MAX = 4294967295;\n/** @moth.sig double_u32 |value U32| -> U32 */\nexport function doubleU32(value) {\n    return value * 2;\n}\n",
    )
    .expect("should write js");

    let builder = HtmlProjectBuilder::new();
    let entry = root.to_str().expect("temp dir should be UTF-8");
    let built = successful_compilation_value(
        build_project(&builder, entry, &[], &BuildConfigInputSet::new())
            .unwrap_or_else(|never| match never {}),
    );
    let outputs = BuiltOutputs::index(&built.project);

    // Exactly one glue module across multiple semantic uses of one provider module:
    // a second glue module would mean split emission rather than shared lowering.
    let glue_path = outputs
        .exactly_one_path("generated glue module", |path| {
            path.starts_with(GLUE_MODULE_PREFIX)
        })
        .to_owned();
    let html = html_text(outputs.at("index.html"));
    assert!(
        html.contains(&format!("from \"./{glue_path}\"")),
        "the page should import the emitted glue module '{glue_path}': {html}"
    );

    // The provider JS asset is a deferred resource output at its declared stable path
    // `_moth/js/gl_mixed.js` (provider path owner: logical source spelling under
    // `_moth/js/`), and the emitted glue imports that same file.
    let provider_paths: Vec<String> = built
        .project
        .deferred_resources
        .iter()
        .map(|resource| portable_path_text(&resource.relative_output_path))
        .filter(|path| path.starts_with("_moth/js/gl_mixed.js"))
        .collect();
    assert_eq!(
        provider_paths,
        vec!["_moth/js/gl_mixed.js".to_owned()],
        "the provider asset should emit once at its declared path"
    );
    let glue = js_text(outputs.exactly_one("generated glue module", |path| {
        path.starts_with(GLUE_MODULE_PREFIX)
    }));
    assert!(
        glue.contains("gl_mixed.js"),
        "the single glue module should import the provider asset: {glue}"
    );
}

/// Write a directory project whose pages under `page_dirs` all call `page_export` from one
/// fallible provider. An empty directory is the homepage.
fn write_shared_glue_project(root: &Path, page_dirs: &[&str], page_export: &str) {
    let src = root.join("src");
    let shared = src.join("shared");
    fs::create_dir_all(&shared).expect("should create provider facade");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"shared_glue\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        shared.join("+package.moth"),
        "export:\n    @provider.js get_number, get_other\n;\n",
    )
    .expect("should write provider facade");
    fs::write(
        shared.join("provider.js"),
        "import { mothOk } from \"@moth/runtime\";\n/** @moth.sig get_number || -> I32, Error! */\nexport function getNumber() { return mothOk(7); }\n/** @moth.sig get_other || -> I32, Error! */\nexport function getOther() { return mothOk(8); }\n",
    )
    .expect("should write provider");
    for page_dir in page_dirs {
        let page_root = src.join(page_dir);
        fs::create_dir_all(&page_root).expect("should create page directory");
        fs::write(
            page_root.join("@page.moth"),
            format!("@shared {page_export}\nvalue = {page_export}() catch:\n    then 0\n;\n"),
        )
        .expect("should write page");
    }
}

fn build_shared_glue_project(root: &Path, flags: &[Flag]) -> BuildResult {
    successful_compilation_value(
        build_project(
            &HtmlProjectBuilder::new(),
            root.to_str().expect("temp dir should be UTF-8"),
            flags,
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    )
}

/// Each page's emitted HTML path and the relative prefix from that page back to the output root.
fn page_path_and_root_prefix(page_dir: &str) -> (String, String) {
    if page_dir.is_empty() {
        return ("index.html".to_owned(), "./".to_owned());
    }
    (
        format!("{page_dir}/index.html"),
        "../".repeat(page_dir.split('/').count()),
    )
}

#[test]
fn build_html_project_identical_wrapper_selections_share_one_glue_module() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let mut glue_names = Vec::new();
    // Different project roots, route names and discovery orders select the same wrappers, so
    // the source-derived glue name must not change.
    for (layout, page_dirs) in [
        ("first", ["", "about", "docs/guide"]),
        ("second", ["", "zeta", "alpha/beta"]),
    ] {
        let root = temp.path().join(layout);
        write_shared_glue_project(&root, &page_dirs, "get_number");
        let built = build_shared_glue_project(&root, &[]);
        let outputs = BuiltOutputs::index(&built.project);
        let glue_path = outputs.exactly_one_path("shared glue module", |path| {
            path.starts_with(GLUE_MODULE_PREFIX)
        });
        let glue = js_text(outputs.at(glue_path));
        assert_glue_exports(glue, &[("../shared/provider.js", &["getNumber"])]);
        for page_dir in page_dirs {
            let (page_path, root_prefix) = page_path_and_root_prefix(page_dir);
            assert_page_imports_glue(
                html_text(outputs.at(&page_path)),
                &format!("{root_prefix}{glue_path}"),
                glue,
            );
        }
        glue_names.push(glue_path.to_owned());
    }
    assert_eq!(glue_names[0], glue_names[1]);
}

#[test]
fn build_html_project_release_wrappers_get_their_own_glue_name() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    write_shared_glue_project(temp.path(), &["", "docs/guide"], "get_number");
    let mut glue_sources = Vec::new();
    for flags in [&[][..], &[Flag::Release][..]] {
        let built = build_shared_glue_project(temp.path(), flags);
        let outputs = BuiltOutputs::index(&built.project);
        let glue_path = outputs.exactly_one_path("shared glue module", |path| {
            path.starts_with(GLUE_MODULE_PREFIX)
        });
        outputs.at(RUNTIME_MODULE_PATH);
        let page = html_text(outputs.at("docs/guide/index.html"));
        assert!(
            page.contains("\"@moth/runtime\": \"../../_moth/js/runtime/moth-runtime.js\""),
            "nested pages must map the runtime relative to themselves: {page}"
        );
        let glue = js_text(outputs.at(glue_path));
        assert_page_imports_glue(page, &format!("../../{glue_path}"), glue);
        glue_sources.push((glue_path.to_owned(), glue.to_owned()));
    }
    let [(dev_path, dev_source), (release_path, release_source)] = glue_sources.as_slice() else {
        unreachable!("one glue module per profile");
    };
    assert_ne!(
        dev_source, release_source,
        "fallible release wrappers use release error fields"
    );
    assert_ne!(dev_path, release_path);
}

#[test]
fn rebuild_with_new_wrapper_selection_replaces_stale_glue_and_keeps_unowned_files() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path();
    let glue_directory = root.join("dev").join(GLUE_MODULE_PREFIX);
    let mut previous_glue = None;
    for page_export in ["get_number", "get_other"] {
        write_shared_glue_project(root, &["", "about"], page_export);
        let built = build_shared_glue_project(root, &[]);
        let plan = built
            .directory_output_plan
            .clone()
            .expect("directory build should carry its output plan");
        write_project_outputs(
            &built.project,
            &WriteOptions {
                output_plan: OutputPlan::Directory(plan),
                write_mode: WriteMode::AlwaysWrite,
            },
        )
        .expect("build outputs should write");

        let mut glue_files = fs::read_dir(&glue_directory)
            .expect("glue directory should exist")
            .map(|entry| entry.expect("glue entry should read").file_name())
            .map(|name| name.into_string().expect("glue name should be UTF-8"))
            .filter(|name| name != "notes.txt")
            .collect::<Vec<_>>();
        assert_eq!(glue_files.len(), 1, "one shared glue file on disk");
        let glue_file = glue_files.pop().expect("one glue file");
        let manifest =
            fs::read_to_string(root.join("dev/.moth_manifest")).expect("manifest should exist");
        assert_eq!(
            manifest
                .lines()
                .map(|line| portable_path_text(Path::new(line)))
                .filter(|line| line.starts_with(GLUE_MODULE_PREFIX))
                .collect::<Vec<_>>(),
            vec![format!("{GLUE_MODULE_PREFIX}{glue_file}")]
        );
        if let Some(previous) = previous_glue.replace(glue_file.clone()) {
            assert_ne!(previous, glue_file, "a new selection needs a new glue name");
            assert_path_missing(&glue_directory.join(previous));
            assert!(
                glue_directory.join("notes.txt").is_file(),
                "cleanup must keep files the manifest never owned"
            );
        } else {
            fs::write(glue_directory.join("notes.txt"), "unowned")
                .expect("should write unowned file");
        }
    }
}
