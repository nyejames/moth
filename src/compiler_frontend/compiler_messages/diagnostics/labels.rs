//! Secondary labels in the compact store, and the draft form that feeds them.
//!
//! WHAT: a draft label carries its span, a message code and two data words. Type words among those data words
//! stay pending until `DiagnosticDraft::capture_types` rewrites them to display IDs. Compaction packs each label
//! into one word, with an optional row in `label_data` for the codes that carry data.
//! WHY: the legacy `DiagnosticLabelMessage` owns heap data such as substitution lists. The draft keeps only
//! fixed-size words, and substitution entries live in the draft's extra part until compaction moves them into
//! the store.

use super::facts::{
    FactCapacityError, IdRemap, pending_type_word, string_word, type_display_from_word,
};
use super::foreign::SiteRef;
use super::store::DiagnosticStore;
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::DiagnosticLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// The message of a compact label. Each variant of the legacy `DiagnosticLabelMessage` has one code, and a label
/// without a message has `NONE`. Codes are internal to one compiler build and carry no meaning outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LabelMessageCode(u8);

impl LabelMessageCode {
    pub(super) const NONE: Self = Self(0);
    pub(super) const PREVIOUS_DECLARATION: Self = Self(1);
    pub(super) const CONFLICTING_ACCESS: Self = Self(2);
    pub(super) const VALUE_MOVED_HERE: Self = Self(3);
    pub(super) const CONFIG_INPUT_ORIGIN: Self = Self(4);
    pub(super) const RENDERED_TEXT: Self = Self(5);
    pub(super) const GENERIC_INSTANTIATION_BODY_SITE: Self = Self(6);
    pub(super) const GENERIC_INSTANTIATION_DECLARATION_SITE: Self = Self(7);
    pub(super) const GENERIC_INSTANTIATION_SUBSTITUTIONS: Self = Self(8);
    pub(super) const GENERIC_INFERENCE_PREVIOUS_EVIDENCE: Self = Self(9);
    pub(super) const IMMUTABLE_BINDING_DECLARATION: Self = Self(10);
    pub(super) const TYPED_FAILURE_PRODUCER: Self = Self(11);
    pub(super) const IMPLICIT_FAILURE_PRODUCER: Self = Self(12);
    pub(super) const BUILTIN_FAILURE_CALL: Self = Self(13);
    pub(super) const BUILTIN_FAILURE_ORIGIN: Self = Self(14);
    pub(super) const BUILTIN_WRITE_BACK_ORIGIN: Self = Self(15);

    /// Whether the label keeps a row in `label_data`. Config origin, rendered text, substitutions, typed failure
    /// producers and write-back origins carry data; the rest never read the row.
    pub(super) const fn carries_data(self) -> bool {
        matches!(self.0, 4 | 5 | 8 | 11 | 15)
    }

    /// Whether every family accepts the label, whatever its row declares. The generic instantiation labels are
    /// appended to any diagnostic, and a message-less label carries no role of its own.
    pub(super) fn is_universal(self) -> bool {
        matches!(
            self,
            Self::NONE
                | Self::GENERIC_INSTANTIATION_BODY_SITE
                | Self::GENERIC_INSTANTIATION_DECLARATION_SITE
                | Self::GENERIC_INSTANTIATION_SUBSTITUTIONS
        )
    }

