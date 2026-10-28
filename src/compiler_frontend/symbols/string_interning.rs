//! String interning for the compiler frontend.
//!
//! WHAT: deduplicates strings into a centralized table and hands out compact `StringId` handles,
//!       making equality comparisons O(1) and reducing memory pressure from repeated identifiers.
//! WHY: the compiler frontend deals with thousands of repeated identifiers, paths, and keywords;
//!      interning is one of the cheapest ways to lower allocation overhead and speed up lookups.

use crate::compiler_frontend::compiler_messages::CompilerStage;
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
/// Render-only lookup over interned strings.
///
/// WHAT: exposes checked and infallible lookup without interning access.
/// WHY: frozen contexts must serve the same renderers as the mutable table without a second
///      implementation or a copied string store.
pub(crate) trait StringTableResolver {
    /// Resolve an interned string ID back to its string content.
    fn resolve(&self, id: StringId) -> &str;

    /// Resolve an interned string ID when it belongs to this identity table.
    #[allow(dead_code)] // Retained for deferred checked lookups across identity tables.
    fn try_resolve(&self, id: StringId) -> Option<&str>;
}

impl StringTableResolver for StringTable {
    #[inline]
    fn resolve(&self, id: StringId) -> &str {
        StringTable::try_resolve(self, id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "StringTableResolver received invalid {id}"
            )
        })
    }

    #[inline]
    fn try_resolve(&self, id: StringId) -> Option<&str> {
        StringTable::try_resolve(self, id)
    }
}
impl StringTableResolver for Box<StringTable> {
    #[inline]
    fn resolve(&self, id: StringId) -> &str {
        self.as_ref().try_resolve(id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "StringTableResolver received invalid {id}"
            )
        })
    }

    #[inline]
    fn try_resolve(&self, id: StringId) -> Option<&str> {
        self.as_ref().try_resolve(id)
    }
}

impl StringTableResolver for FrozenStringTable {
    #[inline]
    fn resolve(&self, id: StringId) -> &str {
        FrozenStringTable::try_resolve(self, id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "StringTableResolver received invalid {id}"
            )
        })
    }

    #[inline]
    fn try_resolve(&self, id: StringId) -> Option<&str> {
        FrozenStringTable::try_resolve(self, id)
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

    /// Number of source IDs that are unchanged in the remap.
    pub(crate) fn identity_prefix_len(&self) -> usize {
        self.identity_prefix_len
    }
}

/// Shared immutable prefix used by module-local string-table forks.
///
/// WHAT: owns the strings visible before a group of module compilations starts.
/// WHY: every module fork can resolve inherited IDs without cloning the full table or rebuilding
/// the inherited reverse lookup map.
#[derive(Debug)]
struct StringTableBase {
    strings: Box<[Arc<str>]>,
    string_to_id: FxHashMap<&'static str, StringId>,
}

impl StringTableBase {
    fn from_table(table: &StringTable) -> Self {
        increment_frontend_counter(FrontendCounter::StringTableForkSourceBaseCopies);

        let mut strings = Vec::with_capacity(table.len());
        if let Some(base) = &table.base {
            strings.extend(base.strings.iter().cloned());
        }
        strings.extend(table.strings.iter().cloned());
        let strings = strings.into_boxed_slice();

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

    fn try_resolve(&self, id: StringId) -> Option<&str> {
        self.strings
            .get(id.0 as usize)
            .map(|string| string.as_ref())
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

/// Immutable interned strings for lookup after identity construction freezes.
///
/// WHAT: retains the dense string storage and stable [`StringId`] indexes while dropping the
///       reverse map required only during interning.
/// WHY: final lookup contexts must move the existing string allocations into immutable storage
///      without keeping a mutable interner or allocating a second copy of every string.
#[derive(Debug, PartialEq, Eq)]
pub struct FrozenStringTable {
    strings: Box<[Arc<str>]>,
}

impl FrozenStringTable {
    /// Return the number of strings in this frozen table.
    #[inline]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Bytes occupied by the frozen string-table backing slice and string payloads.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn storage_bytes(&self) -> usize {
        self.strings
            .len()
            .saturating_mul(std::mem::size_of::<Arc<str>>())
            .saturating_add(
                self.strings
                    .iter()
                    .map(|string| string.len())
                    .sum::<usize>(),
            )
    }
    /// Resolve an interned string ID back to its string content.
    ///
    /// # Panics
    /// Panics when the ID was not issued by this table.
    #[inline]
    pub fn resolve(&self, id: StringId) -> &str {
        self.try_resolve(id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "FrozenStringTable::resolve received invalid {id}"
            )
        })
    }

