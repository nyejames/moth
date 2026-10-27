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
    ABSENT_PLACE_WORD, CharFact, CountFact, FactCapacityError, FactCodec, FactCursor, OptCharFact,
    OptCountFact, OptPlaceFact, OptStringFact, OptTokenFact, OptTypeFact, OptUnitFact, PathFact,
    PlaceFact, StoredPlace, StringFact, TokenFact, TypeFact, UnitCode, UnitFact,
    type_display_from_word, type_display_word,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    GenericInstantiationFact, GenericInstantiationShape, INVALID_GENERIC_INSTANTIATION,
    INVALID_TRAIT_CONFORMANCE, InvalidTraitConformanceFact, InvalidTraitConformanceNames,
    InvalidTraitConformanceShape, decode_generic_instantiation_extra,
    decode_invalid_trait_conformance_names,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::CapacityResourceFact;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TYPE_MISMATCH;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DraftLabel, DraftSubstitution, DraftType, GenericSubstitutionDiagnostic, LabelDraftMessage,
    LabelMessageCode, SecondaryDiagnosticLabel, encode_label,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{FactDescriptor, FamilyRow};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CapacityResource, DiagnosticCapacityResource, DiagnosticPlace,
    DiagnosticToken, GenericInferenceSubject, HirCapacityResource, TokenTag, TypeMismatchContext,
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
        CapacityResource::PathNodes,
        CapacityResource::Store(DiagnosticCapacityResource::Records),
        CapacityResource::Store(DiagnosticCapacityResource::FactValue),
        CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays),
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

fn encode(message: LabelDraftMessage) -> (DraftLabel, Vec<DraftSubstitution>) {
    let mut substitutions = Vec::new();
    let label = encode_label(Some(exact_span(1, 2)), Some(message), &mut substitutions)
        .expect("a small label always encodes");
    (label, substitutions)
}

#[test]
fn plain_labels_carry_their_code_and_span_only() {
    let (label, substitutions) = encode(LabelDraftMessage::ValueMovedHere);
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
            LabelDraftMessage::PreviousDeclaration,
            LabelMessageCode::PREVIOUS_DECLARATION,
        ),
        (
            LabelDraftMessage::ConflictingAccess,
            LabelMessageCode::CONFLICTING_ACCESS,
        ),
        (
            LabelDraftMessage::ValueMovedHere,
            LabelMessageCode::VALUE_MOVED_HERE,
        ),
        (
            LabelDraftMessage::ConfigInputOrigin {
                input_name: name,
                origin: BuildConfigValueOrigin::ExplicitInput,
            },
            LabelMessageCode::CONFIG_INPUT_ORIGIN,
        ),
        (
            LabelDraftMessage::TraitRequirementSite,
            LabelMessageCode::TRAIT_REQUIREMENT_SITE,
        ),
        (
            LabelDraftMessage::ReceiverMethodSite,
            LabelMessageCode::RECEIVER_METHOD_SITE,
        ),
        (
            LabelDraftMessage::GenericInstantiationBodySite,
            LabelMessageCode::GENERIC_INSTANTIATION_BODY_SITE,
        ),
        (
            LabelDraftMessage::GenericInstantiationDeclarationSite,
            LabelMessageCode::GENERIC_INSTANTIATION_DECLARATION_SITE,
        ),
        (
            LabelDraftMessage::GenericInstantiationSubstitutions {
                substitutions: Vec::new(),
            },
            LabelMessageCode::GENERIC_INSTANTIATION_SUBSTITUTIONS,
        ),
        (
            LabelDraftMessage::GenericInferencePreviousEvidence,
            LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE,
        ),
        (
            LabelDraftMessage::ImmutableBindingDeclaration,
            LabelMessageCode::IMMUTABLE_BINDING_DECLARATION,
        ),
        (
            LabelDraftMessage::TypedFailureProducer {
                error_type_id: TypeId(1),
            },
            LabelMessageCode::TYPED_FAILURE_PRODUCER,
        ),
        (
            LabelDraftMessage::ImplicitFailureProducer,
            LabelMessageCode::IMPLICIT_FAILURE_PRODUCER,
        ),
        (
            LabelDraftMessage::BuiltinFailureCall,
            LabelMessageCode::BUILTIN_FAILURE_CALL,
        ),
        (
            LabelDraftMessage::BuiltinFailureOrigin,
            LabelMessageCode::BUILTIN_FAILURE_ORIGIN,
        ),
        (
            LabelDraftMessage::BuiltinWriteBackOrigin {
                target_type_id: TypeId(1),
            },
            LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN,
        ),
    ];
    assert_eq!(cases.len(), 16);
    for (message, code) in cases {
        let (label, _) = encode(message);
        assert_eq!(label.message, code);
    }
}

