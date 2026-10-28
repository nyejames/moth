//! Identity-domain remapping of compact diagnostic drafts.
//!
//! WHAT: a draft built against one string table and path interner is remapped into a merged domain, then compared with
//! the same diagnostic built directly in that domain. The stored words, the decoded label text, and the rendered message
//! must all match.
//! WHY: string and path IDs are bare `u32` words in fact, label, substitution, and extra storage. A missed or misrouted
//! rewrite names a different string or path without any error, so each test pins the remapped words against a
//! destination build that never went through the remap.
//!
//! The destination is seeded with unrelated names and paths before the source is merged in, so a skipped rewrite keeps a
//! source ID that names something else and the comparison fails.

use super::{TEST_LABEL_FAMILY, exact_span, identity};
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::{
    ExtraDataDraft, ExtraDataKind,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    self, InvalidCallShapeView, InvalidReceiverCallView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    self, CompileTimeEvaluationErrorView, non_exhaustive_match,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    self, InvalidReceiverDeclarationView, InvalidTraitConformanceShape,
    InvalidTraitConformanceView, invalid_generic_instantiation, invalid_trait_conformance,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    self, NotExportedByPublicSurfaceView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    self, DuplicateDeclarationView, duplicate_declaration,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    self, LegacyDependencyClauseView, circular_dependency,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    self, CompilerCapacityExceededView, ExpectedTokenView, invalid_multi_bind_syntax,
    missing_closing_delimiter, unexpected_token_from_tag,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    self, SharedMutableConflictView, multiple_mutable_borrows, use_of_uninitialized_local,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::{
    ForeignSiteDraft, ForeignSlot, SiteRef,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DurableLabelMessage, GenericSubstitutionDiagnostic, LabelDraftMessage, LabelMessageCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::diagnostics::store::{
    DiagnosticId, DiagnosticStore,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    capture_rebased_batch, exact_span_in_source, finish_prepared_batch,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::{DiagnosticView, FamilyView};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::context::compact_label_message_text;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, CapacityResource, CompileTimeEvaluationErrorReason, DiagnosticBag,
    DiagnosticCapacityResource, DiagnosticCategory, DiagnosticDescriptor, DiagnosticPlace,
    DiagnosticSeverity, DiagnosticToken, GenericInferenceSubject, ImportPublicSurfaceType,
    InvalidCallShapeReason, InvalidGenericInstantiationReason, InvalidMultiBindReason,
    InvalidReceiverCallReason, InvalidReceiverDeclarationReason, InvalidTraitConformanceReason,
    NonExhaustiveMatchReason, ReceiverCallKind, TokenTag,
};
use crate::compiler_frontend::datatypes::display::TypeDisplayShape;
use crate::compiler_frontend::datatypes::display::TypeDisplayView;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, builtin_type_ids};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, FrozenIdentityContext, SourceId};
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerBuilder, PathInternerFork, PathTable,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::sync::Arc;

fn diagnosed_reports(result: &CompilationResult<()>) -> &DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        other => panic!("expected diagnosed remap result, got {other:?}"),
    }
}

/// A test-only family that declares the label roles these tests decode. It is never registered, so no render
/// path reaches it; only decoded label text is read from it.
static REMAP_LABEL_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9003),
    category: DiagnosticCategory::Rule,
    reserved: false,
    descriptor: DiagnosticDescriptor::new(
        "MOTH-TEST-9003",
        "Test family",
        DiagnosticSeverity::Error,
    ),
    layout: &[],
    extra: ExtraDataKind::None,
    labels: &[
        LabelMessageCode::CONFIG_INPUT_ORIGIN,
        LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE,
    ],
    render: None,
    reason_key: |_| None,
};

/// One draft compacted into its own store, with its report-owned contexts and type snapshot.
struct Stored {
    store: DiagnosticStore,
    foreign_contexts: Vec<Option<Arc<FrozenIdentityContext>>>,
    id: DiagnosticId,
    types: TypeDisplaySnapshot,
}

impl Stored {
    fn new(mut draft: DiagnosticDraft, type_environment: &TypeEnvironment) -> Self {
        let mut builder = TypeDisplaySnapshotBuilder::new(Some(type_environment));
        draft
            .capture_types(&mut |type_id| builder.display_top_level(type_id))
            .expect("small test types always capture");
        let types = builder.finish();
        let mut store = DiagnosticStore::new();
        let mut foreign_contexts = Vec::new();
        let CompactOutcome::Stored(id) = store.compact(draft, &mut foreign_contexts) else {
            panic!("a small test draft stores");
        };
        Self {
            store,
            foreign_contexts,
            id,
            types,
        }
    }

    /// The registered family's message, rendered by the production render function.
    fn rendered(&self, context: DiagnosticRenderContext<'_>) -> String {
        let view = DiagnosticView::new(&self.store, self.id, &self.types, &self.foreign_contexts);
        let render = view
            .family()
            .render
            .expect("every production family renders");
        render(&view, context).message
    }

