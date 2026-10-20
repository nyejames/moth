//! Secondary labels in the compact store, and the draft form that feeds them.
//!
//! WHAT: a draft label carries its span, a message code and two data words. Type words among those data words
//! stay pending until `DiagnosticDraft::capture_types` rewrites them to display IDs. Compaction packs each label
//! into one word, with an optional row in `label_data` for the codes that carry data.
//! WHY: the legacy `DiagnosticLabelMessage` owns heap data such as substitution lists. The draft keeps only
//! fixed-size words, and substitution entries live in the draft's extra part until compaction moves them into
//! the store.

use super::facts::{FactCapacityError, string_word};
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::DiagnosticLabelMessage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;

/// The message of a compact label. Each variant of the legacy `DiagnosticLabelMessage` has one code. Codes are
/// internal to one compiler build and carry no meaning outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LabelMessageCode(u8);

impl LabelMessageCode {
    pub(crate) const PREVIOUS_DECLARATION: Self = Self(1);
    pub(crate) const CONFLICTING_ACCESS: Self = Self(2);
    pub(crate) const VALUE_MOVED_HERE: Self = Self(3);
    pub(crate) const CONFIG_INPUT_ORIGIN: Self = Self(4);
    pub(crate) const RENDERED_TEXT: Self = Self(5);
    pub(crate) const GENERIC_INSTANTIATION_BODY_SITE: Self = Self(6);
    pub(crate) const GENERIC_INSTANTIATION_DECLARATION_SITE: Self = Self(7);
    pub(crate) const GENERIC_INSTANTIATION_SUBSTITUTIONS: Self = Self(8);
    pub(crate) const GENERIC_INFERENCE_PREVIOUS_EVIDENCE: Self = Self(9);
    pub(crate) const IMMUTABLE_BINDING_DECLARATION: Self = Self(10);
    pub(crate) const TYPED_FAILURE_PRODUCER: Self = Self(11);
    pub(crate) const IMPLICIT_FAILURE_PRODUCER: Self = Self(12);
    pub(crate) const BUILTIN_FAILURE_CALL: Self = Self(13);
    pub(crate) const BUILTIN_FAILURE_ORIGIN: Self = Self(14);
    pub(crate) const BUILTIN_WRITE_BACK_ORIGIN: Self = Self(15);

    /// Whether the label keeps a row in `label_data`. Config origin, rendered text, substitutions, typed failure
    /// producers and write-back origins carry data; the rest never read the row.
    pub(super) const fn carries_data(self) -> bool {
        matches!(self.0, 4 | 5 | 8 | 11 | 15)
    }
}

/// A label as a constructor builds it. `pending_types` marks the data words that hold a raw `TypeId` until
/// `capture_types` rewrites them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DraftLabel {
    pub(super) span: Option<SourceSpan>,
    pub(super) message: LabelMessageCode,
    /// Bit `i` set means `data[i]` holds a `TypeId` word that still needs a display ID.
    pub(super) pending_types: u8,
    pub(super) data: [u32; 2],
}

/// One entry of a generic substitution list. Entries live in the draft's extra part until compaction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DraftSubstitution {
    pub(super) parameter: u32,
    pub(super) concrete: DraftType,
}

/// A type word inside a draft: a raw `TypeId` still to be captured, or a display word already captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DraftType {
    Pending(TypeId),
    Captured(u32),
}

/// A generic substitution as the store keeps it. `type_word` is a type display word.
#[allow(dead_code, reason = "P4.3 render reads generic substitutions")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct GenericSubstitutionRecord {
    pub(super) parameter: u32,
    pub(super) type_word: u32,
}

const _: () = assert!(
    size_of::<GenericSubstitutionRecord>() == 8,
    "generic substitutions are locked at 8 bytes"
);

/// Encodes one legacy label. Substitution entries are appended to `substitutions`, the draft's arena.
///
/// The match is exhaustive on purpose: a new `DiagnosticLabelMessage` variant must choose its code here.
pub(super) fn encode_label(
    span: Option<SourceSpan>,
    message: DiagnosticLabelMessage,
    substitutions: &mut Vec<DraftSubstitution>,
) -> Result<DraftLabel, FactCapacityError> {
    let plain = |code: LabelMessageCode| DraftLabel {
        span,
        message: code,
        pending_types: 0,
        data: [0, 0],
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
                    concrete: DraftType::Pending(entry.concrete_type_id),
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
            pending_types: 0b01,
            data: [error_type_id.0, 0],
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
            pending_types: 0b01,
            data: [target_type_id.0, 0],
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

/// A draft-local index or count as a word. Counts beyond `u32::MAX` cannot be stored.
fn word_index(value: usize) -> Result<u32, FactCapacityError> {
    u32::try_from(value).map_err(|_| FactCapacityError)
}

/// A secondary label as the store keeps it: a span plus one packed word.
///
/// Bits 24-31 hold the message code. Bits 0-23 hold the `label_data` row index, which is meaningful only when
/// the code carries data.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SecondaryDiagnosticLabel {
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
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.3 render reads secondary labels")
    )]
    pub(crate) fn span(self) -> Option<SourceSpan> {
        self.span
    }

    /// The message code of the label.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.3 render reads secondary labels")
    )]
    pub(crate) fn message_code(self) -> LabelMessageCode {
        LabelMessageCode((self.packed >> CODE_SHIFT) as u8)
    }

    /// The `label_data` row of a code that carries data, or `None` for the others.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.3 render reads secondary labels")
    )]
    pub(crate) fn data_index(self) -> Option<usize> {
        self.message_code()
            .carries_data()
            .then_some((self.packed & DATA_INDEX_MASK) as usize)
    }
}