    /// The constant's name, as a diagnostic about a missing label role names it.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::NONE => "NONE",
            Self::PREVIOUS_DECLARATION => "PREVIOUS_DECLARATION",
            Self::CONFLICTING_ACCESS => "CONFLICTING_ACCESS",
            Self::VALUE_MOVED_HERE => "VALUE_MOVED_HERE",
            Self::CONFIG_INPUT_ORIGIN => "CONFIG_INPUT_ORIGIN",
            Self::RENDERED_TEXT => "RENDERED_TEXT",
            Self::GENERIC_INSTANTIATION_BODY_SITE => "GENERIC_INSTANTIATION_BODY_SITE",
            Self::GENERIC_INSTANTIATION_DECLARATION_SITE => {
                "GENERIC_INSTANTIATION_DECLARATION_SITE"
            }
            Self::GENERIC_INSTANTIATION_SUBSTITUTIONS => "GENERIC_INSTANTIATION_SUBSTITUTIONS",
            Self::GENERIC_INFERENCE_PREVIOUS_EVIDENCE => "GENERIC_INFERENCE_PREVIOUS_EVIDENCE",
            Self::IMMUTABLE_BINDING_DECLARATION => "IMMUTABLE_BINDING_DECLARATION",
            Self::TYPED_FAILURE_PRODUCER => "TYPED_FAILURE_PRODUCER",
            Self::IMPLICIT_FAILURE_PRODUCER => "IMPLICIT_FAILURE_PRODUCER",
            Self::BUILTIN_FAILURE_CALL => "BUILTIN_FAILURE_CALL",
            Self::BUILTIN_FAILURE_ORIGIN => "BUILTIN_FAILURE_ORIGIN",
            Self::BUILTIN_WRITE_BACK_ORIGIN => "BUILTIN_WRITE_BACK_ORIGIN",
            _ => unreachable!("label codes are the constants above"),
        }
    }
}

/// A label as a constructor builds it. `type_words` marks data words that hold a type word, while
/// `pending_types` marks which of those words still hold a raw `TypeId`.
#[derive(Clone, Copy, Debug)]
pub(super) struct DraftLabel {
    pub(super) span: Option<SourceSpan>,
    pub(super) message: LabelMessageCode,
    /// Bit `i` set means `data[i]` holds a type word.
    pub(super) type_words: u8,
    /// Bit `i` set means `data[i]` still holds a raw `TypeId` that needs a display ID.
    pub(super) pending_types: u8,
    pub(super) data: [u32; 2],
}

/// One entry of a generic substitution list. Entries live in the draft's extra part until compaction.
#[derive(Clone, Copy, Debug)]
pub(super) struct DraftSubstitution {
    pub(super) parameter: u32,
    pub(super) concrete: DraftType,
}

/// A type word inside a draft: a raw `TypeId` still to be captured, or a display word already captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DraftType {
    Pending(u32),
    Captured(u32),
}

/// A generic substitution as the store keeps it. `type_word` is a type display word.
#[derive(Clone, Copy, Debug)]
pub(super) struct GenericSubstitutionRecord {
    pub(super) parameter: u32,
    pub(super) type_word: u32,
}

const _: () = assert!(
    size_of::<GenericSubstitutionRecord>() == 8,
    "generic substitutions are locked at 8 bytes"
);