    /// Each stored label's decoded text, in stored order.
    fn label_texts(&self, context: DiagnosticRenderContext<'_>) -> Vec<String> {
        self.store
            .record(self.id, &self.foreign_contexts)
            .durable_labels()
            .map(|label| {
                compact_label_message_text(label.message, &self.types, context)
                    .expect("every test label decodes to text")
            })
            .collect()
    }
}

/// A render context over the destination tables. The path table must be frozen before the context is built.
fn render_context<'a>(
    strings: &'a StringTable,
    paths: &'a PathTable,
) -> DiagnosticRenderContext<'a> {
    DiagnosticRenderContext::new(strings).with_path_table(paths)
}

/// The string IDs a draft's extra record carries, or none when the draft has no string extra.
fn extra_string_ids(draft: &DiagnosticDraft) -> Vec<StringId> {
    match draft.extra.as_deref().map(|extra| &extra.data) {
        Some(ExtraDataDraft::StringIds(ids)) => ids.clone(),
        _ => Vec::new(),
    }
}

/// Asserts that two drafts hold the same family, span, fact words, extra string IDs, labels, and substitutions.
fn assert_same_draft(remapped: &DiagnosticDraft, direct: &DiagnosticDraft) {
    assert!(
        std::ptr::eq(remapped.core.family, direct.core.family),
        "both drafts use the same family row"
    );
    assert_eq!(
        remapped.core.span, direct.core.span,
        "the primary span is unchanged"
    );
    assert_eq!(
        remapped.core.facts, direct.core.facts,
        "the fact words match the destination build"
    );
    assert_eq!(remapped.core.pending_facts, direct.core.pending_facts);
    assert_eq!(remapped.core.faulted, direct.core.faulted);
    assert_eq!(
        extra_string_ids(remapped),
        extra_string_ids(direct),
        "the extra string IDs match the destination build"
    );
    match (remapped.extra.as_deref(), direct.extra.as_deref()) {
        (None, None) => {}
        (Some(left), Some(right)) => {
            assert_eq!(
                left.labels.len(),
                right.labels.len(),
                "the label count matches"
            );
            for (left_label, right_label) in left.labels.iter().zip(&right.labels) {
                assert_eq!(left_label.span, right_label.span, "the label span matches");
                assert_eq!(
                    left_label.message, right_label.message,
                    "the label role matches"
                );
                assert_eq!(left_label.pending_types, right_label.pending_types);
                assert_eq!(
                    left_label.data, right_label.data,
                    "the label data words match the destination build"
                );
            }
            assert_eq!(
                left.substitutions.len(),
                right.substitutions.len(),
                "the substitution count matches"
            );
            for (left_sub, right_sub) in left.substitutions.iter().zip(&right.substitutions) {
                assert_eq!(
                    left_sub.parameter, right_sub.parameter,
                    "the substitution parameter matches"
                );
                assert_eq!(
                    left_sub.concrete, right_sub.concrete,
                    "the substitution type matches"
                );
            }
            assert_eq!(
                left.foreign_sites.len(),
                right.foreign_sites.len(),
                "the foreign-site count matches"
            );
        }
        _ => panic!("the remapped and direct drafts disagree on whether an extra record exists"),
    }
}

/// Checks the remapped draft against the direct one field by field, then checks that both compact and render to the
/// same message.
fn assert_renders_like_direct(
    remapped: DiagnosticDraft,
    direct: DiagnosticDraft,
    type_environment: &TypeEnvironment,
    context: DiagnosticRenderContext<'_>,
) {
    assert_same_draft(&remapped, &direct);
    let remapped = Stored::new(remapped, type_environment);
    let direct = Stored::new(direct, type_environment);
    assert_eq!(
        remapped.rendered(context),
        direct.rendered(context),
        "the remapped draft renders like the direct one"
    );
}

/// A destination domain: fresh tables seeded with unrelated entries, then the source merged in. The seeds shift every
/// merged ID, so a remap that skips a word leaves a source ID that names something else.
struct MergedDomain {
    strings: StringTable,
    paths: PathInternerBuilder,
    string_remap: StringIdRemap,
    path_remap: PathIdRemap,
}

fn merge_into_seeded(
    source_strings: &StringTable,
    source_paths: &PathInternerFork,
) -> MergedDomain {
    let mut strings = StringTable::new();
    strings.intern("unrelated_first");
    strings.intern("unrelated_second");
    let mut paths = PathInternerBuilder::new();
    paths
        .try_intern_portable_path("unrelated/first", &mut strings)
        .expect("the seed path is portable");
    let string_remap = strings.merge_from(source_strings);
    let path_remap = paths
        .merge_delta_from(source_paths, &string_remap)
        .expect("a fork built from the empty table merges into any builder");
    MergedDomain {
        strings,
        paths,
        string_remap,
        path_remap,
    }
}

