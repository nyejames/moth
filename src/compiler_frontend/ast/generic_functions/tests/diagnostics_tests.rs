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
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    GenericInstantiationShape, InvalidGenericInstantiationView, duplicate_trait_requirement,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::type_mismatch;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    CompactDiagnosticBagForTest, compact_bag_for_test,
};
use crate::compiler_frontend::compiler_messages::render::terminal::format_label_messages_view;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, compact_label_message_text,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, GenericInferenceSubject, GenericSubstitutionDiagnostic, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::GenericParameterId;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
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
        &source_path,
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
fn mismatch_diagnostic(span: Option<SourceSpan>) -> DiagnosticDraft {
    type_mismatch(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        span,
    )
}

fn previous_declaration_diagnostic(
    primary_span: SourceSpan,
    previous_span: SourceSpan,
) -> DiagnosticDraft {
    let mut string_table = StringTable::new();
    duplicate_trait_requirement(
        string_table.intern("TestTrait"),
        string_table.intern("requirement"),
        Some(previous_span),
        Some(primary_span),
    )
}

fn compact_diagnostic(
    diagnostic: DiagnosticDraft,
    environment: Option<&TypeEnvironment>,
) -> CompactDiagnosticBagForTest {
    compact_bag_for_test(DiagnosticBag::from(diagnostic), environment)
}

fn site_span(site: SiteRef<'_>) -> Option<SourceSpan> {
    match site {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}

fn site_has_owner(site: SiteRef<'_>, owner: &FrozenIdentityHandle) -> bool {
    let SiteRef::Foreign { context, .. } = site else {
        return false;
    };
    match (context, owner.get()) {
        (Some(context), Some(owner_context)) => std::ptr::eq(context, owner_context),
        (None, None) => true,
        _ => false,
    }
}

#[test]
fn with_generic_instantiation_context_changes_primary_span_to_call_site() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = mismatch_diagnostic(Some(body_span));

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert_eq!(site_span(diagnostic.primary_site()), Some(call_span));
    assert!(diagnostic.durable_labels().any(|label| {
        label.spanned
            && site_span(label.site) == Some(body_span)
            && matches!(
                label.message,
                DurableLabelMessage::GenericInstantiationBodySite
            )
    }));
}

#[test]
fn with_generic_instantiation_context_without_call_span_retains_body_primary() {
    let body_span = make_span(1, 10, 1);
    let declaration_span = make_span(2, 20, 1);
    let body_owner = FrozenIdentityHandle::new();
    let diagnostic = mismatch_diagnostic(Some(body_span))
        .with_primary_frozen_identity_handle(body_owner.clone());

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: None,
            declaration_span: Some(declaration_span),
            frozen_identity_handle: None,
            call_site_frozen_identity_handle: None,
            substitutions: Vec::new(),
        },
    );
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");
    assert_eq!(site_span(diagnostic.primary_site()), Some(body_span));
    assert!(site_has_owner(diagnostic.primary_site(), &body_owner));
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(
        labels
            .iter()
            .filter(|label| matches!(
                label.message,
                DurableLabelMessage::GenericInstantiationBodySite
            ))
            .count(),
        0,
        "without a call site, the body must remain primary rather than become a label"
    );
    assert!(labels.iter().any(|label| {
        site_span(label.site) == Some(declaration_span)
            && matches!(
                label.message,
                DurableLabelMessage::GenericInstantiationDeclarationSite
            )
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
    let diagnostic = mismatch_diagnostic(Some(body_span));

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert_eq!(span_builder.len(), 1);
    assert_eq!(site_span(diagnostic.primary_site()), Some(call_span));
    assert!(
        diagnostic
            .durable_labels()
            .all(|label| site_span(label.site) != Some(call_span))
    );
}

#[test]
fn with_generic_instantiation_context_preserves_existing_secondary_labels() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let extra_span = make_span(3, 30, 1);
    let diagnostic = previous_declaration_diagnostic(body_span, extra_span);
    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert!(diagnostic.durable_labels().any(|label| {
        label.spanned
            && site_span(label.site) == Some(extra_span)
            && matches!(label.message, DurableLabelMessage::PreviousDeclaration)
    }));
}

#[test]
fn with_generic_instantiation_context_avoids_duplicate_body_secondary_label() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = previous_declaration_diagnostic(body_span, body_span);

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert_eq!(
        diagnostic
            .durable_labels()
            .filter(|label| site_span(label.site) == Some(body_span))
            .count(),
        1
    );
}

