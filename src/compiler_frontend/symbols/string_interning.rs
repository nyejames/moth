//! String interning for the compiler frontend.
//!
//! WHAT: deduplicates strings into a centralized table and hands out compact `StringId` handles,
//!       making equality comparisons O(1) and reducing memory pressure from repeated identifiers.
//! WHY: the compiler frontend deals with thousands of repeated identifiers, paths, and keywords;
//!      interning is one of the cheapest ways to lower allocation overhead and speed up lookups.

use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::projects::settings::MINIMUM_STRING_TABLE_CAPACITY;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// A unique identifier for an interned string, represented as a u32 for memory efficiency.
/// This provides type safety to prevent mixing string IDs with other integer values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringId(u32);

impl StringId {
    /// Build an ID from a numeric index.
    ///
    /// WHAT: used only by owners that manage their own string space, such as the frozen token
    ///       buffer whose pooled strings are indexed contiguously.
    pub(crate) fn from_index(index: u32) -> Self {
        Self(index)
    }

    /// The numeric index of this ID in its owning string space.
    pub(crate) fn index(self) -> u32 {
        self.0
    }
}

/// Display implementation that shows the underlying ID value.
impl std::fmt::Display for StringId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StringId({})", self.0)
    }
}

/// Mapping from StringIds in one table to StringIds in another after a merge.
#[derive(Debug, Clone)]
pub struct StringIdRemap {
    /// IDs below this length are known to be identical in source and target tables.
    identity_prefix_len: usize,

    /// Remapped IDs for the source table suffix after `identity_prefix_len`.
    mapped_suffix: Vec<StringId>,

    /// Cached identity result for the full remap.
    ///
    /// WHY: file preparation often creates identity delta remaps, and detailed-timer builds query
    /// this property from counters and payload remap fast paths. Keeping it with the remap avoids
    /// repeatedly scanning the whole suffix after merge construction already compared each entry.
    is_identity: bool,
}

impl StringIdRemap {
    pub fn get(&self, old: StringId) -> StringId {
        let old_index = old.0 as usize;
        if old_index < self.identity_prefix_len {
            return old;
        }

        self.mapped_suffix[old_index - self.identity_prefix_len]
    }

    /// Returns true when every source ID maps to the same numeric ID in the target table.
    pub fn is_identity(&self) -> bool {
        self.is_identity
    }

    /// Returns true when any ID at or after `base_len` changes during remapping.
    pub fn has_non_identity_after(&self, base_len: usize) -> bool {
        if self.is_identity {
            return false;
        }

        let remap_len = self.identity_prefix_len + self.mapped_suffix.len();
        if base_len >= remap_len {
            return false;
        }

        let suffix_start = base_len.saturating_sub(self.identity_prefix_len);
        self.mapped_suffix
            .iter()
            .enumerate()
            .skip(suffix_start)
            .any(|(offset, mapped)| mapped.0 as usize != self.identity_prefix_len + offset)
    }
}

/// Shared immutable prefix used by module-local string-table forks.
///
/// WHAT: owns the strings visible before a group of module compilations starts.
/// WHY: every module fork can resolve inherited IDs without cloning the full table or rebuilding
/// the inherited reverse lookup map.
#[derive(Debug)]
struct StringTableBase {
    strings: Box<[Box<str>]>,
    string_to_id: FxHashMap<&'static str, StringId>,
}

impl StringTableBase {
    fn from_table(table: &StringTable) -> Self {
        increment_frontend_counter(FrontendCounter::StringTableForkSourceBaseCopies);

        let strings = table
            .iter()
            .map(|(_, string)| Box::<str>::from(string))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let mut string_to_id =
            FxHashMap::with_capacity_and_hasher(strings.len(), Default::default());
        for (index, string) in strings.iter().enumerate() {
            string_to_id.insert(
                StringTable::static_str_key(string.as_ref()),
                StringId(index as u32),
            );
        }

        Self {
            strings,
            string_to_id,
        }
    }

    fn len(&self) -> usize {
        self.strings.len()
    }

    fn resolve(&self, id: StringId) -> &str {
        // SAFETY: forked StringIds below base_len are issued by this base snapshot.
        unsafe { self.strings.get_unchecked(id.0 as usize).as_ref() }
    }
}

/// Reusable source for cheap module-local forks that all share one inherited prefix.
#[derive(Debug, Clone)]
pub struct StringTableForkSource {
    base: Arc<StringTableBase>,
}

