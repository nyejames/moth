//! Tests for owner bucket mapping.
//!
//! WHAT: Validates that function names are correctly mapped to the expected
//! owner buckets, including Moth modules, third-party libraries, and
//! edge cases like empty and unknown names.
//!
//! WHY: Bucket mapping is the bridge between profiler output and actionable
//! source paths. Incorrect mapping would send agents to the wrong directory.

use super::*;

// ----------------------------
//  Moth module buckets
// ----------------------------

#[test]
fn tokenizer_prefix_maps_to_tokenization_bucket() {
    let result = match_owner_bucket("moth::compiler_frontend::tokenizer::tokenize");
    assert_eq!(result.label, "Tokenization");
    assert_eq!(
        result.suggested_paths,
        vec!["src/compiler_frontend/tokenizer/"]
    );
}

#[test]
fn headers_prefix_maps_to_header_parsing_bucket() {
    let result = match_owner_bucket("moth::compiler_frontend::headers::parse_header");
    assert_eq!(result.label, "Header parsing");
    assert_eq!(
        result.suggested_paths,
        vec!["src/compiler_frontend/headers/"]
    );
}

#[test]
fn module_dependencies_prefix_maps_to_dependency_sorting_bucket() {
    let result = match_owner_bucket("moth::compiler_frontend::module_dependencies::sort");
    assert_eq!(result.label, "Dependency sorting");
    assert_eq!(
        result.suggested_paths,
        vec!["src/compiler_frontend/module_dependencies.rs"]
    );
}

#[test]
fn ast_prefix_maps_to_ast_bucket() {
    let result = match_owner_bucket("moth::compiler_frontend::ast::resolve_type");
    assert_eq!(result.label, "AST");
    assert_eq!(result.suggested_paths, vec!["src/compiler_frontend/ast/"]);
}

#[test]
fn hir_prefix_maps_to_hir_bucket() {
    let result = match_owner_bucket("moth::compiler_frontend::hir::generate");
    assert_eq!(result.label, "HIR");
    assert_eq!(result.suggested_paths, vec!["src/compiler_frontend/hir/"]);
}

#[test]
fn borrow_checker_prefix_maps_to_borrow_validation_bucket() {
    let result = match_owner_bucket("moth::compiler_frontend::analysis::borrow_checker::validate");
    assert_eq!(result.label, "Borrow validation");
    assert_eq!(
        result.suggested_paths,
        vec!["src/compiler_frontend/analysis/borrow_checker/"]
    );
}

#[test]
fn build_system_prefix_maps_to_build_system_bucket() {
    let result = match_owner_bucket("moth::build_system::build");
    assert_eq!(result.label, "Build system");
    assert_eq!(result.suggested_paths, vec!["src/build_system/"]);
}

#[test]
fn js_backend_prefix_maps_to_js_bucket() {
    let result = match_owner_bucket("moth::backends::js::emit");
    assert_eq!(result.label, "JS backend");
    assert_eq!(result.suggested_paths, vec!["src/backends/js/"]);
}

#[test]
fn wasm_backend_prefix_maps_to_wasm_bucket() {
    let result = match_owner_bucket("moth::backends::wasm::emit");
    assert_eq!(result.label, "Wasm backend");
    assert_eq!(result.suggested_paths, vec!["src/backends/wasm/"]);
}

#[test]
fn html_project_prefix_maps_to_html_bucket() {
    let result = match_owner_bucket("moth::projects::html_project::build");
    assert_eq!(result.label, "HTML project builder");
    assert_eq!(result.suggested_paths, vec!["src/projects/html_project/"]);
}

// ----------------------------
//  Third-party fallback buckets
// ----------------------------

#[test]
fn std_prefix_maps_to_std_bucket() {
    let result = match_owner_bucket("std::alloc::alloc");
    assert_eq!(result.label, "std");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn core_prefix_maps_to_core_bucket() {
    let result = match_owner_bucket("core::ptr::drop_in_place");
    assert_eq!(result.label, "core");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn alloc_prefix_maps_to_alloc_bucket() {
    let result = match_owner_bucket("alloc::vec::Vec::push");
    assert_eq!(result.label, "alloc");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn rayon_prefix_maps_to_rayon_bucket() {
    let result = match_owner_bucket("rayon::ThreadPool::spawn");
    assert_eq!(result.label, "rayon");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn samply_prefix_maps_to_samply_bucket() {
    let result = match_owner_bucket("samply_something");
    assert_eq!(result.label, "samply/profiler");
}

#[test]
fn profiler_prefix_maps_to_samply_bucket() {
    let result = match_owner_bucket("profiler_something");
    assert_eq!(result.label, "samply/profiler");
}

// ----------------------------
//  Edge cases
// ----------------------------

#[test]
fn empty_name_maps_to_unknown() {
    let result = match_owner_bucket("");
    assert_eq!(result.label, "unknown");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn unknown_literal_maps_to_unknown() {
    let result = match_owner_bucket("unknown");
    assert_eq!(result.label, "unknown");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn unmatched_name_maps_to_other() {
    let result = match_owner_bucket("some_random_library::function");
    assert_eq!(result.label, "other");
    assert!(result.suggested_paths.is_empty());
}

#[test]
fn whitespace_only_name_maps_to_unknown() {
    let result = match_owner_bucket("   ");
    assert_eq!(result.label, "unknown");
}