/// Encodes one legacy label. A label without a message encodes as `NONE` and keeps its span. Substitution entries
/// are appended to `substitutions`, the draft's arena.
///
/// The match is exhaustive on purpose: a new `DiagnosticLabelMessage` variant must choose its code here.
pub(super) fn encode_label(
    span: Option<SourceSpan>,
    message: Option<DiagnosticLabelMessage>,
    substitutions: &mut Vec<DraftSubstitution>,
) -> Result<DraftLabel, FactCapacityError> {
    let plain = |code: LabelMessageCode| DraftLabel {
        span,
        message: code,
        type_words: 0,
        pending_types: 0,
        data: [0, 0],
    };
    let Some(message) = message else {
        return Ok(plain(LabelMessageCode::NONE));
    };
    Ok(match message {
        DiagnosticLabelMessage::PreviousDeclaration => {
            plain(LabelMessageCode::PREVIOUS_DECLARATION)
        }
        DiagnosticLabelMessage::ConflictingAccess => plain(LabelMessageCode::CONFLICTING_ACCESS),
        DiagnosticLabelMessage::ValueMovedHere => plain(LabelMessageCode::VALUE_MOVED_HERE),
        DiagnosticLabelMessage::ConfigInputOrigin { input_name, origin } => DraftLabel {
            data: [string_word(input_name)?, origin_code(origin)],
            ..plain(LabelMessageCode::CONFIG_INPUT_ORIGIN)
        },
        DiagnosticLabelMessage::RenderedText(text) => DraftLabel {
            data: [string_word(text)?, 0],
            ..plain(LabelMessageCode::RENDERED_TEXT)
        },
        DiagnosticLabelMessage::GenericInstantiationBodySite => {
            plain(LabelMessageCode::GENERIC_INSTANTIATION_BODY_SITE)
        }
        DiagnosticLabelMessage::GenericInstantiationDeclarationSite => {
            plain(LabelMessageCode::GENERIC_INSTANTIATION_DECLARATION_SITE)
        }
        DiagnosticLabelMessage::GenericInstantiationSubstitutions {
            substitutions: entries,
        } => {
            let start = word_index(substitutions.len())?;
            let len = word_index(entries.len())?;
            for entry in entries {
                substitutions.push(DraftSubstitution {
                    parameter: string_word(entry.parameter_name)?,
                    concrete: DraftType::Pending(pending_type_word(entry.concrete_type_id)?),
                });
            }
            DraftLabel {
                data: [start, len],
                ..plain(LabelMessageCode::GENERIC_INSTANTIATION_SUBSTITUTIONS)
            }
        }
        DiagnosticLabelMessage::GenericInferencePreviousEvidence => {
            plain(LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE)
        }
        DiagnosticLabelMessage::ImmutableBindingDeclaration => {
            plain(LabelMessageCode::IMMUTABLE_BINDING_DECLARATION)
        }
        DiagnosticLabelMessage::TypedFailureProducer { error_type_id } => DraftLabel {
            type_words: 0b01,
            pending_types: 0b01,
            data: [pending_type_word(error_type_id)?, 0],
            ..plain(LabelMessageCode::TYPED_FAILURE_PRODUCER)
        },
        DiagnosticLabelMessage::ImplicitFailureProducer => {
            plain(LabelMessageCode::IMPLICIT_FAILURE_PRODUCER)
        }
        DiagnosticLabelMessage::BuiltinFailureCall => plain(LabelMessageCode::BUILTIN_FAILURE_CALL),
        DiagnosticLabelMessage::BuiltinFailureOrigin => {
            plain(LabelMessageCode::BUILTIN_FAILURE_ORIGIN)
        }
        DiagnosticLabelMessage::BuiltinWriteBackOrigin { target_type_id } => DraftLabel {
            type_words: 0b01,
            pending_types: 0b01,
            data: [pending_type_word(target_type_id)?, 0],
            ..plain(LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN)
        },
    })
}

/// Exhaustive so a new build-config origin must choose its code here.
fn origin_code(origin: BuildConfigValueOrigin) -> u32 {
    match origin {
        BuildConfigValueOrigin::ExplicitInput => 1,
        BuildConfigValueOrigin::BuilderGlobal => 2,
        BuildConfigValueOrigin::FixedProjectField => 3,
        BuildConfigValueOrigin::DeclarationDefault => 4,
    }
}

/// The origin that `origin_code` wrote for a stored word. A word `origin_code` never writes has no origin.
fn origin_from_code(word: u32) -> Option<BuildConfigValueOrigin> {
    match word {
        1 => Some(BuildConfigValueOrigin::ExplicitInput),
        2 => Some(BuildConfigValueOrigin::BuilderGlobal),
        3 => Some(BuildConfigValueOrigin::FixedProjectField),
        4 => Some(BuildConfigValueOrigin::DeclarationDefault),
        _ => None,
    }
}

/// A draft-local index or count as a word. Counts beyond `u32::MAX` cannot be stored.
fn word_index(value: usize) -> Result<u32, FactCapacityError> {
    u32::try_from(value).map_err(|_| FactCapacityError)
}