    /// Resolve an interned string ID, or `None` when the handle is not in this table.
    #[inline]
    pub fn try_resolve(&self, id: StringId) -> Option<&str> {
        self.strings
            .get(id.0 as usize)
            .map(|string| string.as_ref())
    }

    /// Iterate over all frozen strings with their stable IDs.
    pub fn iter(&self) -> impl Iterator<Item = (StringId, &str)> + use<'_> {
        self.strings
            .iter()
            .enumerate()
            .map(|(index, string)| (StringId(index as u32), string.as_ref()))
    }
}
#[cfg(feature = "data_layout_memory_probe")]
impl Drop for FrozenStringTable {
    fn drop(&mut self) {
        crate::compiler_frontend::instrumentation::release_donor_identity_string(self);
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
    strings: Vec<Arc<str>>,

    /// Local reverse lookup for fast interning.
    ///
    /// Base strings are looked up through `StringTableBase`; this map only owns the local suffix
    /// for forked tables.
    string_to_id: FxHashMap<&'static str, StringId>,

    /// Next available string ID. Exhausting the u32 space would need hundreds of gigabytes of
    /// retained strings, so the checked increment is a guard, not a reachable limit.
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
            .map(|(_, string)| Arc::<str>::from(string))
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

    /// Create an empty table without reserving storage for an identity already owned elsewhere.
    pub(crate) fn empty_unallocated() -> Self {
        Self {
            base: None,
            next_id: 0,
            strings: Vec::new(),
            string_to_id: FxHashMap::default(),
        }
    }
    /// Move this table to another owner without allocating a replacement string buffer.
    ///
    /// Call when the current producer is done interning; the empty replacement has no reserved
    /// capacity because the transferred identity allocation belongs to the recipient.
    pub(crate) fn take_for_handoff(&mut self) -> Self {
        std::mem::replace(self, Self::empty_unallocated())
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
    /// Ordinary [`Clone`] deliberately flattens the table into independent string allocations.
    /// Generated preparation contexts instead remain in the same compilation batch, so they can
    /// share the inherited prefix and fork local storage while sharing each immutable string.
    /// For a root table, its full identity becomes the shared inherited prefix.
    pub(crate) fn clone_preserving_inherited_prefix(&self) -> Self {
        if self.base.is_none() {
            return Self::from_shared_base(Arc::new(StringTableBase::from_table(self)));
        }

        let base_len = self.base_len();
        let strings = self.strings.to_vec();
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

    /// Return the ID of an already interned string without interning it.
    #[inline]
    pub(crate) fn existing(&self, s: &str) -> Option<StringId> {
        if let Some(base) = &self.base
            && let Some(&existing_id) = base.string_to_id.get(s)
        {
            return Some(existing_id);
        }
        self.string_to_id.get(s).copied()
    }

    /// Intern a string slice, returning its unique ID.
    /// If the string already exists, returns the existing ID.
    /// If the string is new, stores it and returns a new ID.
    #[inline]
    pub fn intern(&mut self, s: &str) -> StringId {
        // Fast path: base or local lookup, with no allocation.
        if let Some(existing_id) = self.existing(s) {
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
        let new_id = self.allocate_id();

        // Arc<str> lets frozen snapshots share immutable string bytes with the live table.
        let shared: Arc<str> = Arc::from(s);

        // SAFETY: the reverse lookup stores a reference into the Arc<str> owned by this table.
        // The key is only used while the owning table is alive, strings are never removed, and the
        // heap allocation behind Arc remains stable if the Vec reallocates or snapshots are made.
        let static_ref = Self::static_str_key(shared.as_ref());

        // Insert into reverse lookup (uses the static ref as key - no allocation!)
        self.string_to_id.insert(static_ref, new_id);

        // Store the owned string
        self.strings.push(shared);

        new_id
    }

    /// Resolve an interned string ID back to its string content.
    ///
    /// Time complexity: O(1)
    ///
    /// # Panics
    /// Panics when the ID was not issued by this table.
    #[inline]
    pub fn resolve(&self, id: StringId) -> &str {
        self.try_resolve(id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "StringTable::resolve received invalid {id}"
            )
        })
    }

    /// Resolve an interned string ID, or `None` when the handle is not in this table.
    ///
    /// WHAT: gives retained-state validators a fallible lookup for interned extensions and
    ///       other handles that may have been remapped incorrectly.
    /// WHY: malformed retained state must become `InfrastructureFailure`, not an unchecked index.
    #[inline]
    pub fn try_resolve(&self, id: StringId) -> Option<&str> {
        let index = id.0 as usize;
        let base_len = self.base_len();
        if index < base_len {
            return self.base.as_ref()?.try_resolve(id);
        }

        self.strings
            .get(index - base_len)
            .map(|string| string.as_ref())
    }

    /// Efficiently intern an owned String.
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
        let new_id = self.allocate_id();

        let shared: Arc<str> = Arc::from(s);

        // SAFETY: Same reasoning as intern_new.
        let static_ref = Self::static_str_key(shared.as_ref());

        self.string_to_id.insert(static_ref, new_id);
        self.strings.push(shared);

        new_id
    }

    /// Issue the next `StringId`.
    ///
    /// Every id names a distinct retained `Arc<str>`, so 2^32 ids need far more memory than the
    /// compiler can hold: exhausting the space is a broken invariant, not an authored input limit.
    /// Interning runs in every stage, so the reported stage is nominal.
    fn allocate_id(&mut self) -> StringId {
        let new_id = StringId(self.next_id);
        self.next_id = self.next_id.checked_add(1).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "StringTable exhausted its u32 id space"
            )
        });
        new_id
    }

    /// Get the number of unique strings stored in the table
    #[inline]
    pub fn len(&self) -> usize {
        self.base_len() + self.strings.len()
    }

    /// Consume a merged root table into immutable lookup storage.
    ///
    /// Module-local forks retain an inherited base and therefore cannot be frozen without
    /// flattening or copying that base. Merge all fork deltas into the build-owned root table
    /// first; this operation then moves the existing string allocations and drops both reverse
    /// lookup maps.
    pub fn freeze(self) -> FrozenStringTable {
        assert!(
            self.base.is_none(),
            "StringTable::freeze requires a merged root table"
        );
        #[cfg(feature = "data_layout_memory_probe")]
        self.record_memory_ledger_sample();
        debug_assert_eq!(self.next_id as usize, self.strings.len());

        FrozenStringTable {
            strings: self.strings.into_boxed_slice(),
        }
    }

    /// Capture an immutable lookup snapshot while retaining this mutable table.
    ///
    /// The snapshot clones only Arc handles, so existing string allocations are shared and
    /// later interning cannot change its prefix or StringId ordering.
    pub(crate) fn freeze_snapshot(&self) -> FrozenStringTable {
        #[cfg(feature = "data_layout_memory_probe")]
        self.record_memory_ledger_sample();
        let mut strings = Vec::with_capacity(self.len());
        if let Some(base) = &self.base {
            strings.extend(base.strings.iter().cloned());
        }
        strings.extend(self.strings.iter().cloned());
        debug_assert_eq!(self.next_id as usize, strings.len());

        FrozenStringTable {
            strings: strings.into_boxed_slice(),
        }
    }

    /// Consume this table into immutable lookup storage, flattening a module fork first.
    ///
    /// WHAT: a root table freezes exactly as [`Self::freeze`] does. A module fork moves its
    /// inherited Arc handles when it holds the last reference to the prefix, and otherwise clones
    /// only those handles while sharing the immutable string allocations. Its local suffix is
    /// appended unchanged, so every `StringId` keeps its index.
    /// WHY: a module-local batch can reach the boundary still holding its fork, and the boundary
    /// needs one flat table without rebuilding inherited string contents.
    pub(crate) fn freeze_flattening(mut self) -> FrozenStringTable {
        if self.base.is_none() {
            return self.freeze();
        }
        #[cfg(feature = "data_layout_memory_probe")]
        self.record_memory_ledger_sample();
        let base = self.base.take().expect("the string table base was present");
        let mut strings = Vec::with_capacity(base.len() + self.strings.len());
        match Arc::try_unwrap(base) {
            Ok(base) => strings.extend(Vec::from(base.strings)),
            Err(shared) => strings.extend(shared.strings.iter().cloned()),
        }
        strings.append(&mut self.strings);
        debug_assert_eq!(self.next_id as usize, strings.len());

        FrozenStringTable {
            strings: strings.into_boxed_slice(),
        }
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
    /// Building the shared base snapshots the IDs and shares the immutable string allocations.
    /// Each fork then clones only an `Arc` and starts with an empty local delta.
    pub fn fork_source(&self) -> StringTableForkSource {
        StringTableForkSource {
            base: Arc::new(StringTableBase::from_table(self)),
        }
    }

    /// Create one local fork while remembering the inherited prefix length.
    ///
    /// Directory/module compilation should create one `StringTableForkSource` and reuse it for all
    /// independent module or file workers so the inherited string allocations are shared once
    /// for the batch.
    pub fn fork_for_module(&self) -> StringTableFork {
        self.fork_source().fork_for_module()
    }

    /// Merge all strings from `other` into `self`. Returns a remap so callers can
    /// rewrite `StringId`s that were issued against `other` to IDs valid in `self`.
    pub fn merge_from(&mut self, other: &StringTable) -> StringIdRemap {
        #[cfg(feature = "data_layout_memory_probe")]
        let merge_clock = crate::compiler_frontend::instrumentation::start_merge_remap_clock();
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

        let remap = StringIdRemap {
            identity_prefix_len: 0,
            mapped_suffix: old_to_new,
            is_identity,
        };
        #[cfg(feature = "data_layout_memory_probe")]
        crate::compiler_frontend::instrumentation::record_string_merge_remap(merge_clock);
        remap
    }

    /// Merge only strings added after a module fork's inherited prefix.
    ///
    /// IDs below `base_len` are valid in both tables because the module table was forked from the
    /// build table before local compilation started. The returned remap keeps that prefix implicit
    /// and maps only the local suffix that may collide with strings merged from earlier modules.
    pub fn merge_delta_from(&mut self, other: &StringTable, base_len: usize) -> StringIdRemap {
        #[cfg(feature = "data_layout_memory_probe")]
        let merge_clock = crate::compiler_frontend::instrumentation::start_merge_remap_clock();
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

        let remap = StringIdRemap {
            identity_prefix_len: base_len,
            mapped_suffix,
            is_identity,
        };
        #[cfg(feature = "data_layout_memory_probe")]
        crate::compiler_frontend::instrumentation::record_string_merge_remap(merge_clock);
        remap
    }

    #[inline]
    fn static_str_key(value: &str) -> &'static str {
        // SAFETY: callers only pass strings owned by the same table/base that stores the returned
        // reference in its reverse lookup map. The owning storage outlives all lookups through that
        // map, and strings are never removed while the map is alive.
        unsafe { std::mem::transmute::<&str, &'static str>(value) }
    }

    #[cfg(feature = "data_layout_memory_probe")]
    fn record_memory_ledger_sample(&self) {
        let inherited = self.base.as_deref();
        crate::compiler_frontend::instrumentation::record_mutable_string_table(
            self.strings
                .capacity()
                .saturating_mul(std::mem::size_of::<Arc<str>>()),
            self.strings.iter().map(|string| string.len()).sum(),
            self.string_to_id.capacity(),
            inherited.map_or(0, |base| base.strings.len()),
            inherited.map_or(0, |base| {
                base.strings
                    .len()
                    .saturating_mul(std::mem::size_of::<Arc<str>>())
            }),
            inherited.map_or(0, |base| base.string_to_id.capacity()),
        );
    }

    #[inline]
    fn base_len(&self) -> usize {
        self.base.as_ref().map_or(0, |base| base.len())
    }
}