/// Three labels: two config origins naming strings, and a label with no payload.
fn label_draft(input_name: StringId, second_input_name: StringId) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&REMAP_LABEL_FAMILY, None, |_| Ok(()))
        .with_label(
            Some(exact_span(0, 5)),
            Some(LabelDraftMessage::ConfigInputOrigin {
                input_name,
                origin: BuildConfigValueOrigin::BuilderGlobal,
            }),
        )
        .with_label(
            Some(exact_span(6, 3)),
            Some(LabelDraftMessage::ConfigInputOrigin {
                input_name: second_input_name,
                origin: BuildConfigValueOrigin::BuilderGlobal,
            }),
        )
        .with_label(
            Some(exact_span(10, 4)),
            Some(LabelDraftMessage::GenericInferencePreviousEvidence),
        )
}

/// One generic-substitution label, whose parameter name lives in the extra record's substitution list.
fn substitution_draft(parameter_name: StringId) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&TEST_LABEL_FAMILY, None, |_| Ok(())).with_label(
        Some(exact_span(0, 3)),
        Some(LabelDraftMessage::GenericInstantiationSubstitutions {
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name,
                concrete_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            }],
        }),
    )
}

fn conflicting_inference(
    parameter_name: StringId,
    type_environment: &TypeEnvironment,
) -> InvalidGenericInstantiationReason {
    InvalidGenericInstantiationReason::ConflictingInference {
        subject: GenericInferenceSubject::NominalType,
        parameter_id: GenericParameterId(0),
        parameter_name,
        existing_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        replacement_type_id: type_environment.builtins().string,
    }
}

#[test]
fn identity_remap_leaves_every_word_unchanged() {
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let mut source_paths = PathInternerFork::empty();
    let source_path = source_paths
        .try_intern_portable_path("src/a.bst", &mut source_strings)
        .expect("the test path is portable");

    // A destination that merges the same rows in the same order numbers every entry the same, so both remaps are
    // identities and must leave every word alone.
    let mut strings = StringTable::new();
    let string_remap = strings.merge_from(&source_strings);
    let mut paths = PathInternerBuilder::new();
    let path_remap = paths
        .merge_delta_from(&source_paths, &string_remap)
        .expect("a fork built from the empty table merges into a fresh builder");
    assert!(
        string_remap.is_identity(),
        "the string remap is the identity"
    );
    assert!(path_remap.is_identity(), "the path remap is the identity");

    let mut duplicate = duplicate_declaration(alpha, Some(exact_span(0, 5)));
    duplicate.remap_string_ids(&string_remap);
    duplicate.remap_path_ids(&path_remap);
    assert_same_draft(
        &duplicate,
        &duplicate_declaration(alpha, Some(exact_span(0, 5))),
    );

    let mut circular = circular_dependency(source_path, Some(exact_span(0, 5)));
    circular.remap_string_ids(&string_remap);
    circular.remap_path_ids(&path_remap);
    assert_same_draft(
        &circular,
        &circular_dependency(source_path, Some(exact_span(0, 5))),
    );
}

#[test]
fn string_fact_words_map_to_destination_ids() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let beta = source_strings.intern("beta");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_alpha = domain.string_remap.get(alpha);
    let dest_beta = domain.string_remap.get(beta);
    assert_ne!(alpha, dest_alpha);
    assert_ne!(beta, dest_beta);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    let mut remapped = duplicate_declaration(alpha, Some(exact_span(0, 5)));
    remapped.remap_string_ids(&domain.string_remap);
    let direct = duplicate_declaration(dest_alpha, Some(exact_span(0, 5)));
    assert_renders_like_direct(remapped, direct, &type_environment, context);

    let mut remapped = missing_closing_delimiter(beta, Some(exact_span(4, 1)));
    remapped.remap_string_ids(&domain.string_remap);
    let direct = missing_closing_delimiter(dest_beta, Some(exact_span(4, 1)));
    assert_renders_like_direct(remapped, direct, &type_environment, context);
}

#[test]
fn optional_string_words_remap_only_when_present() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let beta = source_strings.intern("beta");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_alpha = domain.string_remap.get(alpha);
    let dest_beta = domain.string_remap.get(beta);
    assert_ne!(alpha, dest_alpha);
    assert_ne!(beta, dest_beta);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    // `WrongArgumentCount` stores its name in words 3..4, the other named reasons in words 1..2. An absent name keeps
    // the sentinel word.
    let cases = [
        (
            Some(alpha),
            Some(dest_alpha),
            InvalidGenericInstantiationReason::WrongArgumentCount {
                expected: 1,
                found: 2,
            },
        ),
        (
            Some(beta),
            Some(dest_beta),
            InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments,
        ),
        (
            None,
            None,
            InvalidGenericInstantiationReason::MissingTypeArguments,
        ),
    ];
    for (source_name, dest_name, reason) in cases {
        let mut remapped =
            invalid_generic_instantiation(source_name, reason.clone(), Some(exact_span(2, 3)));
        remapped.remap_string_ids(&domain.string_remap);
        let direct = invalid_generic_instantiation(dest_name, reason, Some(exact_span(2, 3)));
        assert_renders_like_direct(remapped, direct, &type_environment, context);
    }
}