/// A secondary label as the store keeps it: a span plus one packed word.
///
/// Bits 24-31 hold the message code. Bits 0-23 hold the `label_data` row index, which is meaningful only when
/// the code carries data.
#[derive(Clone, Copy, Debug)]
pub(super) struct SecondaryDiagnosticLabel {
    pub(super) span: Option<SourceSpan>,
    pub(super) packed: u32,
}

const _: () = assert!(
    size_of::<SecondaryDiagnosticLabel>() == 12,
    "secondary labels are locked at 12 bytes"
);

const CODE_SHIFT: u32 = 24;
const DATA_INDEX_MASK: u32 = (1 << CODE_SHIFT) - 1;

impl SecondaryDiagnosticLabel {
    /// Packs a label. `data_index` is meaningful only for codes that carry data; compaction passes 0 otherwise.
    pub(super) fn pack(span: Option<SourceSpan>, code: LabelMessageCode, data_index: u32) -> Self {
        assert!(
            data_index <= DATA_INDEX_MASK,
            "label data rows fit in 24 bits; compaction checks the label_data limit first"
        );
        Self {
            span,
            packed: (u32::from(code.0) << CODE_SHIFT) | data_index,
        }
    }

    /// The span this label points at. `None` when the span belongs to a foreign site.
    pub(super) fn span(self) -> Option<SourceSpan> {
        self.span
    }

    /// The message code of the label.
    pub(super) fn message_code(self) -> LabelMessageCode {
        LabelMessageCode((self.packed >> CODE_SHIFT) as u8)
    }

    /// The `label_data` row of a code that carries data, or `None` for the others.
    pub(super) fn data_index(self) -> Option<usize> {
        self.message_code()
            .carries_data()
            .then_some((self.packed & DATA_INDEX_MASK) as usize)
    }

    /// The message of this label as a renderer reads it: the code's data, decoded from its `label_data` row or from
    /// the substitution arena for the substitution code.
    pub(super) fn message<'a>(self, store: &'a DiagnosticStore) -> DurableLabelMessage<'a> {
        let row = || {
            let index = self
                .data_index()
                .expect("label codes that carry data have a label_data row");
            store.label_data[index]
        };
        let type_word = |word: u32| {
            type_display_from_word(word).expect("captured label type words are display ids")
        };

        match self.message_code() {
            LabelMessageCode::NONE => DurableLabelMessage::None,
            LabelMessageCode::PREVIOUS_DECLARATION => DurableLabelMessage::PreviousDeclaration,
            LabelMessageCode::CONFLICTING_ACCESS => DurableLabelMessage::ConflictingAccess,
            LabelMessageCode::VALUE_MOVED_HERE => DurableLabelMessage::ValueMovedHere,
            LabelMessageCode::CONFIG_INPUT_ORIGIN => {
                let [input_name, origin] = row();
                DurableLabelMessage::ConfigInputOrigin {
                    input_name: StringId::from_index(input_name),
                    origin: origin_from_code(origin)
                        .expect("config origin rows hold a valid origin"),
                }
            }
            LabelMessageCode::RENDERED_TEXT => {
                let [text, _] = row();
                DurableLabelMessage::RenderedText(StringId::from_index(text))
            }
            LabelMessageCode::GENERIC_INSTANTIATION_BODY_SITE => {
                DurableLabelMessage::GenericInstantiationBodySite
            }
            LabelMessageCode::GENERIC_INSTANTIATION_DECLARATION_SITE => {
                DurableLabelMessage::GenericInstantiationDeclarationSite
            }
            LabelMessageCode::GENERIC_INSTANTIATION_SUBSTITUTIONS => {
                let [start, len] = row();
                let start = start as usize;
                DurableLabelMessage::GenericInstantiationSubstitutions(GenericSubstitutions {
                    records: &store.substitutions[start..start + len as usize],
                })
            }
            LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE => {
                DurableLabelMessage::GenericInferencePreviousEvidence
            }
            LabelMessageCode::IMMUTABLE_BINDING_DECLARATION => {
                DurableLabelMessage::ImmutableBindingDeclaration
            }
            LabelMessageCode::TYPED_FAILURE_PRODUCER => {
                let [error_type, _] = row();
                DurableLabelMessage::TypedFailureProducer {
                    error_type: type_word(error_type),
                }
            }
            LabelMessageCode::IMPLICIT_FAILURE_PRODUCER => {
                DurableLabelMessage::ImplicitFailureProducer
            }
            LabelMessageCode::BUILTIN_FAILURE_CALL => DurableLabelMessage::BuiltinFailureCall,
            LabelMessageCode::BUILTIN_FAILURE_ORIGIN => DurableLabelMessage::BuiltinFailureOrigin,
            LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN => {
                let [target_type, _] = row();
                DurableLabelMessage::BuiltinWriteBackOrigin {
                    target_type: type_word(target_type),
                }
            }
            _ => unreachable!("label codes are the constants in LabelMessageCode"),
        }
    }
}