#[test]
fn generic_reanchoring_does_not_deduplicate_distinct_domainless_label_owners() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let body_owner = FrozenIdentityHandle::new();
    let existing_owner = FrozenIdentityHandle::new();
    let diagnostic = previous_declaration_diagnostic(body_span, body_span)
        .with_label_frozen_identity_handle(0, existing_owner.clone())
        .with_primary_frozen_identity_handle(body_owner.clone());

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: None,
            frozen_identity_handle: None,
            call_site_frozen_identity_handle: None,
            substitutions: Vec::new(),
        },
    );
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();

    assert_eq!(
        labels
            .iter()
            .filter(|label| site_span(label.site) == Some(body_span))
            .count(),
        2,
        "distinct domain-less owners must not deduplicate colliding spans"
    );
    assert!(labels.iter().any(|label| {
        site_span(label.site) == Some(body_span)
            && matches!(
                label.message,
                DurableLabelMessage::GenericInstantiationBodySite
            )
            && site_has_owner(label.site, &body_owner)
    }));
    assert!(labels.iter().any(|label| {
        site_span(label.site) == Some(body_span)
            && matches!(label.message, DurableLabelMessage::PreviousDeclaration)
            && site_has_owner(label.site, &existing_owner)
    }));
}

#[test]
fn generic_reanchoring_deduplicates_same_domainless_label_owner() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let body_owner = FrozenIdentityHandle::new();
    let diagnostic = previous_declaration_diagnostic(body_span, body_span)
        .with_label_frozen_identity_handle(0, body_owner.clone())
        .with_primary_frozen_identity_handle(body_owner.clone());

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: None,
            frozen_identity_handle: None,
            call_site_frozen_identity_handle: None,
            substitutions: Vec::new(),
        },
    );
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert_eq!(
        diagnostic
            .durable_labels()
            .filter(|label| site_span(label.site) == Some(body_span))
            .count(),
        1,
        "clones of the same domain-less owner should deduplicate"
    );
}

#[test]
fn with_generic_instantiation_context_adds_declaration_span_label() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let diagnostic = mismatch_diagnostic(Some(body_span));

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_span, declaration_span),
    );
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert!(diagnostic.durable_labels().any(|label| {
        label.spanned
            && site_span(label.site) == Some(declaration_span)
            && matches!(
                label.message,
                DurableLabelMessage::GenericInstantiationDeclarationSite
            )
    }));
}

#[test]
fn with_generic_instantiation_context_adds_structured_substitution_label() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let parameter_name = string_table.intern("T");
    let diagnostic = mismatch_diagnostic(Some(body_span));

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
    let type_environment = TypeEnvironment::new();
    let compact = compact_diagnostic(transformed, Some(&type_environment));
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");
    let label = diagnostic
        .durable_labels()
        .find(|label| {
            site_span(label.site) == Some(declaration_span)
                && matches!(
                    label.message,
                    DurableLabelMessage::GenericInstantiationSubstitutions(_)
                )
        })
        .expect("generic substitutions should retain their typed label");
    let DurableLabelMessage::GenericInstantiationSubstitutions(substitutions) = label.message
    else {
        unreachable!("the label filter selected generic substitutions")
    };
    let substitutions = substitutions.iter().collect::<Vec<_>>();
    assert_eq!(substitutions.len(), 1);
    assert_eq!(substitutions[0].parameter, parameter_name);
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let render_context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);
    assert_eq!(
        compact_label_message_text(label.message, diagnostic.types(), render_context,).as_deref(),
        Some("generic substitution: T = String")
    );
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
            existing_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            replacement_type_id: builtin_type_ids::STRING,
        },
        parameter_name,
        Some(current_span),
        Some(previous_span),
    );

    let type_environment = TypeEnvironment::new();
    let compact = compact_diagnostic(diagnostic, Some(&type_environment));
    let diagnostic = compact
        .first()
        .expect("generic inference conflict emits a diagnostic");
    assert_eq!(site_span(diagnostic.primary_site()), Some(current_span));
    let facts = diagnostic
        .family_view::<InvalidGenericInstantiationView>()
        .expect("generic inference conflict should retain typed facts");
    let GenericInstantiationShape::ConflictingInference { subject, .. } = facts.shape else {
        panic!("expected conflicting generic inference facts");
    };
    assert_eq!(subject, GenericInferenceSubject::Function);
    assert!(diagnostic.durable_labels().any(|label| {
        label.spanned
            && site_span(label.site) == Some(previous_span)
            && matches!(
                label.message,
                DurableLabelMessage::GenericInferencePreviousEvidence
            )
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
            existing_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            replacement_type_id: builtin_type_ids::STRING,
        },
        parameter_name,
        Some(current_span),
        Some(previous_span),
    );

    let type_environment = TypeEnvironment::new();
    let compact = compact_diagnostic(diagnostic, Some(&type_environment));
    let diagnostic = compact
        .first()
        .expect("generic inference conflict emits a diagnostic");
    assert_eq!(site_span(diagnostic.primary_site()), Some(current_span));
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert_eq!(site_span(labels[0].site), Some(previous_span));
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
    let diagnostic = previous_declaration_diagnostic(body_span, extra_span);

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
    let type_environment = TypeEnvironment::new();
    let compact = compact_diagnostic(transformed, Some(&type_environment));
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");
    assert!(
        diagnostic
            .durable_labels()
            .all(|label| site_has_owner(label.site, &frozen_identity_handle)),
        "every donor label must retain the materialised body's owner"
    );
}

