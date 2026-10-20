//! Codec round trips and rejected word patterns.
//!
//! WHAT: each fact codec encodes a draft value into fixed words and decodes it back. Reserved or
//! malformed patterns decode to `None`.
//! WHY: a codec slip silently changes what a stored record means, so every reserved pattern gets
//! a case.

use super::exact_span;
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    CharFact, CountFact, FactCapacityError, FactCodec, OptCountFact, OptStringFact, OptTokenFact,
    OptTypeFact, PathFact, PlaceFact, StoredPlace, StringFact, TokenFact, TypeFact, UnitCode,
    UnitFact, type_display_from_word, type_display_word,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::CapacityResourceFact;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DraftLabel, DraftSubstitution, DraftType, LabelMessageCode, SecondaryDiagnosticLabel,
    encode_label,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{FactDescriptor, FamilyRow};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CapacityResource, DiagnosticCapacityResource, DiagnosticLabelMessage,
    DiagnosticPlace, DiagnosticToken, GenericSubstitutionDiagnostic, HirCapacityResource, TokenTag,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

#[test]
fn token_fact_round_trips_static_and_string_tokens() {
    let mut words = [0u32; 2];

    let static_token = DiagnosticToken::from_static_tag(TokenTag::COMMA);
    TokenFact::encode(&static_token, &mut words).expect("a static token always encodes");
    let decoded = TokenFact::decode(&words).expect("an encoded static token decodes");
    assert_eq!(decoded.raw_words(), static_token.raw_words());

    let symbol = DiagnosticToken::from_string_tag(TokenTag::SYMBOL, StringId::from_index(7));
    TokenFact::encode(&symbol, &mut words).expect("a symbol token always encodes");
    let decoded = TokenFact::decode(&words).expect("an encoded symbol token decodes");
    assert_eq!(decoded.raw_words(), symbol.raw_words());
}

#[test]
fn token_fact_rejects_the_absent_marker() {
    assert!(TokenFact::decode(&[u32::MAX, 0]).is_none());
}

#[test]
fn optional_token_fact_writes_absence_and_presence() {
    let mut words = [0u32; 2];
    OptTokenFact::encode(&None, &mut words).expect("absence always encodes");
    assert_eq!(words, [u32::MAX, 0]);
    assert!(matches!(OptTokenFact::decode(&words), Some(None)));

    let expected = DiagnosticToken::from_static_tag(TokenTag::COMMA).raw_words();
    let present = Some(DiagnosticToken::from_static_tag(TokenTag::COMMA));
    OptTokenFact::encode(&present, &mut words).expect("a present token always encodes");
    let decoded = OptTokenFact::decode(&words)
        .flatten()
        .expect("a present token decodes");
    assert_eq!(decoded.raw_words(), expected);
}

#[test]
fn string_fact_round_trips_and_rejects_the_absent_marker() {
    let mut word = [0u32; 1];
    StringFact::encode(&StringId::from_index(12), &mut word).expect("a string id always encodes");
    assert_eq!(word, [12]);
    assert_eq!(StringFact::decode(&word).map(|id| id.index()), Some(12));
    assert!(StringFact::decode(&[u32::MAX]).is_none());
    assert_eq!(
        StringFact::encode(&StringId::from_index(u32::MAX), &mut word),
        Err(FactCapacityError)
    );
}

#[test]
fn optional_string_fact_uses_the_absent_marker() {
    let mut word = [0u32; 1];
    OptStringFact::encode(&None, &mut word).expect("absence always encodes");
    assert_eq!(word, [u32::MAX]);
    assert!(matches!(OptStringFact::decode(&word), Some(None)));

    OptStringFact::encode(&Some(StringId::from_index(3)), &mut word)
        .expect("a present string id always encodes");
    assert!(matches!(OptStringFact::decode(&word), Some(Some(id)) if id.index() == 3));
}

#[test]
fn path_fact_round_trips_and_rejects_the_absent_marker() {
    let path = PathId::try_from_index(5).expect("a small index always has a path id");
    let mut word = [0u32; 1];
    PathFact::encode(&path, &mut word).expect("a small path id always encodes");
    assert_eq!(PathFact::decode(&word), Some(path));
    assert!(PathFact::decode(&[u32::MAX]).is_none());
}