/// A label's message as a renderer reads it: the code's data, decoded into typed values.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DurableLabelMessage<'a> {
    None,
    PreviousDeclaration,
    ConflictingAccess,
    ValueMovedHere,
    ConfigInputOrigin {
        input_name: StringId,
        origin: BuildConfigValueOrigin,
    },
    RenderedText(StringId),
    GenericInstantiationBodySite,
    GenericInstantiationDeclarationSite,
    GenericInstantiationSubstitutions(GenericSubstitutions<'a>),
    GenericInferencePreviousEvidence,
    ImmutableBindingDeclaration,
    TypedFailureProducer {
        error_type: TypeDisplayId,
    },
    ImplicitFailureProducer,
    BuiltinFailureCall,
    BuiltinFailureOrigin,
    BuiltinWriteBackOrigin {
        target_type: TypeDisplayId,
    },
}

/// The generic substitutions of one label, borrowed from the store's substitution arena.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GenericSubstitutions<'a> {
    records: &'a [GenericSubstitutionRecord],
}

impl<'a> GenericSubstitutions<'a> {
    /// The substitutions in the order the label names them, with each concrete type decoded.
    pub(crate) fn iter(self) -> impl Iterator<Item = GenericSubstitution> + 'a {
        self.records.iter().map(|record| GenericSubstitution {
            parameter: StringId::from_index(record.parameter),
            concrete: type_display_from_word(record.type_word)
                .expect("captured substitution type words are display ids"),
        })
    }
}

/// One generic substitution: the parameter's name and the concrete type it received.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GenericSubstitution {
    pub(crate) parameter: StringId,
    pub(crate) concrete: TypeDisplayId,
}

/// A secondary label as a renderer reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DurableLabel<'a> {
    /// Whether the label stores a source span. A label without one marks no location.
    pub(crate) spanned: bool,
    /// Where the label points: foreign when a foreign-site row names it, otherwise local.
    pub(crate) site: SiteRef<'a>,
    pub(crate) message: DurableLabelMessage<'a>,
}

impl DraftLabel {
    /// Rewrites the string word in `data[0]` of a config-origin or rendered-text label.
    /// Type words stay pending, and substitution indices are not IDs.
    pub(super) fn remap_ids(&mut self, remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        if self.message == LabelMessageCode::CONFIG_INPUT_ORIGIN
            || self.message == LabelMessageCode::RENDERED_TEXT
        {
            remap.rewrite_string(&mut self.data[0])?;
        }
        Ok(())
    }
}

impl DraftSubstitution {
    /// Rewrites the parameter name. The concrete type is a type word and is left as it is.
    pub(super) fn remap_ids(&mut self, remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        remap.rewrite_string(&mut self.parameter)
    }
}
