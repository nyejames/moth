//! Unit tests for the module-local resource origin table.
//!
//! These protect the interning invariants: repeated names share one origin record, the first
//! authored span wins, distinct origins get distinct handles, and a handle the table never
//! issued is a fallible read rather than a silent index.

use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

fn origin(relative: &str) -> StableResourceOriginId {
    let module = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    );

    let logical_path = PortableResourcePath::from_relative_logical_path(Path::new(relative))
        .expect("relative resource path should be portable");

    StableResourceOriginId::module_owned(module, logical_path)
}

fn span(offset: u32) -> SourceSpan {
    let mut extended = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(offset, 1, &mut extended).expect("test span should fit"),
    )
}

#[test]
fn repeating_one_origin_interns_a_single_record() {
    let mut table = ModuleResourceTable::new();

    let first = table.intern_origin(origin("logo.svg"), Some(span(30)));
    let second = table.intern_origin(origin("logo.svg"), Some(span(90)));

    assert_eq!(first, second);
    assert_eq!(table.origins().len(), 1);
}

#[test]
fn the_first_authored_span_owns_the_origin_record() {
    let mut table = ModuleResourceTable::new();

    let first_authored = span(30);
    let resource = table.intern_origin(origin("logo.svg"), Some(first_authored));
    table.intern_origin(origin("logo.svg"), Some(span(90)));

    let record = table
        .try_origin(resource)
        .expect("origin should be in range");

    assert_eq!(record.first_authored_span, Some(first_authored));
}

#[test]
fn resource_origin_spans_are_stable_through_string_remapping() {
    let mut table = ModuleResourceTable::new();
    let first_authored = span(30);
    let resource = table.intern_origin(origin("logo.svg"), Some(first_authored));

    let mut local_strings = StringTable::new();
    local_strings.intern("unused");
    let mut merged_strings = StringTable::new();
    merged_strings.intern("prefix");
    let remap = merged_strings.merge_from(&local_strings);
    assert!(!remap.is_identity());

    table.remap_string_ids(&remap);

    assert_eq!(
        table
            .try_origin(resource)
            .expect("resource should remain readable after remapping")
            .first_authored_span,
        Some(first_authored)
    );
}

#[test]
fn distinct_origins_get_distinct_dense_handles() {
    let mut table = ModuleResourceTable::new();

    let logo = table.intern_origin(origin("logo.svg"), Some(span(30)));
    let mark = table.intern_origin(origin("mark.svg"), Some(span(40)));

    assert_ne!(logo, mark);
    assert_eq!(table.origins().len(), 2);
}

/// Bounds checking catches a handle past the end of the table. It cannot detect a foreign handle
/// that happens to be in range, so pairing a `ResourceId` with its issuing table stays an
/// architectural invariant of how the handle is passed around, not a runtime check.
#[test]
fn an_out_of_range_resource_handle_is_rejected() {
    let mut donor = ModuleResourceTable::new();
    let past_the_end = donor.intern_origin(origin("logo.svg"), Some(span(30)));

    let empty = ModuleResourceTable::new();

    assert!(empty.try_origin(past_the_end).is_err());
}

#[test]
fn a_module_that_names_no_resource_keeps_an_empty_table() {
    let table = ModuleResourceTable::new();

    assert!(table.origins().is_empty());
}