#[test]
fn count_fact_round_trips_small_counts() {
    let mut word = [0u32; 1];
    CountFact::encode(&7, &mut word).expect("a small count always encodes");
    assert_eq!(word, [7]);
    assert_eq!(CountFact::decode(&word), Some(7));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn count_fact_rejects_counts_beyond_u32() {
    let mut word = [0u32; 1];
    let too_large = u32::MAX as usize + 1;
    assert_eq!(
        CountFact::encode(&too_large, &mut word),
        Err(FactCapacityError)
    );
}

#[test]
fn place_fact_round_trips_each_place_kind() {
    let mut word = [0u32; 1];

    PlaceFact::encode(&DiagnosticPlace::Local(StringId::from_index(4)), &mut word)
        .expect("a local name always encodes");
    assert!(matches!(PlaceFact::decode(&word), Some(StoredPlace::Local(id)) if id.index() == 4));

    let path = PathId::try_from_index(9).expect("a small index always has a path id");
    PlaceFact::encode(&DiagnosticPlace::Path(path), &mut word).expect("a path always encodes");
    assert_eq!(PlaceFact::decode(&word), Some(StoredPlace::Path(path)));

    PlaceFact::encode(&DiagnosticPlace::Unknown, &mut word)
        .expect("an unknown place always encodes");
    assert_eq!(word, [0]);
    assert_eq!(PlaceFact::decode(&word), Some(StoredPlace::Unknown));

    PlaceFact::encode(
        &DiagnosticPlace::RenderedText(StringId::from_index(6)),
        &mut word,
    )
    .expect("rendered text always encodes");
    assert!(matches!(PlaceFact::decode(&word), Some(StoredPlace::Local(id)) if id.index() == 6));
}

#[test]
fn place_fact_rejects_overflowing_names_and_unknown_tags() {
    let mut word = [0u32; 1];
    assert_eq!(
        PlaceFact::encode(
            &DiagnosticPlace::Local(StringId::from_index(1 << 30)),
            &mut word
        ),
        Err(FactCapacityError)
    );
    assert!(PlaceFact::decode(&[0b11u32 << 30]).is_none());
    assert!(PlaceFact::decode(&[5]).is_none());
}

#[test]
fn type_fact_stores_raw_ids_until_capture_and_reads_display_ids() {
    let mut word = [0u32; 1];
    TypeFact::encode(&TypeId(3), &mut word).expect("a raw type id always encodes");
    assert_eq!(word, [3]);

    let display = TypeDisplayId::from_index(4).expect("a small index always has a display id");
    let display_word = type_display_word(display).expect("a small display id always encodes");
    assert_eq!(
        type_display_from_word(display_word).map(|id| id.index()),
        Some(4)
    );
    assert!(type_display_from_word(0).is_none());
    assert!(TypeFact::decode(&[0]).is_none());
}

#[test]
fn unit_fact_round_trips_every_type_mismatch_context() {
    for code in 1..=17 {
        let context = TypeMismatchContext::from_code(code).expect("codes 1 to 17 are assigned");
        assert_eq!(context.code(), code);
        let mut word = [0u32; 1];
        UnitFact::<TypeMismatchContext>::encode(&context, &mut word)
            .expect("a context always encodes");
        assert_eq!(
            UnitFact::<TypeMismatchContext>::decode(&word),
            Some(context)
        );
    }
    assert!(TypeMismatchContext::from_code(0).is_none());
    assert!(TypeMismatchContext::from_code(18).is_none());
}

#[test]
fn capacity_resource_fact_round_trips_every_group() {
    let resources = [
        CapacityResource::Hir(HirCapacityResource::ExpressionRows),
        CapacityResource::Ast(AstCapacityResource::ExpressionRows),
        CapacityResource::Store(DiagnosticCapacityResource::Records),
        CapacityResource::Store(DiagnosticCapacityResource::FactValue),
    ];
    for resource in resources {
        let mut word = [0u32; 1];
        CapacityResourceFact::encode(&resource, &mut word)
            .expect("a capacity resource always encodes");
        assert_eq!(CapacityResourceFact::decode(&word), Some(resource));
    }
}

#[test]
fn secondary_label_packs_its_code_and_data_index() {
    let label = SecondaryDiagnosticLabel::pack(None, LabelMessageCode::CONFIG_INPUT_ORIGIN, 9);
    assert_eq!(label.message_code(), LabelMessageCode::CONFIG_INPUT_ORIGIN);
    assert_eq!(label.data_index(), Some(9));
    assert!(label.span().is_none());

    let plain = SecondaryDiagnosticLabel::pack(None, LabelMessageCode::PREVIOUS_DECLARATION, 0);
    assert_eq!(plain.message_code(), LabelMessageCode::PREVIOUS_DECLARATION);
    assert_eq!(plain.data_index(), None);
}

#[test]
#[should_panic(expected = "fit in 24 bits")]
fn secondary_label_rejects_a_data_index_beyond_24_bits() {
    let _label =
        SecondaryDiagnosticLabel::pack(None, LabelMessageCode::CONFIG_INPUT_ORIGIN, 1 << 24);
}

fn encode(message: DiagnosticLabelMessage) -> (DraftLabel, Vec<DraftSubstitution>) {
    let mut substitutions = Vec::new();
    let label = encode_label(Some(exact_span(1, 2)), Some(message), &mut substitutions)
        .expect("a small label always encodes");
    (label, substitutions)
}

#[test]
fn plain_labels_carry_their_code_and_span_only() {
    let (label, substitutions) = encode(DiagnosticLabelMessage::ValueMovedHere);
    assert_eq!(label.message, LabelMessageCode::VALUE_MOVED_HERE);
    assert!(label.span.is_some());
    assert_eq!(label.pending_types, 0);
    assert_eq!(label.data, [0, 0]);
    assert!(substitutions.is_empty());
}

#[test]
fn message_less_labels_encode_to_none_and_keep_their_span() {
    let mut substitutions = Vec::new();
    let label = encode_label(Some(exact_span(1, 2)), None, &mut substitutions)
        .expect("a message-less label always encodes");
    assert_eq!(label.message, LabelMessageCode::NONE);
    assert!(label.span.is_some());
    assert_eq!(label.pending_types, 0);
    assert_eq!(label.data, [0, 0]);
    assert!(substitutions.is_empty());
}

#[test]
fn every_label_message_maps_to_its_code() {
    let name = StringId::from_index(1);
    let cases = [
        (
            DiagnosticLabelMessage::PreviousDeclaration,
            LabelMessageCode::PREVIOUS_DECLARATION,
        ),
        (
            DiagnosticLabelMessage::ConflictingAccess,
            LabelMessageCode::CONFLICTING_ACCESS,
        ),
        (
            DiagnosticLabelMessage::ValueMovedHere,
            LabelMessageCode::VALUE_MOVED_HERE,
        ),
        (
            DiagnosticLabelMessage::ConfigInputOrigin {
                input_name: name,
                origin: BuildConfigValueOrigin::ExplicitInput,
            },
            LabelMessageCode::CONFIG_INPUT_ORIGIN,
        ),
        (
            DiagnosticLabelMessage::RenderedText(name),
            LabelMessageCode::RENDERED_TEXT,
        ),
        (
            DiagnosticLabelMessage::GenericInstantiationBodySite,
            LabelMessageCode::GENERIC_INSTANTIATION_BODY_SITE,
        ),
        (
            DiagnosticLabelMessage::GenericInstantiationDeclarationSite,
            LabelMessageCode::GENERIC_INSTANTIATION_DECLARATION_SITE,
        ),
        (
            DiagnosticLabelMessage::GenericInstantiationSubstitutions {
                substitutions: Vec::new(),
            },
            LabelMessageCode::GENERIC_INSTANTIATION_SUBSTITUTIONS,
        ),
        (
            DiagnosticLabelMessage::GenericInferencePreviousEvidence,
            LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE,
        ),
        (
            DiagnosticLabelMessage::ImmutableBindingDeclaration,
            LabelMessageCode::IMMUTABLE_BINDING_DECLARATION,
        ),
        (
            DiagnosticLabelMessage::TypedFailureProducer {
                error_type_id: TypeId(1),
            },
            LabelMessageCode::TYPED_FAILURE_PRODUCER,
        ),
        (
            DiagnosticLabelMessage::ImplicitFailureProducer,
            LabelMessageCode::IMPLICIT_FAILURE_PRODUCER,
        ),
        (
            DiagnosticLabelMessage::BuiltinFailureCall,
            LabelMessageCode::BUILTIN_FAILURE_CALL,
        ),
        (
            DiagnosticLabelMessage::BuiltinFailureOrigin,
            LabelMessageCode::BUILTIN_FAILURE_ORIGIN,
        ),
        (
            DiagnosticLabelMessage::BuiltinWriteBackOrigin {
                target_type_id: TypeId(1),
            },
            LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN,
        ),
    ];
    assert_eq!(cases.len(), 15);
    for (message, code) in cases {
        let (label, _) = encode(message);
        assert_eq!(label.message, code);
    }
}

#[test]
fn type_carrying_labels_mark_their_type_word_pending() {
    let (typed, _) = encode(DiagnosticLabelMessage::TypedFailureProducer {
        error_type_id: TypeId(7),
    });
    assert_eq!(typed.pending_types, 0b01);
    assert_eq!(typed.data, [7, 0]);

    let (written, _) = encode(DiagnosticLabelMessage::BuiltinWriteBackOrigin {
        target_type_id: TypeId(8),
    });
    assert_eq!(written.pending_types, 0b01);
    assert_eq!(written.data, [8, 0]);
}

#[test]
fn string_carrying_labels_store_their_string_index() {
    let (rendered, _) = encode(DiagnosticLabelMessage::RenderedText(StringId::from_index(
        9,
    )));
    assert_eq!(rendered.data, [9, 0]);

    let (config, _) = encode(DiagnosticLabelMessage::ConfigInputOrigin {
        input_name: StringId::from_index(4),
        origin: BuildConfigValueOrigin::ExplicitInput,
    });
    assert_eq!(config.data, [4, 1]);
    assert_eq!(config.pending_types, 0);
}

#[test]
fn generic_substitution_labels_append_pending_entries() {
    let mut substitutions = vec![DraftSubstitution {
        parameter: 0,
        concrete: DraftType::Captured(1),
    }];
    let message = DiagnosticLabelMessage::GenericInstantiationSubstitutions {
        substitutions: vec![
            GenericSubstitutionDiagnostic {
                parameter_name: StringId::from_index(5),
                concrete_type_id: TypeId(7),
            },
            GenericSubstitutionDiagnostic {
                parameter_name: StringId::from_index(6),
                concrete_type_id: TypeId(8),
            },
        ],
    };
    let label = encode_label(None, Some(message), &mut substitutions)
        .expect("two substitutions always encode");
    assert_eq!(label.data, [1, 2]);
    assert_eq!(label.pending_types, 0);
    assert_eq!(substitutions.len(), 3);
    assert_eq!(substitutions[1].parameter, 5);
    assert_eq!(substitutions[1].concrete, DraftType::Pending(7));
    assert_eq!(substitutions[2].concrete, DraftType::Pending(8));
}

#[test]
fn label_encoding_rejects_a_name_beyond_the_fact_range() {
    let mut substitutions = Vec::new();
    let result = encode_label(
        None,
        Some(DiagnosticLabelMessage::RenderedText(StringId::from_index(
            u32::MAX,
        ))),
        &mut substitutions,
    );
    assert!(result.is_err());
}

/// A test-only family whose one fact is an optional type. It is never registered, so no render path reaches it.
static OPT_TYPE_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9002),
    stable_code: "MOTH-TEST-9002",
    category: DiagnosticCategory::Rule,
    reserved: false,
    descriptor: None,
    layout: &[FactDescriptor::of::<OptTypeFact>()],
    extra: ExtraDataKind::None,
    labels: &[],
    render: None,
};

