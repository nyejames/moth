//! Unit tests for generic function diagnostic helpers.
//!
//! WHAT: asserts that `with_generic_instantiation_context` rebuilds diagnostics correctly.
//! WHY: the helper is the single point of truth for call-site-primary generic instantiation
//! diagnostics and should be tested in isolation.

use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::ast::generic_functions::diagnostics::{
    GenericInstantiationDiagnosticContext, conflicting_generic_function_argument,
    with_generic_instantiation_context,
};
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticLabelStyle, DiagnosticPayload, GenericSubstitutionDiagnostic, RuleDiagnosticKind,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::GenericParameterId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn make_span(source: usize, start: u32, length: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::from_index(source),
        LocalSpan::exact(start, length, &mut span_builder).unwrap(),
    )
}

fn frozen_identity_for_test(
    directory: &Path,
    text: &str,
) -> (Arc<FrozenIdentityContext>, SourceId) {
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).expect("source root should be created");
    let source_path = source_root.join("main.moth");
    fs::write(&source_path, text).expect("source snapshot should be written");

    let mut string_table = StringTable::new();
    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path.as_path()),
        &source_root,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = source_database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    source_database
        .retain_text(source_id, text.to_owned())
        .expect("source snapshot should be retained");

    (
        Arc::new(FrozenIdentityContext::from_parts(
            string_table,
            source_database,
        )),
        source_id,
    )
}

fn instantiation_context(
    call_span: SourceSpan,
    declaration_span: SourceSpan,
) -> GenericInstantiationDiagnosticContext {
    GenericInstantiationDiagnosticContext {
        call_span: Some(call_span),
        declaration_span: Some(declaration_span),
        frozen_identity_handle: None,
        call_site_frozen_identity_handle: None,
        substitutions: Vec::new(),
    }
}

#[test]
fn with_generic_instantiation_context_changes_primary_span_to_call_site() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert_eq!(transformed.primary_span, Some(call_span));
    assert!(transformed.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(body_span)
            && label.message == Some(DiagnosticLabelMessage::GenericInstantiationBodySite)
    }));
}

#[test]
fn with_generic_instantiation_context_retains_exact_extended_call_span() {
    let body_span = make_span(1, 10, 1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let call_span = SourceSpan::new(
        SourceId::from_index(4),
        LocalSpan::exact(17, 4096, &mut span_builder).unwrap(),
    );
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert_eq!(span_builder.len(), 1);
    assert_eq!(transformed.primary_span, Some(call_span));
    assert!(
        transformed
            .labels
            .iter()
            .all(|label| label.span != Some(call_span))
    );
}

#[test]
fn with_generic_instantiation_context_preserves_existing_secondary_labels() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let extra_span = make_span(3, 30, 1);
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        Some(body_span),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(extra_span),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert!(transformed.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(extra_span)
            && label.message == Some(DiagnosticLabelMessage::PreviousDeclaration)
    }));
}

#[test]
fn with_generic_instantiation_context_avoids_duplicate_body_secondary_label() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        Some(body_span),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(body_span),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert_eq!(
        transformed
            .labels
            .iter()
            .filter(|label| label.span == Some(body_span))
            .count(),
        1
    );
}

#[test]
fn with_generic_instantiation_context_adds_declaration_span_label() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_span, declaration_span),
    );

    assert!(transformed.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(declaration_span)
            && label.message == Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite)
    }));
}

#[test]
fn with_generic_instantiation_context_adds_structured_substitution_label() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let parameter_name = string_table.intern("T");
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: Some(declaration_span),
            frozen_identity_handle: None,
            call_site_frozen_identity_handle: None,
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name,
                concrete_type_id: builtin_type_ids::STRING,
            }],
        },
    );

    assert!(transformed.labels.iter().any(|label| {
        label.span == Some(declaration_span)
            && matches!(
                &label.message,
                Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions {
                    substitutions
                }) if substitutions
                    == &vec![GenericSubstitutionDiagnostic {
                        parameter_name,
                        concrete_type_id: builtin_type_ids::STRING,
                    }]
            )
    }));
}

#[test]
fn conflicting_generic_function_argument_keeps_current_evidence_primary_span() {
    let mut string_table = StringTable::new();
    let current_span = make_span(5, 12, 1);
    let previous_span = make_span(5, 1, 1);
    let function_name = string_table.intern("same");
    let parameter_name = string_table.intern("T");

    let diagnostic = conflicting_generic_function_argument(
        Some(function_name),
        BindingConflict {
            parameter_id: GenericParameterId(0),
            existing_type_id: builtin_type_ids::INT,
            replacement_type_id: builtin_type_ids::STRING,
        },
        parameter_name,
        Some(current_span),
        Some(previous_span),
    );

    assert_eq!(diagnostic.primary_span, Some(current_span));
    assert!(diagnostic.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(previous_span)
            && label.message == Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence)
    }));
}