#[test]
fn same_domain_generic_labels_remain_ownerless() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = mismatch_diagnostic(Some(body_span));

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert!(
        diagnostic
            .durable_labels()
            .all(|label| matches!(label.site, SiteRef::Local(_) | SiteRef::Absent)),
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
    let donor_span = SourceSpan::new(
        donor_source,
        LocalSpan::exact(0, 5, &mut ExtendedSpanBuilder::new())
            .expect("donor span should fit inline"),
    );
    let donor_draft = mismatch_diagnostic(None)
        .with_label(
            Some(donor_span),
            Some(LabelDraftMessage::GenericInstantiationBodySite),
        )
        .with_label_frozen_identity_handle(0, donor_handle.clone());
    let donor_compact = compact_diagnostic(donor_draft, None);
    let requester_context = DiagnosticRenderContext::new(requester_identity.strings())
        .with_frozen_identity(&requester_identity);
    let donor_messages = format_label_messages_view(
        donor_compact.first().expect("donor label compacts"),
        requester_context,
    );
    assert!(
        donor_messages
            .iter()
            .any(|message| message.contains("generic body operation failed here")),
        "the donor-owned label should retain its rendered message"
    );
    let donor_view = donor_compact.first().expect("donor label compacts");
    let donor_label = donor_view
        .durable_labels()
        .next()
        .expect("donor diagnostic retains its label");
    assert!(site_has_owner(donor_label.site, &donor_handle));
    let donor_position = requester_context
        .site_position(donor_label.site)
        .expect("the installed donor owner should resolve its source span");
    assert_eq!(donor_position.line, "donor body");

    let uninstalled_draft = mismatch_diagnostic(None)
        .with_label(
            Some(donor_span),
            Some(LabelDraftMessage::GenericInstantiationBodySite),
        )
        .with_label_frozen_identity_handle(0, FrozenIdentityHandle::new());
    let uninstalled_compact = compact_diagnostic(uninstalled_draft, None);
    assert!(
        format_label_messages_view(
            uninstalled_compact
                .first()
                .expect("uninstalled donor label compacts"),
            requester_context,
        )
        .is_empty(),
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
    let diagnostic = previous_declaration_diagnostic(body_span, body_span)
        .with_label_frozen_identity_handle(0, existing_owner.clone())
        .with_primary_frozen_identity_handle(existing_owner.clone());

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: Some(declaration_span),
            frozen_identity_handle: Some(donor_owner.clone()),
            call_site_frozen_identity_handle: Some(call_site_owner.clone()),
            substitutions: Vec::new(),
        },
    );
    let compact = compact_diagnostic(transformed, None);
    let diagnostic = compact.first().expect("generic rewrite emits a diagnostic");

    assert_eq!(site_span(diagnostic.primary_site()), Some(call_span));
    assert!(site_has_owner(diagnostic.primary_site(), &call_site_owner));
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert!(labels.iter().any(|label| {
        site_span(label.site) == Some(body_span) && site_has_owner(label.site, &existing_owner)
    }));
    assert!(labels.iter().any(|label| {
        site_span(label.site) == Some(declaration_span) && site_has_owner(label.site, &donor_owner)
    }));
}
