//! Fixed-width fact codecs for compact diagnostic records.
//!
//! WHAT: each fact occupies one or more 32-bit words in a record's fact area. A codec pairs the value a
//! producer supplies (`Draft`) with the value a reader gets back (`Durable`).
//! WHY: records stay 32 bytes. Each codec owns its sentinels and bit layout, so a reader only decodes words
//! that the same codec wrote, and every family row checks its layout against the words its constructor writes.

use super::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::DiagnosticPlace;
use crate::compiler_frontend::compiler_messages::DiagnosticToken;
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use std::cell::Cell;
use std::marker::PhantomData;

/// Words in a record's fact area.
pub(super) const FACT_WORDS: usize = 4;

/// A value does not fit the words its codec writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FactCapacityError;

/// Encodes one fact type into record words and decodes it back.
pub(crate) trait FactCodec {
    /// The value a producer supplies.
    type Draft;
    /// The value a reader gets back.
    type Durable;
    /// Words the fact occupies.
    const WORDS: usize;
    /// Whether the words hold raw type ids that compaction rewrites into type display words.
    const PENDING: bool = false;

    fn encode(draft: &Self::Draft, words: &mut [u32]) -> Result<(), FactCapacityError>;
    fn decode(words: &[u32]) -> Option<Self::Durable>;
}

/// A unit enum stored as one word. Codes are small non-zero integers chosen by the enum's `unit_code!` table.
pub(super) trait UnitCode: Sized + Copy {
    fn code(self) -> u32;
    fn from_code(code: u32) -> Option<Self>;
}

