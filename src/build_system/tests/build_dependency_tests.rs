//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::build_system::output::output_path_identity;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::utilities::basic::{normalize_path, portable_path_text};
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
fn deferred_resource_text(
    project: &mut Project,
    relative_path: &str,
    string_table: &mut StringTable,
) -> String {
    let source_id = project
        .deferred_resources
        .iter()
        .find(|resource| portable_path_text(&resource.relative_output_path) == relative_path)
        .map(|resource| resource.source_id)
        .unwrap_or_else(|| panic!("expected a deferred resource at '{relative_path}'"));

    let bytes = project
        .resource_inputs
        .read_source(source_id, string_table)
        .unwrap_or_else(|error| {
            panic!("deferred resource '{relative_path}' should read from its source: {error:?}")
        });

    String::from_utf8(bytes.to_vec()).expect("deferred JS resource text should be UTF-8")
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
            warnings: Vec::new(),
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

        let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
        let result = build_project(&builder, "main.moth", &[], &BuildConfigInputSet::new())
            .expect("build should succeed");

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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@drawing.js draw\nvalue = draw()\n")
        .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 7; }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("project-local JS import should build through generated glue");

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
    assert!(glue.contains("return __moth_external_fn"));

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
fn build_html_project_fallible_js_with_runtime_helper_emits_runtime_import_map() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number() catch:\n    then 0\n;\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("fallible project-local JS import should build through generated glue");

    let outputs = BuiltOutputs::index(&result.project);
    // Exactly one runtime module at its registered path, not merely some path ending in it.
    let runtime = js_text(outputs.at(RUNTIME_MODULE_PATH));
    assert!(
        runtime.contains("mothOk"),
        "the registered runtime module should define the helper the provider imports"
    );

    let html = html_text(outputs.at("index.html"));
    assert!(html.contains("<script type=\"importmap\">"));
    assert!(html.contains("\"@moth/runtime\""));
    assert!(html.contains("\"./_moth/js/runtime/moth-runtime.js\""));
}

#[test]
fn build_html_project_non_fallible_js_with_runtime_helper_emits_runtime_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number()\nio.line([: [value]])\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int\n */\nexport function getNumber() { return mothOk(7).value; }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("non-fallible project-local JS import with runtime helper should build");

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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number() catch:\n    then 0\n;\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return { ok: true, value: 7 }; }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("fallible project-local JS import without runtime helper should build");

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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@drawing.js get_number\nunused || -> Int, Error!:\n    return get_number()!\n;\nvalue = 1\n",
    )
    .expect("should write page");
    fs::write(
        src.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("unreachable project-local JS import should not request runtime artifacts");

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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
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

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("unused @html canvas helper should not request runtime artifacts");

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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@web/canvas\nrun |id String| -> String, Error!:\n    canvas_ref = canvas.get_canvas(id)!\n    ctx ~= canvas.context_2d(canvas_ref)!\n    canvas.set_line_width(~ctx, 2.0)\n    gradient ~= canvas.create_linear_gradient(ctx, 0.0, 0.0, 10.0, 0.0)!\n    canvas.add_color_stop(~gradient, 0.0, \"red\")!\n    canvas.set_fill_gradient(~ctx, gradient)\n    canvas.fill_rect(~ctx, 0.0, 0.0, 10.0, 10.0)\n    return \"ok\"\n;\nresult = run(\"game\") catch:\n    then \"error\"\n;\nio.line([: [result]])\n",
    )
    .expect("should write page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("@web/canvas should build through generated glue");

    let mut result = result;
    let canvas_output_path = deferred_canvas_output_path(&result.project);
    let canvas_asset = deferred_resource_text(
        &mut result.project,
        &canvas_output_path,
        &mut result.string_table,
    );

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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@html get_canvas_context\ndraw || -> String, Error!:\n    context = get_canvas_context(\"game_canvas\")!\n    return \"ok\"\n;\nresult = draw() catch:\n    then \"error\"\n;\nio.line([: [result]])\n",
    )
    .expect("should write page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let mut result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("reachable @html canvas helper should build through generated glue");

    let canvas_output_path = deferred_canvas_output_path(&result.project);
    let canvas_asset = deferred_resource_text(
        &mut result.project,
        &canvas_output_path,
        &mut result.string_table,
    );

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
fn build_project_keeps_one_shared_string_table_for_multi_module_diagnostics() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src_dir = root.join("src");
    let docs_dir = src_dir.join("docs");
    fs::create_dir_all(&docs_dir).expect("should create docs directory");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src_dir.join("@page.moth"), "value = 1\n").expect("should write homepage");
    fs::write(docs_dir.join("@page.moth"), "value = 2\n").expect("should write docs page");

    let builder = ProjectBuilder::new(Box::new(MultiModuleDiagnosticBuilder));
    let Err(messages) = build_project(
        &builder,
        root.to_str().expect("temp dir path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    ) else {
        panic!("builder diagnostics should fail the build");
    };

    let errors = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(errors.len(), 1);
    let warnings = messages.warnings().collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1);

    assert_eq!(
        normalize_path(
            &errors[0]
                .primary_location
                .scope
                .to_path_buf(&messages.string_table)
        ),
        normalize_path(
            &fs::canonicalize(src_dir.join("@page.moth")).expect("homepage should canonicalize")
        )
    );
    assert_eq!(
        normalize_path(
            &warnings[0]
                .primary_location
                .scope
                .to_path_buf(&messages.string_table)
        ),
        normalize_path(
            &fs::canonicalize(docs_dir.join("@page.moth")).expect("docs page should canonicalize")
        )
    );
}