#[test]
fn type_carrying_labels_mark_their_type_word_pending() {
    let (typed, _) = encode(LabelDraftMessage::TypedFailureProducer {
        error_type_id: TypeId(7),
    });
    assert_eq!(typed.pending_types, 0b01);
    assert_eq!(typed.data, [7, 0]);

    let (written, _) = encode(LabelDraftMessage::BuiltinWriteBackOrigin {
        target_type_id: TypeId(8),
    });
    assert_eq!(written.pending_types, 0b01);
    assert_eq!(written.data, [8, 0]);
}

#[test]
fn config_origin_label_stores_its_string_index() {
    let (config, _) = encode(LabelDraftMessage::ConfigInputOrigin {
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
    let message = LabelDraftMessage::GenericInstantiationSubstitutions {
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
        Some(LabelDraftMessage::ConfigInputOrigin {
            input_name: StringId::from_index(u32::MAX),
            origin: BuildConfigValueOrigin::ExplicitInput,
        }),
        &mut substitutions,
    );
    assert!(result.is_err());
}

/// A test-only family whose one fact is an optional type. It is never registered, so no render path reaches it.
static OPT_TYPE_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9002),
    category: DiagnosticCategory::Rule,
    reserved: false,
    descriptor: crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
        "MOTH-TEST-9002",
        "Test family",
        crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error,
    ),
    layout: &[FactDescriptor::of::<OptTypeFact>()],
    extra: ExtraDataKind::None,
    labels: &[],
    render: None,
    reason_key: |_| None,
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
        Some(LabelDraftMessage::TypedFailureProducer {
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

#[test]
fn generic_instantiation_shapes_capture_only_their_type_words() {
    let type_name_id = StringId::from_index(20);
    let type_name = Some(type_name_id);
    let parameter_name = StringId::from_index(30);
    let trait_name = StringId::from_index(31);
    let cases = [
        (
            GenericInstantiationShape::WrongArgumentCount {
                type_name,
                expected: 2,
                found: 3,
            },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::TypeDoesNotAcceptArguments { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::OptionTypeSyntaxNotSupported { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::ResultTypeSyntaxNotSupported { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::ExternalTypeArgumentsUnsupported { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::MissingTypeArguments { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::CannotInferArguments { type_name },
            vec![parameter_name],
            0,
        ),
        (
            GenericInstantiationShape::CannotInferArguments { type_name },
            vec![parameter_name, trait_name],
            0,
        ),
        (
            GenericInstantiationShape::CannotInferFunctionArguments { type_name },
            vec![parameter_name],
            0,
        ),
        (
            GenericInstantiationShape::ConflictingInference {
                subject: GenericInferenceSubject::Function,
                existing_type: TypeId(40),
                replacement_type: TypeId(41),
            },
            vec![parameter_name, type_name_id],
            0b1100,
        ),
        (
            GenericInstantiationShape::ConflictingInference {
                subject: GenericInferenceSubject::NominalType,
                existing_type: TypeId(44),
                replacement_type: TypeId(45),
            },
            vec![parameter_name],
            0b1100,
        ),
        (
            GenericInstantiationShape::MissingTraitEvidence {
                type_name,
                concrete_type: TypeId(42),
            },
            vec![parameter_name, trait_name],
            0b0100,
        ),
        (
            GenericInstantiationShape::MissingNominalTraitEvidence {
                type_name,
                concrete_type: TypeId(43),
            },
            vec![parameter_name, trait_name],
            0b0100,
        ),
        (
            GenericInstantiationShape::RecursiveFunctionInstantiation { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::ExplicitCallTypeArgumentsUnsupported { type_name },
            vec![],
            0,
        ),
        (
            GenericInstantiationShape::GenericFunctionValueDeferred { type_name },
            vec![],
            0,
        ),
    ];

    for (shape, extra, pending_mask) in cases {
        assert!(
            decode_generic_instantiation_extra(shape.code(), &extra).is_some(),
            "each reason code accepts its StringIds suffix"
        );
        assert_eq!(GenericInstantiationFact::pending_mask(&shape), pending_mask);
        let mut draft =
            DiagnosticDraft::with_facts(&INVALID_GENERIC_INSTANTIATION, None, |cursor| {
                cursor.push::<GenericInstantiationFact>(&shape)
            })
            .with_string_ids(extra);
        assert_eq!(draft.core.pending_facts, pending_mask);
        let before = draft.core.facts;
        let mut rewritten = Vec::new();
        draft
            .capture_types(&mut |type_id| {
                rewritten.push(type_id);
                TypeDisplayId::from_index(type_id.0 as usize)
            })
            .expect("test type ids fit the display id space");

        assert_eq!(rewritten.len(), pending_mask.count_ones() as usize);
        for (index, (before, after)) in before
            .iter()
            .zip(draft.core.facts.iter())
            .enumerate()
            .take(GenericInstantiationFact::WORDS)
        {
            if pending_mask & (1 << index) != 0 {
                assert_eq!(*after, *before + 1);
            } else {
                assert_eq!(*after, *before);
            }
        }
        assert!(
            GenericInstantiationFact::decode(&draft.core.facts[..4]).is_some(),
            "every encoded generic-instantiation shape decodes after capture"
        );
    }
}

#[test]
fn invalid_trait_conformance_shapes_capture_only_their_type_words() {
    use InvalidTraitConformanceShape as Shape;

    let target_name = StringId::from_index(20);
    let trait_name = StringId::from_index(21);
    let requirement_name = StringId::from_index(22);
    let incompatible_trait_name = StringId::from_index(23);
    type Case = (
        Shape<TypeId>,
        Shape<TypeDisplayId>,
        Vec<StringId>,
        InvalidTraitConformanceNames,
        u8,
    );
    let cases: Vec<Case> = vec![
        (
            Shape::ImportedModuleRoot,
            Shape::ImportedModuleRoot,
            vec![target_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: None,
            },
            0,
        ),
        (
            Shape::AliasTarget,
            Shape::AliasTarget,
            vec![target_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::NonCanonicalTarget,
            Shape::NonCanonicalTarget,
            vec![target_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: None,
            },
            0,
        ),
        (
            Shape::NonlocalSourceTarget,
            Shape::NonlocalSourceTarget,
            vec![target_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::BuiltinTarget,
            Shape::BuiltinTarget,
            vec![target_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::ExternalOpaqueTarget,
            Shape::ExternalOpaqueTarget,
            vec![target_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: None,
            },
            0,
        ),
        (
            Shape::DuplicateCanonicalEvidence,
            Shape::DuplicateCanonicalEvidence,
            vec![target_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::BuiltinEvidenceOverride,
            Shape::BuiltinEvidenceOverride,
            vec![target_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: None,
                trait_name: None,
            },
            0,
        ),
        (
            Shape::IncompatibleTraitEvidence,
            Shape::IncompatibleTraitEvidence,
            vec![target_name, incompatible_trait_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(incompatible_trait_name),
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::MissingMethod,
            Shape::MissingMethod,
            vec![target_name, requirement_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: None,
            },
            0,
        ),
        (
            Shape::ReceiverMutabilityMismatch,
            Shape::ReceiverMutabilityMismatch,
            vec![target_name, requirement_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::ParameterCountMismatch {
                expected: 2,
                found: 3,
            },
            Shape::ParameterCountMismatch {
                expected: 2,
                found: 3,
            },
            vec![target_name, requirement_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::ParameterModeMismatch { parameter_index: 1 },
            Shape::ParameterModeMismatch { parameter_index: 1 },
            vec![target_name, requirement_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: None,
            },
            0,
        ),
        (
            Shape::ParameterContractMismatch { parameter_index: 2 },
            Shape::ParameterContractMismatch { parameter_index: 2 },
            vec![target_name, requirement_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: Some(trait_name),
            },
            0,
        ),
        (
            Shape::ParameterTypeMismatch {
                parameter_index: 3,
                expected_type: TypeId(40),
                found_type: TypeId(41),
            },
            Shape::ParameterTypeMismatch {
                parameter_index: 3,
                expected_type: TypeDisplayId::from_index(40).expect("display id fits"),
                found_type: TypeDisplayId::from_index(41).expect("display id fits"),
            },
            vec![target_name, requirement_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: Some(trait_name),
            },
            0b1100,
        ),
        (
            Shape::ReturnCountMismatch {
                expected: 2,
                found: 1,
            },
            Shape::ReturnCountMismatch {
                expected: 2,
                found: 1,
            },
            vec![target_name, requirement_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: None,
            },
            0,
        ),
        (
            Shape::ReturnTypeMismatch {
                return_index: 4,
                expected_type: TypeId(42),
                found_type: TypeId(43),
            },
            Shape::ReturnTypeMismatch {
                return_index: 4,
                expected_type: TypeDisplayId::from_index(42).expect("display id fits"),
                found_type: TypeDisplayId::from_index(43).expect("display id fits"),
            },
            vec![target_name, requirement_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: Some(trait_name),
            },
            0b1100,
        ),
        (
            Shape::ReturnChannelMismatch { return_index: 5 },
            Shape::ReturnChannelMismatch { return_index: 5 },
            vec![target_name, requirement_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: None,
            },
            0,
        ),
        (
            Shape::InferredFailureInInfallibleCast,
            Shape::InferredFailureInInfallibleCast,
            vec![target_name, requirement_name, trait_name],
            InvalidTraitConformanceNames {
                target_name,
                reason_name: Some(requirement_name),
                trait_name: Some(trait_name),
            },
            0,
        ),
    ];

    for (shape, durable_shape, extra, expected_names, pending_mask) in cases {
        let code = shape.code();
        assert_eq!(
            decode_invalid_trait_conformance_names(code, &extra),
            Some(expected_names)
        );
        assert_eq!(
            InvalidTraitConformanceFact::pending_mask(&shape),
            pending_mask
        );
        let mut draft = DiagnosticDraft::with_facts(&INVALID_TRAIT_CONFORMANCE, None, |cursor| {
            cursor.push::<InvalidTraitConformanceFact>(&shape)
        })
        .with_string_ids(extra);
        assert_eq!(draft.core.pending_facts, pending_mask);
        let before = draft.core.facts;
        let mut rewritten = Vec::new();
        draft
            .capture_types(&mut |type_id| {
                rewritten.push(type_id);
                TypeDisplayId::from_index(type_id.0 as usize)
            })
            .expect("test type ids fit the display id space");

        assert_eq!(rewritten.len(), pending_mask.count_ones() as usize);
        for (index, (before, after)) in before
            .iter()
            .zip(draft.core.facts.iter())
            .enumerate()
            .take(InvalidTraitConformanceFact::WORDS)
        {
            if pending_mask & (1 << index) != 0 {
                assert_eq!(*after, *before + 1);
            } else {
                assert_eq!(*after, *before);
            }
        }
        assert_eq!(
            InvalidTraitConformanceFact::decode(&draft.core.facts[..4]),
            Some(durable_shape)
        );
    }
}

#[test]
#[should_panic(expected = "every required pending type word is present")]
fn static_rows_reject_a_missing_pending_type_word() {
    let expected = 2usize;
    let found = TypeId(3);
    let context = TypeMismatchContext::General;
    let mut cursor = FactCursor::new();
    cursor
        .push::<CountFact>(&expected)
        .expect("the deliberately wrong count fact encodes");
    cursor
        .push::<TypeFact>(&found)
        .expect("the found type fact encodes");
    cursor
        .push::<UnitFact<TypeMismatchContext>>(&context)
        .expect("the context code encodes");
    cursor.finish_for(&TYPE_MISMATCH);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestCode {
    One,
    Two,
}

impl UnitCode for TestCode {
    fn code(self) -> u32 {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }

    fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            _ => None,
        }
    }
}

#[test]
fn opt_unit_fact_round_trips_present_and_absent_values() {
    let present = Some(TestCode::Two);
    let mut words = [0];
    OptUnitFact::<TestCode>::encode(&present, &mut words).expect("a unit code fits one fact word");
    assert_eq!(OptUnitFact::<TestCode>::decode(&words), Some(present));

    let absent = None;
    OptUnitFact::<TestCode>::encode(&absent, &mut words).expect("absence fits one fact word");
    assert_eq!(OptUnitFact::<TestCode>::decode(&words), Some(absent));
}

#[test]
fn opt_char_fact_round_trips_present_and_absent_values() {
    let present = Some('\0');
    let mut words = [0];
    OptCharFact::encode(&present, &mut words).expect("a character fits one fact word");
    assert_eq!(OptCharFact::decode(&words), Some(present));

    let absent = None;
    OptCharFact::encode(&absent, &mut words).expect("absence fits one fact word");
    assert_eq!(OptCharFact::decode(&words), Some(absent));
}

#[test]
fn opt_place_fact_reserves_an_invalid_place_word_and_round_trips() {
    let mut words = [0];
    let present = Some(DiagnosticPlace::Unknown);
    OptPlaceFact::encode(&present, &mut words).expect("an unknown place fits one fact word");
    assert_eq!(words, [0]);
    assert_eq!(
        OptPlaceFact::decode(&words),
        Some(Some(StoredPlace::Unknown))
    );

    let absent: Option<DiagnosticPlace> = None;
    OptPlaceFact::encode(&absent, &mut words).expect("absence fits one fact word");
    assert_eq!(words, [ABSENT_PLACE_WORD]);
    assert_eq!(PlaceFact::decode(&words), None);
    assert_eq!(OptPlaceFact::decode(&words), Some(None));
}

#[test]
fn opt_place_fact_path_visitor_forwards_present_paths_only() {
    let path = PathId::try_from_index(11).expect("a small index always has a path id");
    let mut words = [0];
    OptPlaceFact::encode(&Some(DiagnosticPlace::Path(path)), &mut words)
        .expect("a path place fits one fact word");
    let mut paths = Vec::new();
    OptPlaceFact::append_path_ids(&words, &mut paths);
    assert_eq!(paths, [path]);

    OptPlaceFact::encode(&None, &mut words).expect("absence fits one fact word");
    paths.clear();
    OptPlaceFact::append_path_ids(&words, &mut paths);
    assert!(paths.is_empty());
}