/// Declares a unit enum's word codes, or attaches them to an existing enum.
///
/// `enum Name { Variant = code, .. }` declares the enum and its codes. `impl Name { Variant = code, .. }`
/// attaches codes to an enum declared elsewhere. Both forms must list every variant.
macro_rules! unit_code {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
        $($variant:ident = $code:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        $vis enum $name { $($variant),+ }

        $crate::compiler_frontend::compiler_messages::diagnostics::facts::unit_code! {
            impl $name { $($variant = $code),+ }
        }
    };
    (impl $name:ident { $($variant:ident = $code:literal),+ $(,)? }) => {
        impl $crate::compiler_frontend::compiler_messages::diagnostics::facts::UnitCode for $name {
            fn code(self) -> u32 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            fn from_code(code: u32) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}
pub(super) use unit_code;

/// Places pack a two-bit tag above a 30-bit value. Tag 0 with value 0 is the unknown place.
const PLACE_TAG_SHIFT: u32 = 30;
const PLACE_VALUE_MASK: u32 = (1 << PLACE_TAG_SHIFT) - 1;
const PLACE_LOCAL_TAG: u32 = 1;
const PLACE_PATH_TAG: u32 = 2;

/// Encodes a string id. `u32::MAX` is the absent sentinel of `OptStringFact`, so it is never a valid id.
pub(super) fn string_word(id: StringId) -> Result<u32, FactCapacityError> {
    let index = id.index();
    if index == u32::MAX {
        return Err(FactCapacityError);
    }
    Ok(index)
}

/// Encodes a type display as its one-based index, so zero stays free for "no type".
pub(super) fn type_display_word(id: TypeDisplayId) -> Result<u32, FactCapacityError> {
    u32::try_from(id.index() + 1).map_err(|_| FactCapacityError)
}

/// Decodes a type display word. Zero means no type.
pub(super) fn type_display_from_word(word: u32) -> Option<TypeDisplayId> {
    let index = word.checked_sub(1)?;
    TypeDisplayId::from_index(index as usize).ok()
}

/// The pending word of an absent type. `capture_types` rewrites it to `NO_TYPE_WORD` without calling `rewrite`.
pub(super) const ABSENT_TYPE_WORD: u32 = u32::MAX;
/// The captured word for "no type". Display words are one-based, so zero stays free.
pub(super) const NO_TYPE_WORD: u32 = 0;

/// Encodes a raw `TypeId` as a pending word. `ABSENT_TYPE_WORD` marks absence, so that one id is refused.
pub(super) fn pending_type_word(id: TypeId) -> Result<u32, FactCapacityError> {
    if id.0 == ABSENT_TYPE_WORD {
        return Err(FactCapacityError);
    }
    Ok(id.0)
}

fn place_word(tag: u32, value: usize) -> Result<u32, FactCapacityError> {
    let value = u32::try_from(value).map_err(|_| FactCapacityError)?;
    if value > PLACE_VALUE_MASK {
        return Err(FactCapacityError);
    }
    Ok((tag << PLACE_TAG_SHIFT) | value)
}

/// A token that is always present.
pub(crate) struct TokenFact;

impl FactCodec for TokenFact {
    type Draft = DiagnosticToken;
    type Durable = DiagnosticToken;
    const WORDS: usize = 2;

    fn encode(draft: &DiagnosticToken, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words.copy_from_slice(&draft.raw_words());
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<DiagnosticToken> {
        DiagnosticToken::from_raw_words([words[0], words[1]])
    }
}

/// Two words that encode `Option::None` as `[u32::MAX, 0]`. No token encodes to that pair.
const ABSENT_TOKEN_WORDS: [u32; 2] = [u32::MAX, 0];

/// A token that may be absent.
pub(crate) struct OptTokenFact;

impl FactCodec for OptTokenFact {
    type Draft = Option<DiagnosticToken>;
    type Durable = Option<DiagnosticToken>;
    const WORDS: usize = 2;

    fn encode(draft: &Option<DiagnosticToken>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        match draft {
            Some(token) => words.copy_from_slice(&token.raw_words()),
            None => words.copy_from_slice(&ABSENT_TOKEN_WORDS),
        }
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<DiagnosticToken>> {
        if words[0] == ABSENT_TOKEN_WORDS[0] && words[1] == ABSENT_TOKEN_WORDS[1] {
            return Some(None);
        }
        DiagnosticToken::from_raw_words([words[0], words[1]]).map(Some)
    }
}

/// A string id that is always present.
pub(crate) struct StringFact;

impl FactCodec for StringFact {
    type Draft = StringId;
    type Durable = StringId;
    const WORDS: usize = 1;

    fn encode(draft: &StringId, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = string_word(*draft)?;
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<StringId> {
        (words[0] != u32::MAX).then(|| StringId::from_index(words[0]))
    }
}

/// A string id that may be absent. `u32::MAX` encodes absence.
pub(crate) struct OptStringFact;

impl FactCodec for OptStringFact {
    type Draft = Option<StringId>;
    type Durable = Option<StringId>;
    const WORDS: usize = 1;

    fn encode(draft: &Option<StringId>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            Some(id) => string_word(*id)?,
            None => u32::MAX,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<StringId>> {
        Some((words[0] != u32::MAX).then(|| StringId::from_index(words[0])))
    }
}

/// An interned path. Zero-based, so `u32::MAX` never decodes.
pub(crate) struct PathFact;

impl FactCodec for PathFact {
    type Draft = PathId;
    type Durable = PathId;
    const WORDS: usize = 1;

    fn encode(draft: &PathId, words: &mut [u32]) -> Result<(), FactCapacityError> {
        let index = u32::try_from(draft.index()).map_err(|_| FactCapacityError)?;
        if index == u32::MAX {
            return Err(FactCapacityError);
        }
        words[0] = index;
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<PathId> {
        PathId::try_from_index(words[0] as usize)
    }
}

/// A count stored as one word.
pub(crate) struct CountFact;

impl FactCodec for CountFact {
    type Draft = usize;
    type Durable = usize;
    const WORDS: usize = 1;

    fn encode(draft: &usize, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = u32::try_from(*draft).map_err(|_| FactCapacityError)?;
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<usize> {
        Some(words[0] as usize)
    }
}

/// A character as one word. Decoding rejects words that are not Unicode scalar values.
pub(crate) struct CharFact;

impl FactCodec for CharFact {
    type Draft = char;
    type Durable = char;
    const WORDS: usize = 1;

    fn encode(draft: &char, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = u32::from(*draft);
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<char> {
        char::from_u32(words[0])
    }
}

/// An optional character as one word. `u32::MAX` encodes absence and is not a Unicode scalar.
pub(crate) struct OptCharFact;

impl FactCodec for OptCharFact {
    type Draft = Option<char>;
    type Durable = Option<char>;
    const WORDS: usize = 1;

    fn encode(draft: &Option<char>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = draft.map_or(u32::MAX, u32::from);
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<char>> {
        if words[0] == u32::MAX {
            Some(None)
        } else {
            char::from_u32(words[0]).map(Some)
        }
    }
}

/// An optional count. `u32::MAX` marks absence, so a present count must stay below it.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.4 family rows use it"))]
pub(crate) struct OptCountFact;

impl FactCodec for OptCountFact {
    type Draft = Option<usize>;
    type Durable = Option<usize>;
    const WORDS: usize = 1;

    fn encode(draft: &Option<usize>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            Some(count) => {
                let word = u32::try_from(*count).map_err(|_| FactCapacityError)?;
                if word == u32::MAX {
                    return Err(FactCapacityError);
                }
                word
            }
            None => u32::MAX,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<usize>> {
        match words[0] {
            u32::MAX => Some(None),
            word => Some(Some(word as usize)),
        }
    }
}

/// A type. The draft holds a raw `TypeId`, which compaction rewrites into a type display word.
pub(crate) struct TypeFact;

impl FactCodec for TypeFact {
    type Draft = TypeId;
    type Durable = TypeDisplayId;
    const WORDS: usize = 1;
    const PENDING: bool = true;

    fn encode(draft: &TypeId, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = pending_type_word(*draft)?;
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<TypeDisplayId> {
        type_display_from_word(words[0])
    }
}

/// An optional type. Absence is `ABSENT_TYPE_WORD`, which `capture_types` captures as `NO_TYPE_WORD`.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.4 family rows use it"))]
pub(crate) struct OptTypeFact;

impl FactCodec for OptTypeFact {
    type Draft = Option<TypeId>;
    type Durable = Option<TypeDisplayId>;
    const WORDS: usize = 1;
    const PENDING: bool = true;

    fn encode(draft: &Option<TypeId>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            Some(id) => pending_type_word(*id)?,
            None => ABSENT_TYPE_WORD,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<TypeDisplayId>> {
        match words[0] {
            NO_TYPE_WORD => Some(None),
            word => type_display_from_word(word).map(Some),
        }
    }
}

/// A place as the store keeps it. A rendered-text place is stored as a local name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredPlace {
    Unknown,
    Local(StringId),
    Path(PathId),
}

impl StoredPlace {
    /// The draft place this stored place renders as. A rendered-text place was stored as a local name, so it reads
    /// back as `Local`; the adapter rejects rendered-text places before compaction.
    pub(crate) fn diagnostic_place(self) -> DiagnosticPlace {
        match self {
            Self::Unknown => DiagnosticPlace::Unknown,
            Self::Local(name) => DiagnosticPlace::Local(name),
            Self::Path(path) => DiagnosticPlace::Path(path),
        }
    }
}

/// A place: unknown, a local name, or a path.
pub(crate) struct PlaceFact;

impl FactCodec for PlaceFact {
    type Draft = DiagnosticPlace;
    type Durable = StoredPlace;
    const WORDS: usize = 1;

    fn encode(draft: &DiagnosticPlace, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            DiagnosticPlace::Unknown => 0,
            DiagnosticPlace::Local(name) | DiagnosticPlace::RenderedText(name) => {
                place_word(PLACE_LOCAL_TAG, name.index() as usize)?
            }
            DiagnosticPlace::Path(path) => place_word(PLACE_PATH_TAG, path.index())?,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<StoredPlace> {
        let tag = words[0] >> PLACE_TAG_SHIFT;
        let value = words[0] & PLACE_VALUE_MASK;
        match (tag, value) {
            (0, 0) => Some(StoredPlace::Unknown),
            (0, _) => None,
            (PLACE_LOCAL_TAG, _) => Some(StoredPlace::Local(StringId::from_index(value))),
            (PLACE_PATH_TAG, _) => PathId::try_from_index(value as usize).map(StoredPlace::Path),
            _ => None,
        }
    }
}

/// A unit enum stored as its one-word code.
pub(crate) struct UnitFact<T>(PhantomData<T>);

impl<T: UnitCode> FactCodec for UnitFact<T> {
    type Draft = T;
    type Durable = T;
    const WORDS: usize = 1;

    fn encode(draft: &T, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = draft.code();
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<T> {
        T::from_code(words[0])
    }
}

/// Writes facts into a record's fact area in declaration order.
pub(super) struct FactCursor {
    words: [u32; FACT_WORDS],
    used: usize,
    pending: u8,
}

impl FactCursor {
    pub(super) fn new() -> Self {
        Self {
            words: [0; FACT_WORDS],
            used: 0,
            pending: 0,
        }
    }

    pub(super) fn push<C: FactCodec>(&mut self, draft: &C::Draft) -> Result<(), FactCapacityError> {
        let end = self.used + C::WORDS;
        assert!(
            end <= FACT_WORDS,
            "family fact layout exceeds the record's fact area"
        );
        C::encode(draft, &mut self.words[self.used..end])?;
        if C::PENDING {
            for word in self.used..end {
                self.pending |= 1 << word;
            }
        }
        self.used = end;
        Ok(())
    }

    /// Checks the written facts against the family row and returns the words and pending mask.
    pub(super) fn finish_for(self, row: &FamilyRow) -> ([u32; FACT_WORDS], u8) {
        assert_eq!(
            self.used,
            row.word_count(),
            "a family constructor writes every fact in its family layout"
        );
        assert_eq!(
            self.pending,
            row.pending_mask(),
            "pending type words match the family layout"
        );
        (self.words, self.pending)
    }
}

/// Reads facts from a record's fact area in declaration order.
pub(super) struct FactReader<'a> {
    words: &'a [u32; FACT_WORDS],
    used: Cell<usize>,
}

impl<'a> FactReader<'a> {
    pub(super) fn new(words: &'a [u32; FACT_WORDS]) -> Self {
        Self {
            words,
            used: Cell::new(0),
        }
    }

    pub(super) fn read<C: FactCodec>(&self) -> C::Durable {
        let start = self.used.get();
        let end = start + C::WORDS;
        self.used.set(end);
        C::decode(&self.words[start..end])
            .expect("fact words are written only by the codec that reads them")
    }

    /// Checks that no word was left unread. Unread words are always zero, because only codecs write words.
    pub(super) fn finish(self) {
        let used = self.used.get();
        assert!(
            self.words[used..].iter().all(|word| *word == 0),
            "unread fact words stay zero"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{FactCodec, OptCharFact};

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
}