impl StringTableForkSource {
    pub fn base_len(&self) -> usize {
        self.base.len()
    }

    pub fn fork_for_module(&self) -> StringTableFork {
        StringTableFork {
            string_table: StringTable::from_shared_base(Arc::clone(&self.base)),
            base_len: self.base.len(),
        }
    }
}

/// A module-local string table plus the inherited prefix length used at merge time.
#[derive(Debug)]
pub struct StringTableFork {
    string_table: StringTable,
    base_len: usize,
}

impl StringTableFork {
    pub fn base_len(&self) -> usize {
        self.base_len
    }

    pub fn into_parts(self) -> (StringTable, usize) {
        (self.string_table, self.base_len)
    }
}

/// A centralized string interning system that stores unique strings only once in memory.
///
/// The StringTable uses a dual-mapping approach for optimal performance. A build-owned table
/// stores all strings locally. A module fork stores only module-local strings while sharing an
/// immutable base snapshot for inherited IDs.
#[derive(Debug)]
pub struct StringTable {
    /// Optional inherited prefix shared by cheap module-local forks.
    base: Option<Arc<StringTableBase>>,

    /// Local storage: ID → String mapping for fast resolution.
    ///
    /// Build tables have no base, so this contains every string. Forked tables keep only strings
    /// interned after the fork's base length.
    strings: Vec<Box<str>>,

    /// Local reverse lookup for fast interning.
    ///
    /// Base strings are looked up through `StringTableBase`; this map only owns the local suffix
    /// for forked tables.
    string_to_id: FxHashMap<&'static str, StringId>,

    /// Next available string ID
    next_id: u32,
}

impl Default for StringTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for StringTable {
    fn clone(&self) -> Self {
        increment_frontend_counter(FrontendCounter::StringTableFullClones);

        let strings = self
            .iter()
            .map(|(_, string)| Box::<str>::from(string))
            .collect::<Vec<_>>();
        let mut string_to_id =
            FxHashMap::with_capacity_and_hasher(strings.len(), Default::default());

        for (index, string) in strings.iter().enumerate() {
            string_to_id.insert(
                Self::static_str_key(string.as_ref()),
                StringId(index as u32),
            );
        }

        Self {
            base: None,
            strings,
            string_to_id,
            next_id: self.len() as u32,
        }
    }
}

impl StringTable {
    /// Create a new empty string table
    pub fn new() -> Self {
        Self {
            base: None,
            next_id: 0,
            strings: Vec::with_capacity(MINIMUM_STRING_TABLE_CAPACITY),
            string_to_id: FxHashMap::default(),
        }
    }

