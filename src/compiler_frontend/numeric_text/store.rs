//! Source-owned cold storage for numeric literal lexical records.
//!
//! The lexer may still expose a `NumericLiteralToken` through the transient `TokenKind` adapter
//! while parser consumers migrate. `NumericLiteralStore` is the durable owner at the source
//! boundary and uses checked one-based `u32` handles so `0` remains an absent marker.

use super::token::NumericLiteralToken;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
#[cfg(test)]
use rustc_hash::FxHashMap;

/// Dense handle into one [`NumericLiteralStore`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NumericLiteralId(u32);

impl NumericLiteralId {
    /// Absent marker. It is never a valid store row.
    pub const NONE: Self = Self(0);

    pub const fn try_from_raw(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn try_from_index(index: usize) -> Option<Self> {
        if index >= u32::MAX as usize {
            return None;
        }
        Some(Self((index as u32) + 1))
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
    #[cfg(test)]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
    pub const fn index(self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some((self.0 - 1) as usize)
        }
    }
}

/// Checked failures from numeric cold-store construction and lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericLiteralStoreError {
    Capacity(NumericLiteralCapacityError),
    Absent,
    OutOfRange {
        raw: u32,
        len: usize,
    },
    ForeignSource {
        expected: SourceId,
        actual: SourceId,
    },
    Frozen,
}

/// Capacity lane for one compact numeric side store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericLiteralCapacityError {
    StoreFull,
}

impl From<NumericLiteralCapacityError> for NumericLiteralStoreError {
    fn from(error: NumericLiteralCapacityError) -> Self {
        Self::Capacity(error)
    }
}

/// Dense source-owned numeric lexical records.
#[derive(Clone, Debug, Default)]
pub struct NumericLiteralStore {
    literals: Vec<NumericLiteralToken>,
    owner_source: Option<SourceId>,
    frozen: bool,
}

impl NumericLiteralStore {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_source(source: SourceId) -> Self {
        Self {
            owner_source: Some(source),
            ..Self::default()
        }
    }

    pub fn owner_source(&self) -> Option<SourceId> {
        self.owner_source
    }

    /// Rebind the single source owner without changing any local rows.
    ///
    /// Row handles are source-local indices, so only the owning `SourceId` is
    /// restamped. Mirrors `PathSyntaxTable::rebind_source_identity`.
    pub fn rebind_source_identity(&mut self, source: SourceId) {
        self.owner_source = Some(source);
    }