#[test]
fn extra_string_ids_remap_to_destination_ids() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let beta = source_strings.intern("beta");
    let widget = source_strings.intern("Widget");
    let printable = source_strings.intern("Printable");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_alpha = domain.string_remap.get(alpha);
    let dest_beta = domain.string_remap.get(beta);
    let dest_widget = domain.string_remap.get(widget);
    let dest_printable = domain.string_remap.get(printable);
    assert_ne!(alpha, dest_alpha);
    assert_ne!(beta, dest_beta);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    // Missing variant names live in the extra record, not in the fact words.
    let mut remapped = non_exhaustive_match(
        NonExhaustiveMatchReason::MissingVariants,
        vec![alpha, beta],
        Some(exact_span(6, 4)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    let direct = non_exhaustive_match(
        NonExhaustiveMatchReason::MissingVariants,
        vec![dest_alpha, dest_beta],
        Some(exact_span(6, 4)),
    );
    assert_eq!(extra_string_ids(&remapped), [dest_alpha, dest_beta]);
    assert_renders_like_direct(remapped, direct, &type_environment, context);

    // Trait-conformance names are extra string IDs too. The reason's name is stored only when the reason has one.
    let mut remapped = invalid_trait_conformance(
        widget,
        Some(printable),
        InvalidTraitConformanceReason::ImportedModuleRoot,
        Some(exact_span(7, 3)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    let direct = invalid_trait_conformance(
        dest_widget,
        Some(dest_printable),
        InvalidTraitConformanceReason::ImportedModuleRoot,
        Some(exact_span(7, 3)),
    );
    assert_renders_like_direct(remapped, direct, &type_environment, context);
}

#[test]
fn path_fact_words_map_to_destination_path_ids() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let mut source_paths = PathInternerFork::empty();
    let source_path = source_paths
        .try_intern_portable_path("src/app/main.bst", &mut source_strings)
        .expect("the test path is portable");
    let mut domain = merge_into_seeded(&source_strings, &source_paths);
    let dest_path = domain.path_remap.get(source_path);
    assert_ne!(source_path, dest_path);

    // Interning the same spelling directly in the destination must land on the merged node.
    let direct_path = domain
        .paths
        .try_intern_portable_path("src/app/main.bst", &mut domain.strings)
        .expect("the test path is portable");
    assert_eq!(dest_path, direct_path);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    let mut remapped = circular_dependency(source_path, Some(exact_span(0, 5)));
    remapped.remap_path_ids(&domain.path_remap);
    let direct = circular_dependency(direct_path, Some(exact_span(0, 5)));
    assert_renders_like_direct(remapped, direct, &type_environment, context);
}

#[test]
fn place_words_remap_by_tag() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let mut source_paths = PathInternerFork::empty();
    let source_path = source_paths
        .try_intern_portable_path("src/app/main.bst", &mut source_strings)
        .expect("the test path is portable");
    let domain = merge_into_seeded(&source_strings, &source_paths);
    let dest_alpha = domain.string_remap.get(alpha);
    let dest_path = domain.path_remap.get(source_path);
    assert_ne!(alpha, dest_alpha);
    assert_ne!(source_path, dest_path);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    // Local stores a string ID, Path stores a path ID, and Unknown stores neither.
    let places = [
        (
            DiagnosticPlace::Local(alpha),
            DiagnosticPlace::Local(dest_alpha),
        ),
        (
            DiagnosticPlace::Path(source_path),
            DiagnosticPlace::Path(dest_path),
        ),
        (DiagnosticPlace::Unknown, DiagnosticPlace::Unknown),
    ];
    for (source_place, dest_place) in places {
        let mut remapped = use_of_uninitialized_local(source_place, Some(exact_span(3, 2)));
        remapped.remap_string_ids(&domain.string_remap);
        remapped.remap_path_ids(&domain.path_remap);
        let direct = use_of_uninitialized_local(dest_place, Some(exact_span(3, 2)));
        assert_renders_like_direct(remapped, direct, &type_environment, context);
    }

    // The conflicting place is optional: `Some` remaps both places, `None` keeps its sentinel.
    let mut remapped = multiple_mutable_borrows(
        DiagnosticPlace::Local(alpha),
        Some(DiagnosticPlace::Path(source_path)),
        Some(exact_span(1, 1)),
        Some(exact_span(4, 1)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    remapped.remap_path_ids(&domain.path_remap);
    let direct = multiple_mutable_borrows(
        DiagnosticPlace::Local(dest_alpha),
        Some(DiagnosticPlace::Path(dest_path)),
        Some(exact_span(1, 1)),
        Some(exact_span(4, 1)),
    );
    assert_renders_like_direct(remapped, direct, &type_environment, context);

    let mut remapped = multiple_mutable_borrows(
        DiagnosticPlace::Local(alpha),
        None,
        None,
        Some(exact_span(4, 1)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    remapped.remap_path_ids(&domain.path_remap);
    let direct = multiple_mutable_borrows(
        DiagnosticPlace::Local(dest_alpha),
        None,
        None,
        Some(exact_span(4, 1)),
    );
    assert_renders_like_direct(remapped, direct, &type_environment, context);
}

#[test]
fn path_place_word_ignores_the_string_remap() {
    let mut source_strings = StringTable::new();
    source_strings.intern("source_only");
    let mut source_paths = PathInternerFork::empty();
    let source_path = source_paths
        .try_intern_portable_path("src/app/main.bst", &mut source_strings)
        .expect("the test path is portable");
    let domain = merge_into_seeded(&source_strings, &source_paths);
    assert!(
        !domain.string_remap.is_identity(),
        "the string remap must move IDs, or a wrong rewrite could go unnoticed"
    );

    // A path place stores a path index, not a string ID, so the string remap must leave its word exactly as written.
    let mut remapped =
        use_of_uninitialized_local(DiagnosticPlace::Path(source_path), Some(exact_span(3, 2)));
    remapped.remap_string_ids(&domain.string_remap);
    let unchanged =
        use_of_uninitialized_local(DiagnosticPlace::Path(source_path), Some(exact_span(3, 2)));
    assert_same_draft(&remapped, &unchanged);
}

#[test]
fn token_words_remap_their_string_payloads() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_alpha = domain.string_remap.get(alpha);
    assert_ne!(alpha, dest_alpha);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    // A symbol and a style directive carry a string payload, so their IDs remap. A static token carries none.
    let cases = [
        (
            DiagnosticToken::from_string_tag(TokenTag::SYMBOL, alpha),
            DiagnosticToken::from_string_tag(TokenTag::SYMBOL, dest_alpha),
        ),
        (
            DiagnosticToken::from_string_tag(TokenTag::STYLE_DIRECTIVE, alpha),
            DiagnosticToken::from_string_tag(TokenTag::STYLE_DIRECTIVE, dest_alpha),
        ),
        (
            DiagnosticToken::from_static_tag(TokenTag::COMMA),
            DiagnosticToken::from_static_tag(TokenTag::COMMA),
        ),
    ];
    for (source_token, dest_token) in cases {
        let mut remapped = unexpected_token_from_tag(source_token, Some(exact_span(1, 5)));
        remapped.remap_string_ids(&domain.string_remap);
        let direct = unexpected_token_from_tag(dest_token, Some(exact_span(1, 5)));
        assert_renders_like_direct(remapped, direct, &type_environment, context);
    }

    // The optional target name is a string word; `None` keeps the sentinel.
    let mut remapped = invalid_multi_bind_syntax(
        InvalidMultiBindReason::ThisTargetReserved,
        Some(alpha),
        Some(exact_span(4, 2)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    let direct = invalid_multi_bind_syntax(
        InvalidMultiBindReason::ThisTargetReserved,
        Some(dest_alpha),
        Some(exact_span(4, 2)),
    );
    assert_renders_like_direct(remapped, direct, &type_environment, context);

    let mut remapped = invalid_multi_bind_syntax(
        InvalidMultiBindReason::ThisTargetReserved,
        None,
        Some(exact_span(4, 2)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    let direct = invalid_multi_bind_syntax(
        InvalidMultiBindReason::ThisTargetReserved,
        None,
        Some(exact_span(4, 2)),
    );
    assert_renders_like_direct(remapped, direct, &type_environment, context);
}

#[test]
fn label_words_remap_only_string_labels() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let input_name = source_strings.intern("alpha");
    let second_input_name = source_strings.intern("beta");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_input = domain.string_remap.get(input_name);
    let dest_second = domain.string_remap.get(second_input_name);
    assert_ne!(input_name, dest_input);
    assert_ne!(second_input_name, dest_second);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    let mut remapped = label_draft(input_name, second_input_name);
    remapped.remap_string_ids(&domain.string_remap);
    let direct = label_draft(dest_input, dest_second);
    assert_same_draft(&remapped, &direct);

    // The first two labels carry a string ID and the third carries none. Each decodes to the text its role names.
    let expected = [
        "configuration input `alpha` resolved from a builder-provided global".to_owned(),
        "configuration input `beta` resolved from a builder-provided global".to_owned(),
        "previous generic inference evidence here".to_owned(),
    ];
    assert_eq!(
        Stored::new(remapped, &type_environment).label_texts(context),
        expected
    );
    assert_eq!(
        Stored::new(direct, &type_environment).label_texts(context),
        expected
    );
}

#[test]
fn substitution_parameter_names_remap() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let parameter = source_strings.intern("T");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_parameter = domain.string_remap.get(parameter);
    assert_ne!(parameter, dest_parameter);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    let mut remapped = substitution_draft(parameter);
    remapped.remap_string_ids(&domain.string_remap);
    let direct = substitution_draft(dest_parameter);
    assert_same_draft(&remapped, &direct);

    let expected = ["generic substitution: T = I32".to_owned()];
    assert_eq!(
        Stored::new(remapped, &type_environment).label_texts(context),
        expected
    );
    assert_eq!(
        Stored::new(direct, &type_environment).label_texts(context),
        expected
    );
}

#[test]
fn foreign_sites_keep_their_span_slot_and_context() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let alpha = source_strings.intern("alpha");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    assert_ne!(alpha, domain.string_remap.get(alpha));

    // A foreign site's span, slot, and identity context live outside the string domain, so the remap leaves them alone.
    let mut remapped =
        duplicate_declaration(alpha, Some(exact_span(2, 3))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: identity(),
        });
    remapped.remap_string_ids(&domain.string_remap);
    let stored = Stored::new(remapped, &type_environment);
    assert!(matches!(
        stored
            .store
            .record(stored.id, &stored.foreign_contexts)
            .primary(),
        SiteRef::Foreign { span, .. } if span == exact_span(2, 3)
    ));
}

#[test]
fn path_index_past_the_place_field_faults_the_draft() {
    let type_environment = TypeEnvironment::new();
    // The place word keeps 30 bits for the path index. A destination index past that field cannot be stored, so the
    // remap turns the draft into a capacity fault instead of writing a truncated index.
    let deep = PathId::try_from_index(1 << 30).expect("the index fits a path id");
    let remap = PathIdRemap::from_full(vec![PathId::ROOT, deep]);
    let source_path = PathId::try_from_index(1).expect("the index fits a path id");
    let mut draft =
        use_of_uninitialized_local(DiagnosticPlace::Path(source_path), Some(exact_span(3, 2)));
    draft.remap_path_ids(&remap);
    assert!(
        draft.core.faulted,
        "a path index past the place field faults the draft"
    );

    let stored = Stored::new(draft, &type_environment);
    let view = CompilerCapacityExceededView::from_record(
        stored.store.record(stored.id, &stored.foreign_contexts),
    )
    .expect("the fault compacts to the capacity family");
    assert!(matches!(
        view.resource,
        CapacityResource::Store(DiagnosticCapacityResource::FactValue)
    ));
    assert!(matches!(
        stored
            .store
            .record(stored.id, &stored.foreign_contexts)
            .primary(),
        SiteRef::Local(_)
    ));
}

#[test]
fn conflicting_inference_names_live_in_the_extra_and_remap() {
    let type_environment = TypeEnvironment::new();
    let mut source_strings = StringTable::new();
    let type_name = source_strings.intern("Box");
    let parameter_name = source_strings.intern("T");
    let domain = merge_into_seeded(&source_strings, &PathInternerFork::empty());
    let dest_type = domain.string_remap.get(type_name);
    let dest_parameter = domain.string_remap.get(parameter_name);
    assert_ne!(type_name, dest_type);
    assert_ne!(parameter_name, dest_parameter);
    let paths = domain.paths.freeze();
    let context = render_context(&domain.strings, &paths);

    let mut remapped = invalid_generic_instantiation(
        Some(type_name),
        conflicting_inference(parameter_name, &type_environment),
        Some(exact_span(5, 4)),
    );
    remapped.remap_string_ids(&domain.string_remap);
    let direct = invalid_generic_instantiation(
        Some(dest_type),
        conflicting_inference(dest_parameter, &type_environment),
        Some(exact_span(5, 4)),
    );
    // The parameter name is stored before the type name, so the extra record holds both in that order.
    assert_eq!(extra_string_ids(&remapped), [dest_parameter, dest_type]);
    assert_renders_like_direct(remapped, direct, &type_environment, context);
}

#[test]
fn not_exported_by_public_surface_remaps_path_and_surface_name_after_prepared_capture() {
    let mut local_table = StringTable::new();
    let surface_name = local_table.intern("surface-name");
    let mut local_fork = PathInternerFork::empty();
    let requested_path = local_fork
        .try_intern_portable_path("module/root", &mut local_table)
        .expect("requested path fits");
    let draft = imports::not_exported_by_public_surface(
        requested_path,
        surface_name,
        ImportPublicSurfaceType::ModuleRoot,
        None,
    );
    let prepared = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(draft),
        local_table,
        None,
        || Some(Arc::new(local_fork.snapshot_table())),
    )
    .expect("requested path table pairs with local string table");
    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("collision-padding");
    let mut accumulator = BoundaryAccumulator::with_root_string_table(aggregate_table);
    accumulator.append_prepared(prepared, None);
    let failure = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let facts = view
        .family_view::<NotExportedByPublicSurfaceView>()
        .expect("public-surface facts remain typed");
    assert_eq!(facts.requested_path, requested_path);
    assert_eq!(
        facts.public_surface_type,
        ImportPublicSurfaceType::ModuleRoot
    );
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(facts.public_surface_name),
        "surface-name"
    );
    let rendered = terse::format_terse_compilation_result(&failure).join("\n");
    assert!(
        rendered.contains("Cannot bind 'module/root' from module 'surface-name'"),
        "public-surface remapping rendered incorrectly: {rendered}"
    );
}

#[test]
fn prepared_rebase_updates_payloads_labels_and_tokens() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let import_path = path_fork
        .try_intern_portable_path("lib.moth", &mut local_table)
        .expect("test path fits");
    let rendered_import = path_fork.render_portable(import_path, &local_table, &mut Vec::new());
    let name = local_table.intern("Button");
    let alias = local_table.intern("AliasButton");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let primary_span = exact_span_in_source(source, 4, 3, &mut span_builder);
    let first_span = exact_span_in_source(source, 10, 2, &mut span_builder);
    let expected_token = token_lexical::expected_token_from_projections(
        DiagnosticToken::from_string_tag(TokenTag::SYMBOL, name),
        Some(DiagnosticToken::from_static_tag(TokenTag::PATH)),
        Some(primary_span),
    );
    let duplicate = names_declarations::duplicate_declaration(name, Some(primary_span)).with_label(
        Some(first_span),
        Some(LabelDraftMessage::PreviousDeclaration),
    );
    let conformance = generics_traits_api::invalid_trait_conformance(
        alias,
        None,
        InvalidTraitConformanceReason::ImportedModuleRoot,
        Some(primary_span),
    )
    .with_label(
        Some(first_span),
        Some(LabelDraftMessage::TraitRequirementSite),
    );
    let borrow = types_borrows_targets::shared_mutable_conflict(
        DiagnosticPlace::Local(name),
        BorrowAccessKind::Shared,
        BorrowAccessKind::Mutable,
        None,
        Some(first_span),
        Some(primary_span),
    );
    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(
        vec![expected_token, duplicate, conformance, borrow],
        local_table,
        &mut merged_table,
        None,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let mut diagnostics = diagnosed_reports(&failure).iter();

    let (token_report, token_view) = diagnostics.next().expect("expected-token record");
    let token = token_view
        .family_view::<ExpectedTokenView>()
        .expect("expected-token family");
    assert_eq!(token.expected.tag(), TokenTag::SYMBOL);
    assert_eq!(
        token_report
            .context()
            .strings()
            .resolve(StringId::from_index(token.expected.data())),
        "Button"
    );
    let found = token.found.expect("found-token fact");
    assert_eq!(found.tag(), TokenTag::PATH);
    assert_eq!(found.data(), 0);
    assert_eq!(rendered_import, "lib.moth");
    assert!(matches!(token_view.primary_site(), SiteRef::Local(span) if span == primary_span));

    let (duplicate_report, duplicate_view) =
        diagnostics.next().expect("duplicate-declaration record");
    let duplicate = duplicate_view
        .family_view::<DuplicateDeclarationView>()
        .expect("duplicate family");
    assert_eq!(
        duplicate_report.context().strings().resolve(duplicate.name),
        "Button"
    );
    let previous = duplicate_view
        .durable_labels()
        .next()
        .expect("previous-location label");
    assert!(matches!(
        previous.message,
        DurableLabelMessage::PreviousDeclaration
    ));
    assert!(matches!(previous.site, SiteRef::Local(span) if span == first_span));
    assert!(matches!(duplicate_view.primary_site(), SiteRef::Local(span) if span == primary_span));

    let (conformance_report, conformance_view) =
        diagnostics.next().expect("trait-conformance record");
    let conformance = conformance_view
        .family_view::<InvalidTraitConformanceView>()
        .expect("trait-conformance family");
    assert!(matches!(
        conformance.shape,
        InvalidTraitConformanceShape::ImportedModuleRoot
    ));
    let target_name = conformance_view
        .string_ids()
        .first()
        .expect("trait target name");
    assert_eq!(
        conformance_report.context().strings().resolve(*target_name),
        "AliasButton"
    );
    let label = conformance_view
        .durable_labels()
        .next()
        .expect("authored trait label");
    assert!(matches!(
        label.message,
        DurableLabelMessage::TraitRequirementSite
    ));
    assert!(matches!(label.site, SiteRef::Local(span) if span == first_span));
    assert!(
        matches!(conformance_view.primary_site(), SiteRef::Local(span) if span == primary_span)
    );

    let (borrow_report, borrow_view) = diagnostics.next().expect("borrow-conflict record");
    let borrow = borrow_view
        .family_view::<SharedMutableConflictView>()
        .expect("borrow-conflict family");
    match borrow.place.diagnostic_place() {
        DiagnosticPlace::Local(name) => {
            assert_eq!(borrow_report.context().strings().resolve(name), "Button")
        }
        place => panic!("unexpected borrow place: {place:?}"),
    }
    assert_eq!(borrow.existing_access, BorrowAccessKind::Shared);
    assert_eq!(borrow.requested_access, BorrowAccessKind::Mutable);
    let conflict = borrow_view.durable_labels().next().expect("access label");
    assert!(matches!(
        conflict.message,
        DurableLabelMessage::ConflictingAccess
    ));
    assert!(matches!(conflict.site, SiteRef::Local(span) if span == first_span));
    assert!(matches!(borrow_view.primary_site(), SiteRef::Local(span) if span == primary_span));
    assert!(
        diagnostics.next().is_none(),
        "expected exactly four frozen records"
    );
}

#[test]
fn prepared_rebase_updates_legacy_dependency_replacement() {
    let mut local_table = StringTable::new();
    let replacement = local_table.intern("@vendor/drawing.js as drawing");
    let draft = project_config::legacy_dependency_clause(Some(replacement), None);
    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(vec![draft], local_table, &mut merged_table, None, None);
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let dependency = view
        .family_view::<LegacyDependencyClauseView>()
        .expect("dependency replacement facts");
    let replacement = dependency.replacement.expect("replacement text");
    assert_eq!(
        report.context().strings().resolve(replacement),
        "@vendor/drawing.js as drawing"
    );
}

#[test]
fn prepared_rebase_updates_compile_time_evaluation_operation() {
    let mut local_table = StringTable::new();
    let operation = local_table.intern("string equality comparison");
    let draft = choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
        Some(operation),
        None,
    );
    let mut merged_table = StringTable::new();
    merged_table.intern("a string owned by another module");
    merged_table.intern("a second string owned by another module");
    let prepared = capture_rebased_batch(vec![draft], local_table, &mut merged_table, None, None);
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let evaluation = view
        .family_view::<CompileTimeEvaluationErrorView>()
        .expect("compile-time evaluation facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(evaluation.operation.expect("operation name")),
        "string equality comparison"
    );
}

#[test]
fn prepared_rebase_updates_receiver_declaration_function_name_and_type() {
    let mut local_table = StringTable::new();
    let function_name = local_table.intern("describe");
    let type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let draft = generics_traits_api::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name,
            type_id,
        },
        None,
    );
    let type_environment = TypeEnvironment::new();
    let path_table = Arc::new(PathInternerFork::empty().snapshot_table());
    let mut merged_table = StringTable::new();
    merged_table.intern("a string owned by another module");
    merged_table.intern("a second string owned by another module");
    let prepared = capture_rebased_batch(
        vec![draft],
        local_table,
        &mut merged_table,
        Some(&type_environment),
        Some(path_table),
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let receiver = view
        .family_view::<InvalidReceiverDeclarationView>()
        .expect("receiver-declaration facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(receiver.function_name.expect("function name")),
        "describe"
    );
    assert_eq!(
        view.types().shape(receiver.type_id.expect("receiver type")),
        TypeDisplayShape::FixedScalar(FixedScalar::I32)
    );
}