#[test]
fn conflicting_generic_function_argument_retains_evidence_spans() {
    let mut string_table = StringTable::new();
    let current_span = make_span(5, 2, 4096);
    let previous_span = make_span(5, 1, 4);
    let function_name = string_table.intern("same");
    let parameter_name = string_table.intern("T");

    let diagnostic = conflicting_generic_function_argument(
        Some(function_name),
        BindingConflict {
            parameter_id: GenericParameterId(0),
            existing_type_id: builtin_type_ids::INT,
            replacement_type_id: builtin_type_ids::STRING,
        },
        parameter_name,
        Some(current_span),
        Some(previous_span),
    );

    assert_eq!(diagnostic.primary_span, Some(current_span));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Some(previous_span));
}

#[test]
fn with_generic_instantiation_context_attaches_donor_owner_to_all_donor_labels() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let extra_span = make_span(1, 40, 1);
    let parameter_name = string_table.intern("T");
    let frozen_identity_handle = FrozenIdentityHandle::new();
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        Some(body_span),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(extra_span),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: Some(declaration_span),
            frozen_identity_handle: Some(frozen_identity_handle.clone()),
            call_site_frozen_identity_handle: None,
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name,
                concrete_type_id: builtin_type_ids::STRING,
            }],
        },
    );

    assert!(
        transformed
            .labels
            .iter()
            .all(|label| label.frozen_identity_handle.as_ref() == Some(&frozen_identity_handle)),
        "every donor label must retain the materialised body's owner"
    );
}

#[test]
fn same_domain_generic_labels_remain_ownerless() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert!(
        transformed
            .labels
            .iter()
            .all(|label| label.frozen_identity_handle.is_none()),
        "ordinary same-domain labels must continue using the requester context"
    );
}

#[test]
fn donor_owned_label_uses_installed_donor_context() {
    let requester_directory = tempfile::tempdir().expect("requester directory should exist");
    let donor_directory = tempfile::tempdir().expect("donor directory should exist");
    let (requester_identity, requester_source) =
        frozen_identity_for_test(requester_directory.path(), "requester body\n");
    let (donor_identity, donor_source) =
        frozen_identity_for_test(donor_directory.path(), "donor body\n");
    assert_eq!(
        requester_source, donor_source,
        "independently built domains should exercise colliding SourceIds"
    );

    let donor_handle = FrozenIdentityHandle::new();
    donor_handle
        .install(Arc::clone(&donor_identity))
        .expect("donor identity should install once");
    let donor_label = DiagnosticLabel::secondary_with_frozen_identity(
        Some(SourceSpan::new(
            donor_source,
            LocalSpan::exact(0, 5, &mut ExtendedSpanBuilder::new())
                .expect("donor span should fit inline"),
        )),
        Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
        donor_handle,
    );
    let requester_context = DiagnosticRenderContext::new(requester_identity.strings())
        .with_frozen_identity(&requester_identity);
    let position = requester_context
        .label_position(&donor_label)
        .expect("donor-owned label should resolve through its installed owner");
    assert_eq!(position.line, "donor body");

    let uninstalled_label = DiagnosticLabel::secondary_with_frozen_identity(
        donor_label.span,
        Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
        FrozenIdentityHandle::new(),
    );
    assert!(
        requester_context
            .label_position(&uninstalled_label)
            .is_none(),
        "an uninstalled donor owner must not fall back to requester context"
    );
}

#[test]
fn generic_reanchoring_preserves_mixed_domain_label_owners() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(1, 30, 1);
    let existing_owner = FrozenIdentityHandle::for_domain(StablePackageIdentity::source_package(
        PackageOrigin::ProjectLocal,
        "existing",
    ));
    let donor_owner = FrozenIdentityHandle::for_domain(StablePackageIdentity::source_package(
        PackageOrigin::ProjectLocal,
        "donor",
    ));
    let call_site_owner =
        FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("requester"));
    let mut diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    )
    .with_labels(vec![DiagnosticLabel::secondary_with_frozen_identity(
        Some(body_span),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
        existing_owner.clone(),
    )]);
    diagnostic.primary_frozen_identity_handle = Some(existing_owner.clone());

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: Some(declaration_span),
            frozen_identity_handle: Some(donor_owner.clone()),
            call_site_frozen_identity_handle: Some(call_site_owner),
            substitutions: Vec::new(),
        },
    );

    assert_eq!(
        transformed
            .primary_frozen_identity_handle
            .as_ref()
            .map(FrozenIdentityHandle::domain),
        Some(Some(&StablePackageIdentity::project_local("requester"))),
    );
    assert!(transformed.labels.iter().any(|label| {
        label.span == Some(body_span)
            && label.frozen_identity_handle.as_ref() == Some(&existing_owner)
    }));
    assert!(transformed.labels.iter().any(|label| {
        label.span == Some(declaration_span)
            && label.frozen_identity_handle.as_ref() == Some(&donor_owner)
    }));
}
