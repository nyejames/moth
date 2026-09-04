//! Classification tests for graph-active file references.

use super::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget, classify_prepared_file_references,
};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::paths::dependency_resolution::exact_case_mismatch_for_components;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn classify_one(spelling: &str) -> PreparedFileReferenceClass {
    let mut strings = StringTable::new();
    let mut table = PathSyntaxTable::new();
    let location = SourceLocation::default();
    table.push(
        if spelling.is_empty() {
            InternedPath::new()
        } else {
            InternedPath::from_single_str(spelling, &mut strings)
        },
        location,
    );
    let classified = classify_prepared_file_references(&table, [], None, &strings);
    classified.references()[0].class
}

#[test]
fn empty_path_is_a_site_root() {
    assert_eq!(classify_one(""), PreparedFileReferenceClass::SiteRoot);
}

#[test]
fn mtf_and_md_are_content_sources() {
    assert_eq!(
        classify_one("docs/intro.mtf"),
        PreparedFileReferenceClass::ContentSource
    );
    assert_eq!(
        classify_one("legal/license.md"),
        PreparedFileReferenceClass::ContentSource
    );
}

#[test]
fn moth_is_retained_for_the_no_file_value_diagnostic() {
    assert_eq!(
        classify_one("helpers.moth"),
        PreparedFileReferenceClass::SourceKindNoFileValue
    );
}

#[test]
fn ordinary_extensions_are_resource_files() {
    assert_eq!(
        classify_one("assets/logo.svg"),
        PreparedFileReferenceClass::ResourceFile
    );
}

#[test]
fn extensionless_paths_are_left_for_ast() {
    assert_eq!(
        classify_one("docs/intro"),
        PreparedFileReferenceClass::Extensionless
    );
}

#[test]
fn dependency_clause_rows_are_not_reclassified_as_file_values() {
    let mut strings = StringTable::new();
    let mut table = PathSyntaxTable::new();
    let location = SourceLocation::default();
    let clause = table.push(
        InternedPath::from_single_str("core/math", &mut strings),
        location.clone(),
    );
    let value = table.push(
        InternedPath::from_single_str("assets/logo.svg", &mut strings),
        location,
    );

    let classified = classify_prepared_file_references(&table, [clause], None, &strings);
    let references = classified.references();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].path_syntax, value);
    assert_eq!(
        references[0].class,
        PreparedFileReferenceClass::ResourceFile
    );
}

#[test]
fn quoted_url_strings_are_not_path_rows() {
    let strings = StringTable::new();
    let table = PathSyntaxTable::new();
    let classified = classify_prepared_file_references(&table, [], None, &strings);
    assert!(classified.references().is_empty());
}

#[test]
fn resolved_references_are_lookupable_by_file_and_path_handle() {
    let mut strings = StringTable::new();
    let mut syntax = PathSyntaxTable::new();
    let path_syntax = syntax.push(
        InternedPath::from_single_str("assets/logo.svg", &mut strings),
        SourceLocation::default(),
    );
    let mut table = ResolvedFileReferenceTable::new();
    let reference = ResolvedFileReference {
        source_file: SourceId::from_index(7),
        path_syntax,
        class: PreparedFileReferenceClass::SourceKindNoFileValue,
        outcome: ResolvedFileReferenceOutcome::Target(
            ResolvedFileReferenceTarget::IdentifiedSourceKind,
        ),
    };

    table
        .push(reference)
        .expect("first composite key should be unique");

    assert_eq!(table.iter().count(), 1);
    assert!(table.get(SourceId::from_index(7), path_syntax).is_some());
    assert!(table.get(SourceId::from_index(8), path_syntax).is_none());
}

#[test]
fn resolved_reference_duplicate_composite_keys_are_rejected() {
    let mut strings = StringTable::new();
    let mut syntax = PathSyntaxTable::new();
    let path_syntax = syntax.push(
        InternedPath::from_single_str("assets/logo.svg", &mut strings),
        SourceLocation::default(),
    );
    let mut table = ResolvedFileReferenceTable::new();
    let make_reference = || ResolvedFileReference {
        source_file: SourceId::from_index(7),
        path_syntax,
        class: PreparedFileReferenceClass::SourceKindNoFileValue,
        outcome: ResolvedFileReferenceOutcome::Target(
            ResolvedFileReferenceTarget::IdentifiedSourceKind,
        ),
    };

    table
        .push(make_reference())
        .expect("first composite key should be unique");
    assert!(table.push(make_reference()).is_err());
}

#[test]
fn resolved_reference_validation_rejects_class_outcome_mismatch() {
    let mut strings = StringTable::new();
    let mut syntax = PathSyntaxTable::new();
    let path_syntax = syntax.push(
        InternedPath::from_single_str("assets/logo.svg", &mut strings),
        SourceLocation::default(),
    );
    let mut table = ResolvedFileReferenceTable::new();
    table
        .push(ResolvedFileReference {
            source_file: SourceId::from_index(7),
            path_syntax,
            class: PreparedFileReferenceClass::ResourceFile,
            outcome: ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::IdentifiedSourceKind,
            ),
        })
        .expect("first composite key should be unique");

    assert!(table.validate().is_err());
}

#[test]
fn exact_case_policy_reports_component_mismatches_for_full_filenames() {
    let authored = vec!["Assets".to_owned(), "logo.svg".to_owned()];
    let mismatch = exact_case_mismatch_for_components(
        &authored,
        std::path::Path::new("/project"),
        std::path::Path::new("/project/assets/logo.svg"),
        false,
    );

    assert_eq!(mismatch, Some(("Assets".to_owned(), "assets".to_owned())));
}