    pub fn len(&self) -> usize {
        self.literals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.literals.is_empty()
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }
    #[cfg(test)]
    pub fn records(&self) -> &[NumericLiteralToken] {
        &self.literals
    }

    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = (NumericLiteralId, &NumericLiteralToken)> + '_ {
        self.literals.iter().enumerate().map(|(index, literal)| {
            (
                NumericLiteralId::try_from_index(index)
                    .expect("stored numeric literal index must fit its handle"),
                literal,
            )
        })
    }

    #[cfg(test)]
    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (NumericLiteralId, &mut NumericLiteralToken)> + '_ {
        assert!(
            !self.frozen,
            "numeric literal iteration cannot mutate a frozen store"
        );
        self.literals
            .iter_mut()
            .enumerate()
            .map(|(index, literal)| {
                (
                    NumericLiteralId::try_from_index(index)
                        .expect("stored numeric literal index must fit its handle"),
                    literal,
                )
            })
    }

    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Checked append with a stable one-based handle.
    pub fn try_push(
        &mut self,
        literal: NumericLiteralToken,
    ) -> Result<NumericLiteralId, NumericLiteralStoreError> {
        if self.frozen {
            return Err(NumericLiteralStoreError::Frozen);
        }
        let id = NumericLiteralId::try_from_index(self.literals.len())
            .ok_or(NumericLiteralCapacityError::StoreFull)?;
        self.literals.push(literal);
        Ok(id)
    }

    /// Compatibility constructor for fixtures that are not exercising the capacity lane.
    #[cfg(test)]
    pub fn push(&mut self, literal: NumericLiteralToken) -> NumericLiteralId {
        self.try_push(literal)
            .expect("numeric literal insertion must be checked at its owner boundary")
    }

    pub fn try_push_for_source(
        &mut self,
        source: SourceId,
        literal: NumericLiteralToken,
    ) -> Result<NumericLiteralId, NumericLiteralStoreError> {
        if let Some(expected) = self.owner_source
            && expected != source
        {
            return Err(NumericLiteralStoreError::ForeignSource {
                expected,
                actual: source,
            });
        }
        if self.owner_source.is_none() {
            self.owner_source = Some(source);
        }
        self.try_push(literal)
    }

    pub fn try_get(
        &self,
        id: NumericLiteralId,
    ) -> Result<&NumericLiteralToken, NumericLiteralStoreError> {
        let Some(index) = id.index() else {
            return Err(NumericLiteralStoreError::Absent);
        };
        self.literals
            .get(index)
            .ok_or(NumericLiteralStoreError::OutOfRange {
                raw: id.raw(),
                len: self.literals.len(),
            })
    }

    pub fn try_get_for_source(
        &self,
        id: NumericLiteralId,
        source: SourceId,
    ) -> Result<&NumericLiteralToken, NumericLiteralStoreError> {
        if let Some(expected) = self.owner_source
            && expected != source
        {
            return Err(NumericLiteralStoreError::ForeignSource {
                expected,
                actual: source,
            });
        }
        self.try_get(id)
    }

    /// Infallible construction-time string remap. Panics on a frozen store.
    ///
    /// Post-publication remap must go through the owning `FileTokens` boundary (which
    /// rejects it) or the materialisation clone-remap-freeze path (which remaps an
    /// explicit unfrozen clone via [`Self::clone_for_materialisation`]).
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.try_remap_string_ids(&mut |id| {
            Ok::<StringId, std::convert::Infallible>(remap.get(id))
        })
        .expect("numeric literal StringId remapping is infallible");
    }

    /// Fallible construction-time string remap. Panics on a frozen store.
    ///
    /// The generic error lane carries only the caller's mapping failure; a frozen-store
    /// mutation is a lifecycle violation, so it panics like the owning `FileTokens`
    /// post-freeze remap rather than surfacing as a caller error.
    pub fn try_remap_string_ids<E>(
        &mut self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<(), E> {
        if self.frozen {
            panic!("numeric literal remapping was requested after the source publication freeze");
        }
        for literal in &mut self.literals {
            literal.try_remap_string_ids(map)?;
        }
        Ok(())
    }

    /// Copy only referenced records in first-reference order and return the old-to-new map.
    #[cfg(test)]
    pub fn compact_subset<I>(
        &self,
        ids: I,
    ) -> Result<(Self, FxHashMap<NumericLiteralId, NumericLiteralId>), NumericLiteralStoreError>
    where
        I: IntoIterator<Item = NumericLiteralId>,
    {
        let mut subset = Self {
            literals: Vec::new(),
            owner_source: self.owner_source,
            frozen: false,
        };
        let mut old_to_new = FxHashMap::default();
        for old_id in ids {
            if old_id.is_none() {
                return Err(NumericLiteralStoreError::Absent);
            }
            if old_to_new.contains_key(&old_id) {
                continue;
            }
            let literal = self.try_get(old_id)?.clone();
            let new_id = subset.try_push(literal)?;
            old_to_new.insert(old_id, new_id);
        }
        Ok((subset, old_to_new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    #[test]
    fn ids_and_lookup_reject_reserved_and_foreign_handles() {
        assert_eq!(NumericLiteralId::try_from_raw(0), None);
        assert_eq!(NumericLiteralId::try_from_index(u32::MAX as usize), None);

        let owner = SourceId::COMPILATION_ROOT;
        let foreign = SourceId::from_index(7);
        let mut strings = StringTable::new();
        let mut store = NumericLiteralStore::with_source(owner);
        let id = store.push(NumericLiteralToken::test_new("7", &mut strings));
        assert!(matches!(
            store.try_get(NumericLiteralId::NONE),
            Err(NumericLiteralStoreError::Absent)
        ));
        assert!(matches!(
            store.try_get(NumericLiteralId::try_from_raw(2).unwrap()),
            Err(NumericLiteralStoreError::OutOfRange { .. })
        ));
        assert!(matches!(
            store.try_get_for_source(id, foreign),
            Err(NumericLiteralStoreError::ForeignSource { .. })
        ));
    }

    #[test]
    fn subset_copy_is_deterministic_and_deduplicates_rows() {
        let mut strings = StringTable::new();
        let mut store = NumericLiteralStore::new();
        let first = store.push(NumericLiteralToken::test_new("1", &mut strings));
        let second = store.push(NumericLiteralToken::test_new("2", &mut strings));
        let (subset, map) = store
            .compact_subset([second, second, first])
            .expect("subset should be representable");
        let ids = subset.iter().map(|(id, _)| id).collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                NumericLiteralId::try_from_raw(1).unwrap(),
                NumericLiteralId::try_from_raw(2).unwrap()
            ]
        );
        assert_eq!(map[&second], ids[0]);
        assert_eq!(map[&first], ids[1]);
    }

    #[test]
    fn freeze_and_remap_are_checked_and_in_place() {
        let mut local = StringTable::new();
        let mut global = StringTable::new();
        let mut store = NumericLiteralStore::new();
        let id = store.push(NumericLiteralToken::test_new("1_000", &mut local));
        global.intern("preexisting");
        let remap = global.merge_from(&local);
        store.remap_string_ids(&remap);
        let literal = store.try_get(id).unwrap();
        assert_eq!(global.resolve(literal.source_text), "1_000");
        assert_eq!(global.resolve(literal.normalized_text), "1000");
        store.freeze();
        assert!(store.is_frozen());
        assert!(matches!(
            store.try_push(NumericLiteralToken::test_new("3", &mut global)),
            Err(NumericLiteralStoreError::Frozen)
        ));
    }

    #[test]
    fn frozen_store_rejects_source_checked_push_and_remaps_both_texts() {
        let mut local = StringTable::new();
        let mut global = StringTable::new();
        let owner = SourceId::COMPILATION_ROOT;
        let mut store = NumericLiteralStore::with_source(owner);
        let id = store
            .try_push_for_source(owner, NumericLiteralToken::test_new("1_2", &mut local))
            .expect("owner push should succeed");
        assert_eq!(store.owner_source(), Some(owner));
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
        assert!(!store.is_frozen());
        assert_eq!(store.records().len(), 1);
        assert_eq!(store.iter().count(), 1);
        assert_eq!(store.iter_mut().count(), 1);
        let foreign = SourceId::from_index(9);
        assert!(matches!(
            store.try_push_for_source(foreign, NumericLiteralToken::test_new("3", &mut local)),
            Err(NumericLiteralStoreError::ForeignSource { .. })
        ));
        global.intern("preexisting");
        let remap = global.merge_from(&local);
        store.remap_string_ids(&remap);
        let literal = store.try_get(id).expect("retained row");
        assert_eq!(global.resolve(literal.source_text), "1_2");
        assert_eq!(global.resolve(literal.normalized_text), "12");
        store.freeze();
        assert!(store.is_frozen());
        assert!(matches!(
            store.try_push_for_source(owner, NumericLiteralToken::test_new("4", &mut global)),
            Err(NumericLiteralStoreError::Frozen)
        ));
        let frozen_remap = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.remap_string_ids(&remap);
        }));
        assert!(
            frozen_remap.is_err(),
            "frozen store remap must not silently mutate"
        );
        let frozen_iter = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.iter_mut().count();
        }));
        assert!(
            frozen_iter.is_err(),
            "frozen store row iteration must not expose mutation"
        );
    }
    #[test]
    fn subset_rejects_absent_handles_and_preserves_first_reference_order() {
        let mut strings = StringTable::new();
        let mut store = NumericLiteralStore::new();
        let first = store.push(NumericLiteralToken::test_new("1", &mut strings));
        let second = store.push(NumericLiteralToken::test_new("2", &mut strings));
        assert!(matches!(
            store.compact_subset([NumericLiteralId::NONE]),
            Err(NumericLiteralStoreError::Absent)
        ));
        assert!(matches!(
            store.compact_subset([NumericLiteralId::try_from_raw(99).unwrap()]),
            Err(NumericLiteralStoreError::OutOfRange { .. })
        ));
        let (subset, map) = store
            .compact_subset([second, first, second])
            .expect("subset should be representable");
        assert!(!subset.is_frozen());
        let copied = subset.iter().map(|(id, _)| id).collect::<Vec<_>>();
        assert_eq!(
            copied,
            [
                NumericLiteralId::try_from_raw(1).unwrap(),
                NumericLiteralId::try_from_raw(2).unwrap()
            ]
        );
        assert_eq!(map[&second], copied[0]);
        assert_eq!(map[&first], copied[1]);
        assert_eq!(subset.len(), 2);
    }
}