#[test]
fn prepared_rebase_updates_invalid_call_shape_names() {
    let mut local_table = StringTable::new();
    let parameter = local_table.intern("values");
    let binding = local_table.intern("values");
    let callee = local_table.intern("consume");
    let draft = calls_data::invalid_call_shape(
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        Some(callee),
        None,
    );
    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(vec![draft], local_table, &mut merged_table, None, None);
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let call = view
        .family_view::<InvalidCallShapeView>()
        .expect("invalid-call-shape facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(call.callee_name.expect("callee")),
        "consume"
    );
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(call.parameter_name.expect("parameter")),
        "values"
    );
    let [binding] = view.string_ids() else {
        panic!("expected one frozen binding-name string ID")
    };
    assert_eq!(report.context().strings().resolve(*binding), "values");
}

#[test]
fn prepared_rebase_updates_invalid_receiver_call_binding_name() {
    let mut local_table = StringTable::new();
    let method = local_table.intern("move");
    let binding = local_table.intern("p");
    let draft = calls_data::invalid_receiver_call(
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        None,
        Some(method),
        Some(ReceiverCallKind::SourceMethod),
        Some(binding),
        None,
    );
    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(vec![draft], local_table, &mut merged_table, None, None);
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let call = view
        .family_view::<InvalidReceiverCallView>()
        .expect("receiver-call facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(call.method_name.expect("method name")),
        "move"
    );
    assert_eq!(
        calls_data::invalid_receiver_call_binding_name(view.string_ids())
            .map(|name| report.context().strings().resolve(name)),
        Some("p"),
    );
}

