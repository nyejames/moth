//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::utilities::basic::{normalize_path, portable_path_text};
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use std::fs;

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
        let result = build_project(&builder, "main.moth", &[]).expect("build should succeed");

        assert!(
            !result.project.output_files.is_empty(),
            "single-file build should compile reachable imported files"
        );
    }
}

#[test]
fn build_html_project_local_js_import_emits_generated_glue() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@drawing.js draw\nvalue = draw()\n",
    )
    .expect("should write page");
    fs::write(
        root.join("drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 7; }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("project-local JS import should build through generated glue");

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
    assert!(html.contains("<script type=\"module\">"));
    assert!(html.contains("import { __moth_glue_fn"));
    assert!(html.contains("from \"./_moth/js/glue/module-"));

    let glue = result
        .project
        .output_files
        .iter()
        .find_map(|file| {
            let path = portable_path_text(file.relative_output_path());
            if !path.contains("_moth/js/glue/") {
                return None;
            }
            match file.file_kind() {
                FileKind::Js(source) => Some(source.as_str()),
                _ => None,
            }
        })
        .expect("build should emit generated glue");
    assert!(glue.contains("import { draw as __moth_external_fn"));
    assert!(glue.contains("return __moth_external_fn"));
}

#[test]
fn build_html_project_fallible_js_with_runtime_helper_emits_runtime_import_map() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number() catch:\n    then 0\n;\n",
    )
    .expect("should write page");
    fs::write(
        root.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("fallible project-local JS import should build through generated glue");

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "JS files that import runtime helpers should emit the registered runtime module"
    );

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
    assert!(html.contains("<script type=\"importmap\">"));
    assert!(html.contains("\"@moth/runtime\""));
    assert!(html.contains("\"./_moth/js/runtime/moth-runtime.js\""));
}

#[test]
fn build_html_project_non_fallible_js_with_runtime_helper_emits_runtime_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number()\nio.line([: [value]])\n",
    )
    .expect("should write page");
    fs::write(
        root.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int\n */\nexport function getNumber() { return mothOk(7).value; }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("non-fallible project-local JS import with runtime helper should build");

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "non-fallible JS with runtime helper import should emit the registered runtime module"
    );

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
    assert!(html.contains("<script type=\"importmap\">"));
    assert!(html.contains("\"@moth/runtime\""));
}

#[test]
fn build_html_project_fallible_js_without_runtime_import_does_not_emit_runtime_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@drawing.js get_number\nvalue = get_number() catch:\n    then 0\n;\n",
    )
    .expect("should write page");
    fs::write(
        root.join("drawing.js"),
        "/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return { ok: true, value: 7 }; }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("fallible project-local JS import without runtime helper should build");

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        !output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "fallible JS without runtime helper import should not emit the registered runtime module"
    );
}

#[test]
fn build_html_project_unreachable_provider_js_import_does_not_emit_runtime_artifacts() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@drawing.js get_number\nunused || -> Int, Error!:\n    return get_number()!\n;\nvalue = 1\n",
    )
    .expect("should write page");
    fs::write(
        root.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("unreachable project-local JS import should not request runtime artifacts");

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        !output_paths
            .iter()
            .any(|path| portable_path_text(path).contains("_moth/js/glue/")),
        "unreachable provider-created JS calls should not emit generated glue"
    );
    assert!(
        !output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "unreachable provider-created JS calls should not emit runtime modules"
    );

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
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

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
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
    )
    .expect("unused @html canvas helper should not request runtime artifacts");

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        !output_paths
            .iter()
            .any(|path| portable_path_text(path).starts_with("_moth/js/canvas-")),
        "unused @html canvas helper should not emit the built-in canvas asset"
    );
    assert!(
        !output_paths
            .iter()
            .any(|path| portable_path_text(path).contains("_moth/js/glue/")),
        "unused @html canvas helper should not emit generated glue"
    );
    assert!(
        !output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "unused @html canvas helper should not emit the registered runtime module"
    );

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
    assert!(html.contains("<canvas"));
    assert!(!html.contains("<script type=\"module\">"));
    assert!(!html.contains("<script type=\"importmap\">"));
}