#[test]
fn char_fact_round_trips_its_boundaries_and_rejects_non_scalars() {
    let mut word = [0u32; 1];
    for value in ['\0', 'A', char::MAX] {
        CharFact::encode(&value, &mut word).expect("every char encodes");
        assert_eq!(CharFact::decode(&word), Some(value));
    }
    assert!(CharFact::decode(&[0xD800]).is_none());
    assert!(CharFact::decode(&[0x11_0000]).is_none());
}

#[test]
fn optional_count_fact_round_trips_zero_none_and_the_largest_count() {
    let mut word = [0u32; 1];
    for value in [Some(0), None, Some(u32::MAX as usize - 1)] {
        OptCountFact::encode(&value, &mut word).expect("every representable count encodes");
        assert_eq!(OptCountFact::decode(&word), Some(value));
    }
}

#[test]
fn optional_count_fact_rejects_the_absent_marker_as_a_count() {
    let mut word = [0u32; 1];
    assert!(OptCountFact::encode(&Some(u32::MAX as usize), &mut word).is_err());
    assert_eq!(OptCountFact::decode(&[u32::MAX]), Some(None));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn optional_count_fact_rejects_a_count_past_u32() {
    let mut word = [0u32; 1];
    assert!(OptCountFact::encode(&Some(1 << 32), &mut word).is_err());
}

#[test]
fn optional_type_fact_encodes_absence_as_the_sentinel() {
    let mut word = [0u32; 1];
    OptTypeFact::encode(&None, &mut word).expect("absence always encodes");
    assert_eq!(word, [u32::MAX]);

    OptTypeFact::encode(&Some(TypeId(3)), &mut word).expect("a small type id encodes");
    assert_eq!(word, [3]);
}

#[test]
fn pending_type_words_refuse_the_absent_marker_as_an_id() {
    let mut word = [0u32; 1];
    assert!(TypeFact::encode(&TypeId(u32::MAX), &mut word).is_err());
    assert!(OptTypeFact::encode(&Some(TypeId(u32::MAX)), &mut word).is_err());
    let label = encode_label(
        None,
        Some(DiagnosticLabelMessage::TypedFailureProducer {
            error_type_id: TypeId(u32::MAX),
        }),
        &mut Vec::new(),
    );
    assert!(label.is_err());
}

#[test]
fn optional_type_fact_absent_survives_capture_without_calling_rewrite() {
    let mut draft =
        DiagnosticDraft::with_facts(&OPT_TYPE_FAMILY, Some(exact_span(0, 1)), |cursor| {
            cursor.push::<OptTypeFact>(&None)
        });
    let mut rewrite_calls = 0;
    draft
        .capture_types(&mut |id: TypeId| {
            rewrite_calls += 1;
            TypeDisplayId::from_index(id.0 as usize)
        })
        .expect("an absent type captures without a rewrite");

    assert_eq!(rewrite_calls, 0);
    assert_eq!(draft.core.pending_facts, 0);
    assert_eq!(draft.core.facts[0], 0, "no type captures as the zero word");
    assert_eq!(OptTypeFact::decode(&draft.core.facts[..1]), Some(None));
}

#[test]
fn optional_type_fact_rewrites_a_present_type_through_capture() {
    let mut draft =
        DiagnosticDraft::with_facts(&OPT_TYPE_FAMILY, Some(exact_span(0, 1)), |cursor| {
            cursor.push::<OptTypeFact>(&Some(TypeId(3)))
        });
    let mut rewritten = Vec::new();
    draft
        .capture_types(&mut |id: TypeId| {
            rewritten.push(id.0);
            TypeDisplayId::from_index(id.0 as usize)
        })
        .expect("a small type id captures");

    assert_eq!(rewritten, vec![3]);
    assert_eq!(draft.core.pending_facts, 0);
    assert_eq!(draft.core.facts[0], 4, "display words are one-based");
    assert_eq!(
        OptTypeFact::decode(&draft.core.facts[..1]).map(|id| id.map(|id| id.index())),
        Some(Some(3))
    );
}