#[test]
fn prepared_rebase_preserves_source_spans_and_strings() {
    let mut local_table = StringTable::new();
    let name = local_table.intern("Button");
    let primary_source = SourceId::from_index(3);
    let related_source = SourceId::from_index(4);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span_in_source(primary_source, 4, 6, &mut builder);
    let related = exact_span_in_source(related_source, 18, 5, &mut builder);
    let draft = generics_traits_api::invalid_trait_conformance(
        name,
        None,
        InvalidTraitConformanceReason::ImportedModuleRoot,
        Some(primary),
    )
    .with_label(Some(related), Some(LabelDraftMessage::TraitRequirementSite));
    let mut merged_table = StringTable::new();
    merged_table.intern("preexisting string");
    let prepared = capture_rebased_batch(vec![draft], local_table, &mut merged_table, None, None);
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared diagnostic freezes");
    let conformance = view
        .family_view::<InvalidTraitConformanceView>()
        .expect("trait-conformance facts");
    assert!(matches!(
        conformance.shape,
        InvalidTraitConformanceShape::ImportedModuleRoot
    ));
    let target_name = view.string_ids().first().expect("trait target name");
    assert_eq!(report.context().strings().resolve(*target_name), "Button");
    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == primary));
    let label = view
        .durable_labels()
        .next()
        .expect("frozen trait-conformance label");
    assert!(matches!(
        label.message,
        DurableLabelMessage::TraitRequirementSite
    ));
    assert!(matches!(label.site, SiteRef::Local(span) if span == related));
}