#[test]
fn build_html_project_web_canvas_emits_builtin_js_asset_and_glue() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@web/canvas\nrun |id String| -> String, Error!:\n    canvas_ref = canvas.get_canvas(id)!\n    ctx ~= canvas.context_2d(canvas_ref)!\n    canvas.set_line_width(~ctx, 2.0)\n    gradient ~= canvas.create_linear_gradient(ctx, 0.0, 0.0, 10.0, 0.0)!\n    canvas.add_color_stop(~gradient, 0.0, \"red\")!\n    canvas.set_fill_gradient(~ctx, gradient)\n    canvas.fill_rect(~ctx, 0.0, 0.0, 10.0, 10.0)\n    return \"ok\"\n;\nresult = run(\"game\") catch:\n    then \"error\"\n;\nio.line([: [result]])\n",
    )
    .expect("should write page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("@web/canvas should build through generated glue");

    let canvas_asset = result
        .project
        .output_files
        .iter()
        .find_map(|file| {
            let path = portable_path_text(file.relative_output_path());
            if !path.starts_with("_moth/js/canvas-") {
                return None;
            }
            match file.file_kind() {
                FileKind::Js(source) => Some(source.as_str()),
                _ => None,
            }
        })
        .expect("@web/canvas should emit its built-in JS asset");
    assert!(canvas_asset.contains("export function getCanvas"));
    assert!(canvas_asset.contains("@moth.opaque Canvas2d"));
    assert!(canvas_asset.contains("@moth.opaque CanvasGradient"));
    assert!(canvas_asset.contains("export function createLinearGradient"));
    assert!(canvas_asset.contains("export function imageDataSetPixel"));

    let glue = result
        .project
        .output_files
        .iter()
        .find_map(|file| {
            let path = portable_path_text(file.relative_output_path());
            if !path.contains("_moth/js/glue/") {
                return None;
            }
            match file.file_kind() {
                FileKind::Js(source) => Some(source.as_str()),
                _ => None,
            }
        })
        .expect("@web/canvas calls should emit generated glue");
    assert!(glue.contains("getCanvas as __moth_external_fn"));
    assert!(glue.contains("fillRect as __moth_external_fn"));
    assert!(glue.contains("createLinearGradient as __moth_external_fn"));
    assert!(glue.contains("addColorStop as __moth_external_fn"));
    assert!(glue.contains("setFillGradient as __moth_external_fn"));
    assert!(
        glue.contains("from \"../canvas-"),
        "glue imports should be relative to the glue module"
    );

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
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

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "@web/canvas imports runtime helpers, so the registered runtime module should be emitted"
    );
}

#[test]
fn build_html_project_html_canvas_helper_emits_builtin_js_asset_and_glue() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("config.moth"), "project #= \"html\"\n").expect("should write config");
    fs::write(
        root.join("@page.moth"),
        "@html get_canvas_context\ndraw || -> String, Error!:\n    context = get_canvas_context(\"game_canvas\")!\n    return \"ok\"\n;\nresult = draw() catch:\n    then \"error\"\n;\nio.line([: [result]])\n",
    )
    .expect("should write page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("temp dir should be UTF-8"),
        &[],
    )
    .expect("reachable @html canvas helper should build through generated glue");

    let canvas_asset = result
        .project
        .output_files
        .iter()
        .find_map(|file| {
            let path = portable_path_text(file.relative_output_path());
            if !path.starts_with("_moth/js/canvas-") {
                return None;
            }
            match file.file_kind() {
                FileKind::Js(source) => Some(source.as_str()),
                _ => None,
            }
        })
        .expect("reachable @html canvas helper should emit its built-in JS asset");
    assert!(canvas_asset.contains("export function getCanvas"));
    assert!(canvas_asset.contains("export function context2d"));

    let glue = result
        .project
        .output_files
        .iter()
        .find_map(|file| {
            let path = portable_path_text(file.relative_output_path());
            if !path.contains("_moth/js/glue/") {
                return None;
            }
            match file.file_kind() {
                FileKind::Js(source) => Some(source.as_str()),
                _ => None,
            }
        })
        .expect("reachable @html canvas helper should emit generated glue");
    assert!(glue.contains("getCanvas as __moth_external_fn"));
    assert!(glue.contains("context2d as __moth_external_fn"));
    assert!(
        glue.contains("from \"../canvas-"),
        "glue imports should be relative to the glue module"
    );

    let html = result
        .project
        .output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(html) => Some(html.as_str()),
            _ => None,
        })
        .expect("build should emit HTML");
    assert!(html.contains("<script type=\"module\">"));
    assert!(html.contains("import { __moth_glue_"));
    assert!(html.contains("<script type=\"importmap\">"));

    let output_paths = result
        .project
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        output_paths
            .iter()
            .any(|path| path.ends_with("_moth/js/runtime/moth-runtime.js")),
        "@html canvas helper imports runtime helpers, so the registered runtime module should be emitted"
    );
}

#[test]
fn build_project_keeps_one_shared_string_table_for_multi_module_diagnostics() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src_dir = root.join("src");
    let docs_dir = src_dir.join("docs");
    fs::create_dir_all(&docs_dir).expect("should create docs directory");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");
    fs::write(src_dir.join("@page.moth"), "value = 1\n").expect("should write homepage");
    fs::write(docs_dir.join("@page.moth"), "value = 2\n").expect("should write docs page");

    let builder = ProjectBuilder::new(Box::new(MultiModuleDiagnosticBuilder));
    let Err(messages) = build_project(
        &builder,
        root.to_str().expect("temp dir path should be valid UTF-8"),
        &[],
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