    /// Create a new string table with a specified initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            base: None,
            next_id: 0,
            strings: Vec::with_capacity(capacity + MINIMUM_STRING_TABLE_CAPACITY),
            string_to_id: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
        }
    }

    fn from_shared_base(base: Arc<StringTableBase>) -> Self {
        Self {
            next_id: base.len() as u32,
            base: Some(base),
            strings: Vec::with_capacity(MINIMUM_STRING_TABLE_CAPACITY),
            string_to_id: FxHashMap::default(),
        }
    }

    /// Clone this table while retaining any inherited shared prefix.
    ///
    /// Ordinary [`Clone`] deliberately flattens the table into independent storage. Generated
    /// preparation contexts instead remain in the same compilation batch, so they can share the
    /// inherited prefix and copy only their local suffix. A root table has no inherited prefix and
    /// therefore still performs one full clone.
    pub(crate) fn clone_preserving_inherited_prefix(&self) -> Self {
        if self.base.is_none() {
            return self.clone();
        }

        let base_len = self.base_len();
        let strings = self
            .strings
            .iter()
            .map(|string| Box::<str>::from(string.as_ref()))
            .collect::<Vec<_>>();
        let mut string_to_id =
            FxHashMap::with_capacity_and_hasher(strings.len(), Default::default());
        for (index, string) in strings.iter().enumerate() {
            string_to_id.insert(
                Self::static_str_key(string.as_ref()),
                StringId((base_len + index) as u32),
            );
        }

        Self {
            base: self.base.clone(),
            strings,
            string_to_id,
            next_id: self.next_id,
        }
    }

    /// Intern a string slice, returning its unique ID.
    /// If the string already exists, returns the existing ID.
    /// If the string is new, stores it and returns a new ID.
    #[inline]
    pub fn intern(&mut self, s: &str) -> StringId {
        if let Some(base) = &self.base
            && let Some(&existing_id) = base.string_to_id.get(s)
        {
            return existing_id;
        }

        // Fast path: check if the local suffix already has this string. This is a single hash
        // lookup with no allocations.
        if let Some(&existing_id) = self.string_to_id.get(s) {
            return existing_id;
        }

        // Slow path: string is new, so we need to store it
        self.intern_new(s)
    }

    /// Internal helper for interning new strings (cold path)
    /// Marked as cold to optimize the hot path in `intern`
    #[cold]
    #[inline(never)]
    fn intern_new(&mut self, s: &str) -> StringId {
        let new_id = StringId(self.next_id);
        self.next_id += 1;

        // Box<str> is more memory efficient than String (no capacity field)
        let boxed: Box<str> = s.into();

        // SAFETY: the reverse lookup stores a reference into the Box<str> owned by this table.
        // The key is only used while the owning table is alive, strings are never removed, and the
        // heap allocation behind Box<str> remains stable even if the Vec reallocates.
        let static_ref = Self::static_str_key(boxed.as_ref());

        // Insert into reverse lookup (uses the static ref as key - no allocation!)
        self.string_to_id.insert(static_ref, new_id);

        // Store the owned string
        self.strings.push(boxed);

        new_id
    }

    /// Resolve an interned string ID back to its string content.
    ///
    /// Time complexity: O(1)
    ///
    /// # Safety
    /// This uses unchecked indexing for maximum performance.
    /// StringIds are only created by this StringTable, so indices are guaranteed valid.
    #[inline]
    pub fn resolve(&self, id: StringId) -> &str {
        let index = id.0 as usize;
        if let Some(base) = &self.base
            && index < base.len()
        {
            return base.resolve(id);
        }

        let local_index = index - self.base_len();

        // SAFETY: StringIds are only created by this StringTable and are guaranteed
        // to be valid indices into either the shared base or the local suffix.
        unsafe { self.strings.get_unchecked(local_index).as_ref() }
    }

    /// Resolve an interned string ID, or `None` when the handle is not in this table.
    ///
    /// WHAT: gives retained-state validators a fallible lookup for interned extensions and
    ///       other handles that may have been remapped incorrectly.
    /// WHY: `resolve` is only safe for IDs issued by this table. Malformed retained state
    ///      must become `CompilerError`, not an unchecked index.
    #[inline]
    pub fn try_resolve(&self, id: StringId) -> Option<&str> {
        let index = id.0 as usize;
        if index >= self.len() {
            return None;
        }

        Some(self.resolve(id))
    }

    /// Efficiently intern a String by taking ownership, avoiding an extra allocation
    /// if the string is new.
    ///
    /// Time complexity: O(1) average case
    #[inline]
    pub fn get_or_intern(&mut self, s: String) -> StringId {
        if let Some(base) = &self.base
            && let Some(&existing_id) = base.string_to_id.get(s.as_str())
        {
            return existing_id;
        }

        // Fast path: check if the local suffix already has this string.
        if let Some(&existing_id) = self.string_to_id.get(s.as_str()) {
            return existing_id;
        }

        // Slow path: string is new
        self.intern_new_owned(s)
    }

    /// Internal helper for interning new owned strings (cold path)
    #[cold]
    #[inline(never)]
    fn intern_new_owned(&mut self, s: String) -> StringId {
        let new_id = StringId(self.next_id);
        self.next_id += 1;

        // Convert directly to Box<str> to avoid keeping String's capacity overhead
        let boxed: Box<str> = s.into_boxed_str();

        // SAFETY: Same reasoning as intern_new.
        let static_ref = Self::static_str_key(boxed.as_ref());

        self.string_to_id.insert(static_ref, new_id);
        self.strings.push(boxed);

        new_id
    }

    /// Get the number of unique strings stored in the table
    #[inline]
    pub fn len(&self) -> usize {
        self.base_len() + self.strings.len()
    }

    /// Iterate over all interned strings with their IDs.
    pub fn iter(&self) -> impl Iterator<Item = (StringId, &str)> + use<'_> {
        let base_iter = self
            .base
            .iter()
            .flat_map(|base| base.strings.iter().enumerate())
            .map(|(index, string)| (StringId(index as u32), string.as_ref()));

        let base_len = self.base_len();
        let local_iter = self
            .strings
            .iter()
            .enumerate()
            .map(move |(index, string)| (StringId((base_len + index) as u32), string.as_ref()));

        base_iter.chain(local_iter)
    }

    /// Create a reusable fork source for compiling multiple modules or files against the same prefix.
    ///
    /// Building the shared base copies the current table once. Each fork after that clones only an
    /// `Arc` and starts with an empty local delta.
    pub fn fork_source(&self) -> StringTableForkSource {
        StringTableForkSource {
            base: Arc::new(StringTableBase::from_table(self)),
        }
    }

    /// Create one local fork while remembering the inherited prefix length.
    ///
    /// Directory/module compilation should create one `StringTableForkSource` and reuse it for all
    /// independent module or file workers so the inherited prefix is copied once for the batch.
    pub fn fork_for_module(&self) -> StringTableFork {
        self.fork_source().fork_for_module()
    }

    /// Merge all strings from `other` into `self`. Returns a remap so callers can
    /// rewrite `StringId`s that were issued against `other` to IDs valid in `self`.
    pub fn merge_from(&mut self, other: &StringTable) -> StringIdRemap {
        add_frontend_counter(
            FrontendCounter::StringTableMergeFromSourceEntriesScanned,
            other.len(),
        );

        let mut old_to_new = Vec::with_capacity(other.len());
        let mut is_identity = true;
        for (old_id, s) in other.iter() {
            let new_id = self.intern(s);
            if new_id != old_id {
                is_identity = false;
            }
            old_to_new.push(new_id);
            debug_assert_eq!(old_id.0 as usize, old_to_new.len() - 1);
        }

        StringIdRemap {
            identity_prefix_len: 0,
            mapped_suffix: old_to_new,
            is_identity,
        }
    }

    /// Merge only strings added after a module fork's inherited prefix.
    ///
    /// IDs below `base_len` are valid in both tables because the module table was forked from the
    /// build table before local compilation started. The returned remap keeps that prefix implicit
    /// and maps only the local suffix that may collide with strings merged from earlier modules.
    pub fn merge_delta_from(&mut self, other: &StringTable, base_len: usize) -> StringIdRemap {
        debug_assert!(base_len <= self.len());
        debug_assert!(base_len <= other.len());
        increment_frontend_counter(FrontendCounter::StringTableDeltaMergeCalls);

        #[cfg(debug_assertions)]
        for index in 0..base_len {
            let id = StringId(index as u32);
            debug_assert_eq!(self.resolve(id), other.resolve(id));
        }

        let delta_len = other.len().saturating_sub(base_len);
        add_frontend_counter(
            FrontendCounter::StringTableMergeFromSourceEntriesScanned,
            delta_len,
        );
        add_frontend_counter(FrontendCounter::StringTableDeltaEntriesScanned, delta_len);

        let mut mapped_suffix = Vec::with_capacity(delta_len);
        let mut is_identity = true;
        #[cfg(feature = "detailed_timers")]
        let mut non_identity_entries = 0usize;
        for (old_id, string) in other.iter().skip(base_len) {
            let expected_old_index = base_len + mapped_suffix.len();
            debug_assert_eq!(old_id.0 as usize, expected_old_index);

            let new_id = self.intern(string);
            if new_id.0 as usize != expected_old_index {
                is_identity = false;
                #[cfg(feature = "detailed_timers")]
                {
                    non_identity_entries += 1;
                }
            }
            mapped_suffix.push(new_id);
        }

        #[cfg(feature = "detailed_timers")]
        {
            if is_identity {
                increment_frontend_counter(FrontendCounter::StringTableDeltaIdentityRemaps);
            } else {
                increment_frontend_counter(FrontendCounter::StringTableDeltaNonIdentityRemaps);
                add_frontend_counter(
                    FrontendCounter::StringTableDeltaNonIdentityEntries,
                    non_identity_entries,
                );
            }
        }

        StringIdRemap {
            identity_prefix_len: base_len,
            mapped_suffix,
            is_identity,
        }
    }

    #[inline]
    fn static_str_key(value: &str) -> &'static str {
        // SAFETY: callers only pass strings owned by the same table/base that stores the returned
        // reference in its reverse lookup map. The owning storage outlives all lookups through that
        // map, and strings are never removed while the map is alive.
        unsafe { std::mem::transmute::<&str, &'static str>(value) }
    }

    #[inline]
    fn base_len(&self) -> usize {
        self.base.as_ref().map_or(0, |base| base.len())
    }
}
