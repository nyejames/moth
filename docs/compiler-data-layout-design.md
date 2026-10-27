# Moth Compiler Data Layout Design

> **Current architecture.** This document owns the compiler's physical source, path, token and
> diagnostic representations, their lifetimes, and the boundary between user diagnostics,
> infrastructure failures and compiler bugs.
>
> `docs/src/docs/progress/@page.moth` records implementation support. This document describes the
> current data and ownership contracts.


## Authority and ownership

This document is the canonical authority for the physical representation, ownership, lifetime and
extension rules of Moth compiler source metadata, retained tokens and diagnostics.

It owns:

- build-lifetime source identity and source snapshots
- exact byte-based source spans
- compact local and global span encodings
- true logical-path interning
- source-owned retained token storage, including the lossless retained numeric
  text side store described under `Compact source-owned tokens`
- compact token records and typed token side stores
- compact diagnostic drafts, records, labels, facts and side stores
- diagnostic schema declaration and validation
- reduced type-display snapshots for diagnostics
- build-then-freeze compiler identity contexts
- the MON caller-owned input snapshot, cursor, span, public error context and owned-result handoff, which uses no compiler identity table
- the boundary between user diagnostics, operational infrastructure failures and compiler bugs

It does not override:

- `docs/compiler-design-overview.md` for compiler stage and semantic ownership
- `docs/build-system-design.md` for Stage 0, graph scheduling, project tooling and host orchestration
- `docs/src/docs/mon/mon-format.mtf` for MON literal-data format semantics
- `docs/src/developer-docs/language/overview.mtf` and its selected canonical references for source-language behaviour
- the memory-management authorities for Moth program semantics
- `docs/src/docs/progress/@page.moth` for current implementation status
- `docs/roadmap/roadmap.md` for sequencing and deferred work

Update every shared boundary in both owning authorities in the same change. This document sets
physical representation and ownership contracts; it does not replace stage, scheduling or language
semantics owned elsewhere.

## Change-control policy

The distinction keeps hard layout contracts separate from measured representation choices.

### Locked decisions

The following decisions may not be changed by an implementation agent without explicit user
approval and a corresponding architecture update:

- `LocalSpan` is exactly 4 bytes.
- `SourceSpan` is exactly 8 bytes.
- `Option<SourceSpan>` is exactly 8 bytes.
- `TokenShape` is exactly 8 bytes.
- the durable common `DiagnosticRecord` is exactly 32 bytes.
- spans are exact half-open UTF-8 byte ranges.
- durable spans never contain paths, line/column pairs or syntax-aware end recipes.
- source text and line starts are owned by a build-lifetime source database.
- retained tokens are source-owned; retained syntax uses ranges or handles instead of cloned token
  vectors.
- complex token payloads and unusual diagnostic data use typed side stores rather than widening
  common records.
- `PathId` is a genuine interned path identity, not a wrapper around an owned component vector.
- one source-owned `PathSyntaxTable` owns every authored path in a source; path tokens carry the
  canonical `TokenTag` plus a dense `PathSyntaxId` handle rather than expanded component payloads.
- diagnostic families are declared through one schema authority.
- user mistakes, operational infrastructure failures and compiler bugs are three separate failure
  lanes.
- only proven compiler invariant violations may panic.
- complete `TypeEnvironment` values are not retained solely to render diagnostics.
- normal compilation and rendering contexts are shared or moved, not deep-cloned.
- process-local compact IDs are not persistent artefact identities.

### Current measured choices

`LocalSpan` uses a 22-bit start/index and 10-bit length split. Source tokens use separate dense
shape and span arrays, with numeric, path and segmented-sequence data in typed source-local stores.
Logical paths use eight-byte parent-linked `PathNode` rows. Extended spans are append-only and are
not deduplicated.

Changes to these choices require an explicit architecture update, a bounded experiment against the
current corpus, and the re-entry evidence in `Deferred capabilities`. Record exact measurements and
maintenance costs in the existing benchmark report.

## Design goals

The layout is designed around the data that appears most often, not around the largest exceptional
case.

1. **Keep ubiquitous records compact.** Source locations and tokens are carried through most of the
   frontend. Their common representation must remain small and trivially movable.
2. **Keep locations exact.** Compression must never require reparsing source or guessing where a
   construct ends.
3. **Pay for rare data only when it exists.** Large lists, secondary spans, path groups, numeric
   details and unusual diagnostics belong in typed side stores.
4. **Make ownership obvious.** One build context owns each identity table. Mutable construction is
   followed by an explicit immutable freeze.
5. **Preserve deterministic parallel compilation.** IDs, diagnostics and output order must not depend
   on worker completion order.
6. **Keep rendering separate from semantic production.** Compiler stages emit compact facts. Renderers
   resolve source, strings, paths and type display records at the boundary.
7. **Prevent future enum widening.** New token or diagnostic families must fit the fixed common
   record and use a side store when they do not.
8. **Measure real wins.** Aggressive packing remains only when memory or throughput evidence is
   repeatable and complexity is justified.

## Non-goals

This design does not introduce:

- source-language syntax or semantic changes
- new diagnostic wording merely because storage changes
- persistent cache or artefact serialization for process-local IDs
- a general-purpose serialization format for compiler records
- a global conversion of every compiler ID to a packed/non-zero representation
- a general allocator or lifetime-heavy arena framework
- a second parser or delimiter-matching language inside source spans
- a global mutable interner protected by a lock
- LSP protocol implementation beyond the source-position conversion primitives needed by tooling
- compatibility wrappers for the current location, token, diagnostic or error models

## Hard representation invariants

All size assertions below apply to supported 64-bit targets. Types made entirely from fixed-width
integers must also retain their stated logical size on supported 32-bit targets.
Code paths in the table are relative to `src/compiler_frontend/`.

| Type | Asserted size | Assertion owner |
|---|---:|---|
| `SourceId` | 4 bytes | `source/tests/database_tests.rs` |
| `Option<SourceId>` | 4 bytes | `source/tests/database_tests.rs` |
| `LocalSpan` | 4 bytes | `source/span.rs` |
| `Option<LocalSpan>` | 4 bytes | `source/span.rs` |
| `SourceSpan` | 8 bytes | `source/span.rs` |
| `Option<SourceSpan>` | 8 bytes | `source/span.rs` |
| `ExtendedSpan` | 8 bytes | `source/span.rs` |
| `PathId` | 4 bytes | `symbols/path_interner/tests.rs` |
| `Option<PathId>` | 4 bytes | `symbols/path_interner/tests.rs` |
| `PathNode` | 8 bytes | `symbols/path_interner/tests.rs` |
| Test-only `LogicalPath` wrapper | 4 bytes | `symbols/path_interner/tests.rs` |
| `Option<LogicalPath>` | 4 bytes | `symbols/path_interner/tests.rs` |
| `TokenShape` | 8 bytes | `tokenizer/schema.rs` |
| `TokenSequenceRange` | 8 bytes | `tokenizer/storage.rs` |
| `DiagnosticToken` | 8 bytes | `compiler_messages/diagnostic_token.rs` |
| `DiagnosticTypeRecord` | 12 bytes | `compiler_messages/diagnostic_types/type_display_snapshot.rs` |
| `ExtraDataRef` | 12 bytes | `compiler_messages/diagnostics/extras.rs` |
| `DiagnosticExtraRecord` | 28 bytes | `compiler_messages/diagnostics/extras.rs` |
| `ForeignSiteRecord` | 8 bytes | `compiler_messages/diagnostics/foreign.rs` |
| `GenericSubstitutionRecord` | 8 bytes | `compiler_messages/diagnostics/labels.rs` |
| `SecondaryDiagnosticLabel` | 12 bytes | `compiler_messages/diagnostics/labels.rs` |
| `DiagnosticRecord` | 32 bytes | `compiler_messages/diagnostics/record.rs` |
| `DiagnosticDraft` | at most 48 bytes | `compiler_messages/diagnostics/draft.rs` |
| `InfrastructureFailure` | at most 56 bytes | `compiler_messages/compiler_errors.rs` |
| `PendingDiagnostics` | at most 48 bytes | `compiler_messages/diagnostics/freeze.rs` |
| `PreparedDiagnosticBatch` | at most 48 bytes | `compiler_messages/prepared_batch.rs` |
| `DiagnosticRun` | 12 bytes | `compiler_messages/diagnostics/report.rs` |
| `DiagnosticReportSet` | at most 48 bytes | `compiler_messages/diagnostics/report.rs` |

The implementation must use compile-time or unit-test layout assertions. Exact layout is a real
internal invariant here, so the testing rule against incidental layout assertions does not apply.

The following are also hard rules:

- no `usize` appears in a durable fixed-width source, token or diagnostic record
- no `Vec`, `String`, `PathBuf`, `HashMap` or wide enum appears inline in a common token or diagnostic
  record
- no common record stores both an identity and another field that can be derived from that identity's
  descriptor table
- no missing identity is represented by a magic valid ID
- reserved bits must be zero on construction and validated when records cross a trust boundary

## Compilation identity context

One project or package compilation boundary owns one identity context. That context is mutable while
compilation runs and becomes a lookup-only frozen bundle afterwards. Diagnostics are **not** part of
the frozen identity bundle; they belong to the report that a diagnosed or warning outcome owns.

```rust
/// Immutable source, string and logical-path identity for one compiler boundary.
pub(crate) struct FrozenIdentityContext {
    sources: FrozenSourceDatabase,
    strings: Arc<FrozenStringTable>,
}

/// One frozen diagnostic identity domain and its compact records.
pub(crate) struct DiagnosticReport {
    context: Arc<FrozenIdentityContext>,
    foreign_contexts: Vec<Option<Arc<FrozenIdentityContext>>>,
    diagnostics: DiagnosticStore,
    types: TypeDisplaySnapshot,
    counts: SeverityCounts,
}

/// Move-only ordered reports and the runs that preserve cross-domain order.
pub struct DiagnosticReportSet {
    reports: Vec<DiagnosticReport>,
    runs: Vec<DiagnosticRun>,
}
```
These are the current owners. `FrozenIdentityContext::sources` is a `FrozenSourceDatabase` that owns
the source slots, snapshots and path table; the context's `strings` field shares the frozen string
table. Mutable construction uses the existing source-database and string-table owners rather than a
separate aggregate identity builder.

- Mutable builders have one explicit owner.
- Compiler APIs borrow only the tables they can extend or inspect.
- Workers use local append-only deltas rather than mutating shared global tables.
- Deltas merge in canonical source or module order.
- Worker-owned records are remapped once before a later consumer can observe them.
- The completed identity context freezes once into immutable lookup storage.
- A result retains `Arc<FrozenIdentityContext>` only while it still contains compact IDs that need
  it. A clean success whose final outputs carry no compact compiler IDs builds no frozen context at
  all and drops source/token/identity data at its last diagnostic-capable boundary.
- `DiagnosticReport` owns its compact `DiagnosticStore` and `TypeDisplaySnapshot`;
  `DiagnosticReportSet` owns ordered reports and canonical runs without a second context or store layer.
- Bulk owning stores are move-only until frozen.
- Compact IDs and records may be `Copy`.
- Broad `Clone` implementations on report sets, diagnostic bags, source databases and owning render
  contexts are prohibited.

A frozen context is a process-local rendering and inspection boundary. It is not a persistent cache
key or a cross-process protocol.

## Source identity and database

### `SourceId`

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(NonZeroU32);
```

`SourceId` identifies one source slot inside an identity context; only a successfully loaded slot
has a `SourceRecord` snapshot.

Rules:

- `SourceId::COMPILATION_ROOT` is the deterministic synthetic compilation-root slot that never
  loads a snapshot. It gives project-wide user diagnostics an exact context-owned primary span
  before or outside any physical source. Its provenance is `SourceProvenance::CompilationRoot`,
  not a fake filesystem path.
- All non-root registered source slots use `SourceProvenance::AuthoredPhysical` and begin after
  the root.
- Inventory-backed lanes assign final IDs in canonical order before tokenization. Traversal-only
  lanes use the single private discovery-finalization barrier below. Every discovery output and
  downstream compiler boundary carries final, immutable source identities.
- Physical sources are sorted by canonical logical source order, never filesystem iteration or
  worker completion order.
- A source adapter may produce synthetic token content, but it must retain provenance to the authored
  source record when diagnostics should point to authored bytes.
- No parallel stage may allocate final `SourceId`s from a shared atomic counter.
- If a future stage genuinely discovers new synthetic source after the initial registration barrier,
  it must return a deterministic source delta that is merged before any persistent record using that
  ID escapes the stage. Late IDs may not leak directly from workers.
- `SourceId` values are not serialized. Persistent artefacts store canonical source identity plus
  exact byte ranges and remap into a new context.
- Absence uses `Option<SourceSpan>`, never a zero source ID or a fabricated source.

### Private discovery finalization

Synthetic single-file traversal and recursive direct-template discovery learn source membership
from preparation. Only these membership-discovering lanes may prepare under provisional IDs.
Directory/package inventories and standalone templates register final IDs before tokenization.

- Keep the provisional database private and disposable. Never compare its IDs with final IDs or
  retain either the database or its IDs outside the traversal owner. Use ordinary `SourceId`,
  not a second provisional-ID type.
- Keep `SourceDatabase::insert` a discovery escape hatch, with the narrowest Rust visibility
  that permits its owners. Inventory-backed compilation uses ordered registration.
- Prepare each source once. On success, one barrier settles the complete reachable closure,
  constructs its final database and normalizes every retained source-bearing record exactly once.
  Normalization may rebind an existing fact or rebuild it against the final table.
- Move snapshots and live span builders without copying. Identity finalization does not freeze a
  builder whose later consumers can still produce spans. Destroy the provisional domain before
  publishing the result.
- Preserve each lane's canonical order: Stage 0 inventory identity order for directory/packages,
  deterministic canonical logical order for synthetic traversal. Neither visit order nor worker
  completion order determines final IDs.
- On diagnosis, apply the same barrier to the known source set before publishing diagnostics and
  their source context. Every retained source-bearing fact must leave in the final domain.
- Discovery outputs, module preparation, header aggregation, semantic compilation, escaped
  diagnostics and resolved-reference tables see only final IDs. Those identities never change
  within their owning context. Cross-context import still requires explicit context ownership
  or canonical remapping under the interface contract.

### Source records

Registration, snapshot ownership and load failure are three arrays, not one row. Every candidate
gets a slot when its identity is assigned; only a candidate whose snapshot loads gets a record, and
only a candidate that fails to load occupies a failure row.

```rust
pub struct SourceSlot {
    pub id: SourceId,
    pub canonical_os_path: Option<Box<Path>>,
    pub logical_path: PathId,
    pub kind: Option<SourceKind>,
    pub provenance: SourceProvenance,
    pub(super) load: SourceLoadStatus,
}

pub struct SourceRecord {
    pub(super) text: Box<str>,
    pub(super) line_starts: Box<[u32]>,
    pub(super) extended_spans: Option<ExtendedSpanTable>,
}
```
This is the current field layout. A record exists only after a successful load; its `extended_spans`
table is installed at the final span-producing boundary.

This is the current split: registration metadata belongs to source slots, snapshots to loaded
records, and failure payloads to the database-owned cold array.

- a slot exists for every registered candidate, addressed directly by its `SourceId`.
- `logical_path` is the compiler-visible logical path.
- `canonical_os_path` exists only for filesystem-adjacent operations and is absent for synthetic
  sources.
- `load` is the candidate's lifecycle: identity assigned and nothing attempted, one loaded record,
  or one read failure. A failure belongs to the slot's lifecycle, but its payload does not: an
  `InfrastructureFailure` is wide and absent for every source in a successful build, so it lives in a cold
  database-owned failure array that the slot indexes. The transition out of the pending state
  happens exactly once per candidate, and both indexes stay private to the source database.
- a record is created only by a successful load, so it unconditionally owns its payload and has no
  absent or failed state of its own. Its index is private to the source database: a consumer
  addresses source text and spans through the `SourceId` on the slot.
- `text` is the exact UTF-8 source snapshot compiled.
- `line_starts` contains byte offsets into `text`. A snapshot with any content starts its first
  entry at `0`; an empty snapshot has no lines, so its table is empty rather than carrying a
  phantom entry every consumer would have to special-case.
- `extended_spans` owns exact ranges that do not fit inline in `LocalSpan`.
- `kind` stores `SourceKind::Compiler(SourceFileKind)` for compiler-recognized inputs or
  `SourceKind::ProviderOwned` for provider-owned files. It is `None` only for the reserved
  compilation root, which is not a file and has no lexical kind.
- `SourceKind` needs a provider-owned variant because the database is registered from Stage 0's
  whole sorted canonical inventory, which includes provider-owned physical files such as an
  external `.js` module. Those slots exist to hold an identity, not to be compiled, so they
  carry no compiler source kind and their extension stays in `canonical_os_path`.
- a candidate's kind states what the compiler recognizes, not what the active builder supports, and
  it is the kind of the authored spelling rather than of whatever the path resolves to. Registration
  is unconditional: an unsupported kind still gets an identity, and support is diagnosed where the
  source is referenced.
- `SourceProvenance` distinguishes `SourceProvenance::CompilationRoot` from
  `SourceProvenance::AuthoredPhysical`.

The source database takes ownership of source strings already loaded by Stage 0. It must not retain a
second full copy merely for diagnostics.

### Source size and complexity limits

Byte offsets are `u32`, so one source snapshot must be shorter than `u32::MAX` bytes. A physical
source exceeding that capacity is a typed user-facing source-size diagnostic, not a panic.
A compiler-produced synthetic source exceeding the documented bound is a compiler bug.

If one source requires more extended-span entries than the selected `LocalSpan` index field can
address, compilation emits a typed source-complexity diagnostic. It does not truncate, wrap or panic
because of user-authored input.

### Source snapshot consistency

Renderers, the dev server and future editor tooling resolve spans against retained `SourceRecord::text`.
They do not reopen a source file and accidentally render a newer disk version than the version that
was compiled.

Filesystem operations may still use `canonical_os_path`, but source excerpts and line/column
conversion always use the retained snapshot.

## Exact compact source spans

### Public representation

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalSpan(NonZeroU32);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    source: SourceId,
    local: LocalSpan,
}
```

Every span denotes the exact half-open byte range:

```text
[start, end)
end = start + length
```

Zero-length spans are valid. They are used for insertion points, EOF and file-level failures that
have a source identity but no wider authored range.

The underlying packed logical word is stored as `logical + 1` inside `NonZeroU32`. This leaves zero
as the `Option` niche. The logical value `u32::MAX` is reserved and may not be produced.

Callers never manipulate the packed word directly. Construction, decoding and validation stay in the
source-span module.

### Logical bit layout

`LocalSpan` uses the fixed 22-bit start/index and 10-bit length split asserted in
`source/span_encoding.rs`:

```text
31                                      LENGTH_BITS  LENGTH_BITS - 1            0
+--------------------------------------------------+-----------------------------+
|        inline start OR extended table index      |          length code        |
+--------------------------------------------------+-----------------------------+
```

Let:

```text
LENGTH_MASK     = (1 << LENGTH_BITS) - 1
LENGTH_SENTINEL = LENGTH_MASK
PAYLOAD_BITS    = 32 - LENGTH_BITS
```

Interpretation:

- `length_code < LENGTH_SENTINEL` means an exact inline span.
  - upper payload = exact start byte offset
  - lower code = exact byte length
- `length_code == LENGTH_SENTINEL` means an exact source-local extended span.
  - upper payload = zero-based `u32` index into the source-local extended-span table
  - the source record supplies exact `start` and `length`
- the all-ones logical word is reserved for niche encoding and cannot be used as an extended index

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExtendedSpan {
    start: u32,
    length: u32,
}
```

Resolving an inline span is arithmetic only. Resolving an extended span is one bounds-checked lookup
in the source-local extended-span table. No source scan, token scan, delimiter matching or parser
state is involved.

### Selected `LocalSpan` split

`LENGTH_BITS = 10` and `PAYLOAD_BITS = 22`: the high 22 bits carry either the inline start byte
offset or an extended-table index, and the low 10 bits carry the length code. Inline starts are
below 4 MiB and inline lengths are at most 1,022 bytes. Length code 1,023 marks an extended span.
The codec reserves the all-ones logical word and accepts extended indexes through 4,194,302.

The final corpus census counted 661,044 exact spans across 6,623 tokenized files, with 4,560,698
source bytes. It recorded 355 extended rows (2,840 bytes), at most 18 in one source, no start
overflow for any tested split from 8 to 12 length bits, and a maximum start of 226,191. Span
lengths had a median of 1 byte, p95 of 13, p99 of 25 and a maximum of 31,164 bytes. These results
support the current 22/10 split: it keeps the extended table small while preserving a 4 MiB inline
start range and exact encoding.

The constants and reserved-word conditions are compile-time assertions in
`src/compiler_frontend/source/span_encoding.rs`. The selected split is a current representation
choice. Reconsider it only with a new corpus census and the full architecture and performance gates.

### Extended-span insertion

Each mutable source record owns its extended-span builder. A source tokenizer or parser can append to
its own table without synchronization.

The default implementation is append-only. Deduplication is allowed only if measurement shows that
repeated extended ranges are common enough to offset the hash lookup and storage cost. Whether or not
entries are deduplicated is not observable through `LocalSpan`.

Construction returns a typed capacity error when the exact range cannot be encoded. The caller maps
that to the source-size/source-complexity diagnostic lane for authored input.

### Span API

The span module exposes named operations rather than public fields or raw bit functions:

```rust
impl LocalSpan {
    pub fn exact(start: u32, length: u32, extended: &mut ExtendedSpanBuilder)
        -> Result<Self, SpanCapacityError>;

    pub fn resolve_with(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange;
}

impl SourceSpan {
    pub fn source(self) -> SourceId;
    pub fn resolve_with(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange;
    pub(crate) fn byte_range<S: SourceSpanDatabase>(self, sources: &S) -> ResolvedByteRange;
}

impl ResolvedByteRange {
    pub fn start(self) -> u32;
    pub fn end(self) -> u32;
}
```
`SourceSpan::resolve_with` is retained for source tests; production consumers use `byte_range`.
Callers compare resolved byte ranges after confirming source identity. Spans from different sources do
not overlap.

### Line and column conversion

Canonical locations never store line or column numbers.

For rendering or tooling:

1. Resolve the exact byte range.
2. Binary-search `line_starts` for the containing line.
3. Slice the retained UTF-8 source text from the line start to the byte offset.
4. Count Unicode scalar values for terminal/source character columns.
5. Count UTF-16 code units only for an LSP-facing conversion.

The source database may later cache hot conversions, but caching is benchmark-selectable and cannot
change span representation. Newline handling must preserve the exact authored bytes, including CRLF.

The line-break set is the tokenizer's, not `str::lines()`: `\n`, `\r\n` and a bare `\r` each start
a new line, because a bare `\r` already ends a statement in the authored language. A line's visible
text drops its terminator, and an offset inside a terminator resolves to the visible end of the
line that terminator ends, so a caret cannot land on a byte that renders nowhere. A trailing
terminator closes its line without opening another, so the zero-width EOF of a file ending in one
resolves to the end of the last authored line rather than to a phantom line after it. An empty
snapshot has no lines and still resolves its EOF to line 0, column 0. Columns count scalar starts
before the offset, so an offset inside a scalar counts that scalar and no conversion slices on a
non-boundary offset.

## Genuine path interning

### Identity

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathId(NonZeroU32);
```

`PathId` represents one complete logical or semantic path in a build-lifetime path table. It does not
own path components.

`SourceId` and `PathId` are deliberately different:

- `SourceId` identifies a source snapshot.
- `PathId` identifies a canonical sequence of logical components.
- a source record contains its logical `PathId`.
- canonical operating-system paths remain cold source-database data.

Domain-specific wrappers may be used where source paths, semantic namespace paths or package paths
must not be mixed, but those wrappers remain four-byte IDs over the same table when their component
semantics are compatible.

An owner-relative logical resource path has two representations, and which one applies depends on
whether the path stays inside one build context:

- A build-local resource path is one such wrapper: a `PathId` over the same table, never an owned
  component vector and never a `PathBuf`.
- A stable cross-boundary resource origin is self-contained instead. `PathId` is a build-lifetime
  table-local identity, so it cannot carry meaning across an independent compilation context,
  a persisted artefact or a sidecar boundary. The owner-relative path inside a stable resource
  origin therefore uses a portable owned spelling, exactly as stable module and package identities
  already do. This changes only if a frozen cross-boundary path context is designed, at which point
  the owned spelling becomes an ID into that frozen context rather than into a build-lifetime one.

### Dense path table

The current representation is a parent-linked path trie:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PathNode {
    pub(super) parent: Option<PathId>,
    pub(super) component: StringId,
}

#[derive(Clone, Debug)]
pub struct PathInternerBuilder {
    table: PathTable,
    lookup_layers: Vec<Arc<FxHashMap<(PathId, StringId), PathId>>>,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}
```

Rules:

- `PathId::ROOT` is the root path.
- the root node's component field is not interpreted
- every non-root node names one parent and one component
- interning `(parent, component)` returns the existing child or appends one node
- complete path equality is `PathId` equality
- appending a component is one lookup
- rendering walks parents into reusable scratch storage, then reverses the components
- prefix checks walk ancestors or use depth-guided traversal
- no path operation clones a component vector

Path nodes currently use eight-byte array-of-structures `PathNode` rows. Revisit a structure-of-arrays
layout only if profiling identifies path traversal as a hotspot.

### Parallel path interning

Normal compilation does not put a lock around one mutable global path interner.

- The build context freezes an immutable base before a parallel wave.
- Each worker owns a `PathInternerFork` and may reference base paths plus worker-local paths.
- Deltas merge in canonical module and source order after the corresponding string-ID remap.
- Merge returns a compact `PathIdRemap`.
- Worker outputs are remapped before consumers observe them.
- Numeric `PathId` assignment never depends on worker completion order.
- After canonical publication, the project/package boundary freezes one final `PathTable` and shares it
  across completed module and generated-sidecar artefacts.
- A provider context retained across identity boundaries keeps its paired path/string tables as a
  justified cold subset and rebases requesters by spelling rather than numeric identity.

Source logical paths known at Stage 0 are interned before source IDs are assigned.

## Compact source-owned tokens

### Common token shape

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenTag(u16);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenShape {
    pub(crate) tag: TokenTag,
    pub(crate) flags: u16,
    pub(crate) data: u32,
}
```

`TokenTag` values are explicit internal constants. They are not inferred from Rust enum ordinals.
Reserved token flag bits must be zero.

Payload policy:

- punctuation, keywords and structural tokens use `data = 0`
- authored path tokens carry the canonical `TokenTag` plus one dense `PathSyntaxId` handle into
  the source-owned path syntax table
- numeric details and other complex lexical facts use a typed source-local side-store ID
- adding a new token family may not widen `TokenShape`

### Source-owned token store

```rust
pub struct SourceTokens {
    pub(super) source: SourceId,
    pub(super) shapes: Box<[TokenShape]>,
    pub(super) spans: Box<[LocalSpan]>,
    pub(super) numeric_literals: NumericLiteralStore,
    pub(super) path_syntax: Option<Arc<PathSyntaxTable>>,
    pub(super) sequence_store: TokenSequenceStore,
    pub(super) token_stats: TokenStats,
}
```

`shapes.len() == spans.len()` is a hard invariant. A token index addresses both arrays.

The builder emits the separate shape and span arrays in one pass. Freezing stores them as immutable
boxed slices. The current split representation retains the 8-byte `TokenShape` and 4-byte
`LocalSpan` independently.

`SourceTokens` is the immutable source-owned token model. The tokenizer schema in
`src/compiler_frontend/tokenizer/schema.rs` owns explicit `TokenTag` values, payload kinds, flags
and classification facts. `TokenShape` is eight bytes and `DiagnosticToken` is a separate
eight-byte projection over the same taxonomy.

`SourceTokens` stores boxed `TokenShape` and `LocalSpan` arrays, a `NumericLiteralStore`, an
optional shared `PathSyntaxTable`, a source-local `TokenSequenceStore` and token statistics.
`TokenIndex`, half-open `TokenRange`, `TokenSequenceId`, borrowed `TokenCursor` and `TokenRef`
provide checked views. Retained headers, declarations, templates and generic bodies keep ranges or
sequence handles instead of cloned token vectors.

`NumericLiteralStore` in `src/compiler_frontend/numeric_text/store.rs` retains lossless numeric
spelling and lexical facts until the receiving type is known. `PathSyntaxTable` in
`src/compiler_frontend/paths/path_syntax.rs` stores authored path roots and local spans. Both are
typed cold stores owned with source tokens; neither widens the common token shape.

The `SourceTokensBuilder` constructs shapes, spans and typed payloads during lexing. It validates
the final token count before mutation, then freezes one canonical owner. Token ranges and cold-store
handles are checked against that source owner before publication or use.

### Token references and ranges

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenIndex(pub(super) u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenRange {
    pub(super) source: SourceId,
    pub(super) start: TokenIndex,
    pub(super) end: TokenIndex,
}

#[derive(Clone, Copy, Debug)]
pub struct TokenRef<'a> {
    pub(super) tokens: &'a SourceTokens,
    pub(super) index: TokenIndex,
}
```

- ranges are half-open
- `TokenRef` borrows a canonical token; parser APIs do not clone complete tokens
- declaration shells, dependency clause shells and retained syntax store `TokenRange` or typed range wrappers
- no retained syntax structure owns a cloned token vector
- a range is valid only with the `SourceTokens` for its `SourceId`
- bounds violations in trusted internal records are compiler bugs
- malformed user source produces diagnostics before an invalid range can be constructed

### Path syntax table

One source-owned `PathSyntaxTable` owns each authored path in the source. Path tokens carry the
canonical `TokenTag` plus one dense `PathSyntaxId` handle.
Dependency selections are owned by retained dependency clauses, not by this path table.

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathSyntaxId(u32);

#[derive(Clone, Debug, Default)]
pub struct PathSyntaxTable {
    paths: Vec<PathSyntax>,
    owner_source: Option<SourceId>,
    frozen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSyntax {
    pub root: PathId,
    pub span: LocalSpan,
}
```

Rules:

- The test-only `PathSyntaxId::NONE` marker is zero and means no path row; it is never a valid
  identity.
- one authored path exists exactly once as one `PathSyntax` row
- a path token carries the canonical `TokenTag` for the path token family plus one dense
  `PathSyntaxId` in its shape data; that tag-plus-handle vocabulary is the only path-token
  vocabulary, there is no second stable path-token shape, and there is no inline path fast path
- token-driven consumers look up rows with a token-aware ownership check so a same-index handle
  from another source's table cannot be treated as valid
- dependency clause shape, selected-name order, aliases and source-versus-provider target
  classification are retained separately by the header dependency-clause owner
- the table is owned by the source-owned token store and freezes with it
- string-table remapping visits the table once for path roots
- source identity rebinding updates table locations once
- frozen generic bodies copy no path rows. Capture walks the retained canonical range with a
  `TokenCursor`, validates every donor path handle against the source-owned path table, and
  retains the matching resolved file-reference entries in the same pass so a materialised body
  reaches its content or resource target through stable facts instead of a second filesystem lookup
- generic donor identity is one `SharedDonorIdentity` per declaring domain, frozen once and shared
  across that domain's templates and every clone of the declaring preparation. `StableBodySyntax`
  carries the donor/source identity, frozen identity handle, canonical `StableBodyOwner`, resolved
  file-reference facts and the identity tables that issued payload IDs. `StableBodyOwner` retains
  only the canonical `SourceTokens`; filesystem identity is resolved through its sibling syntax
  fields and the `SourceDatabase`. Capture retains a frozen string owner, and a path table joins
  it when path roots must be rebased. The parser borrows the retained donor range and translates
  only consumed payloads through typed `TokenPayloadOrigin`; it never builds or retains a rebased
  token window. The body constructor is the single owner of the rule that a donor path table is
  invalid without its issuing string table.
- no path row owns a vector-backed path or a global source span when the enclosing source is
  already known
- path syntax rows stay syntax-only. Semantic resource identity, filesystem resolution, output
  placement and content state belong to their own owners and never widen a `PathSyntax` row
- structural file references are classified from these dense rows during source preparation. That
  classification reads the row and its spelling only: it adds no second tokenization, no second
  expression parse and no arbitrary source-text scan, and it stores its result beside the table
  rather than inside a row

### Numeric token side store

`NumericLiteralStore` in `src/compiler_frontend/numeric_text/store.rs` owns
the compiler's retained lexical records for authored numbers. Shared grammar
belongs to `moth-lexical`. The store retains the facts later semantic parsing
and diagnostics need and never widens the common token word:

- the authored spelling, kept losslessly, with whole-number and decimal or
  exponent spellings distinguishable until a destination is known
- the numeric lexical kind and the validation facts the shared `moth-lexical`
  grammar produced
- normalised digits, separator positions and exponent facts only where that
  grammar already owns them; the store must not re-derive them

Retained numeric text is cold source-local data. A token carries one dense
one-based `NumericLiteralId` handle, and counts and flags use fixed-width integer
fields, never `usize`.

The lexical store holds no materialised numeric value, selected destination type or semantic profile, and never routes through an `i32`/`f64` intermediate. Authored spelling, lexical category and the dense `NumericLiteralId` remain available until the receiving boundary can materialise the exact destination.

`NumericConstructionState` owns one construction domain per independent executable body. Dense origin/equality and binding tables retain eligible numeric constraints, literal fragments and narrow statement continuations, never placeholder `TypeId`s or fabricated declarations. Each binding row records its origin facts (known shape, annotation, reference view and raw or forward eligibility) once at declaration; an alias copies its source's facts, so origin queries never walk alias chains and the facts outlive recipe removal at completion. `numeric_known_shape.rs` stores shape nodes and parameter-to-shape edges by dense ID so collection, map, optional and declared generic structure can remain fixed while numeric leaves await resolution. Shapes form a shared DAG, so each traversal expands a shared node once. Declared-binding edges are kept in parameter-identity order. These IDs belong only to their issuing body domain.

Each exact receiving witness keeps its `SourceSpan` and authored-order key. An equality class retains only the witnesses its decisions read: the earliest receiver, the earliest receiver of a differing type and the earliest loop-ineligible or refused receiver. Registration and union maintain them by authored order, never by table traversal or union-root choice. Conflicting receivers report both witnesses in authored order. Literal handles and spans remain valid through body completion and diagnostic construction under their existing source-context lifetimes. This changes no packed span, token or diagnostic representation.

`statements/numeric_body_completion.rs` materialises each retained fragment once, upgrades pending scope slots and resumes continuations in authored order without reparsing statements or reconstructing argument routing. Nested blocks share the enclosing body domain. Independently nested functions and concrete generic bodies own separate domains.

Template TIR stores payload-free positions for retained numeric intakes. `templates/tir/numeric_intake.rs` owns their parsed fragments, receiving metadata and per-domain ordered intake queues. Body completion drains only its domain and installs completed expressions at those positions, without scanning sibling bodies or template subtrees. Completion releases pending constraints, shape tables and recipes before publishing a completed AST body. Diagnostic reports retain their existing source and type-display context, not construction state.

After materialisation, semantic, folded and public value owners carry their exact canonical type/value as appropriate; the lexical token store does not become a second type or value authority. The store remaps at its owner boundary, freezes with ordinary prepared-source publication and drops with its `SourceTokens` owner; no later stage re-reads it to recover a number whose semantic value another stage already owns.

### Diagnostic token projection

Diagnostics share the canonical `TokenTag` taxonomy but do not retain source-token side-store
ownership.

```rust
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DiagnosticToken {
    tag_and_flags: u32,
    data: u32,
}
```

Projection rules:

- static tokens preserve only their tag
- names and literals preserve the one compact ID needed for useful rendering
- a path normally projects to the path category and, only when useful, a `PathId`
- numeric literals preserve the authored text or kind needed by the diagnostic, not the complete
  tokenizer record
- a diagnostic must remain renderable after the source token side store is dropped or independently
  frozen

The separate projection reflects diagnostic retention needs: frozen diagnostics remain renderable
after the source-token owner and its cold stores are released.

## Compact diagnostic architecture

### Drafts, compaction and frozen report views

Diagnostics flow through one local draft model and one compact report model:

```text
compiler producer
-> DiagnosticDraft
-> DiagnosticBag
-> owning-boundary capture and canonical merge
-> DiagnosticRecord plus typed side stores in DiagnosticStore
-> DiagnosticReport / DiagnosticReportSet
-> DiagnosticView plus family-specific typed facts
-> terminal, terse, dev-server and tooling renderers
```

Producers construct typed `DiagnosticDraft` values and `DiagnosticBag` accumulates those drafts during stage-local work. Owning boundaries capture type-display facts and required identity owners before canonical merge. Freeze builds each report's compact `DiagnosticStore` and `TypeDisplaySnapshot`; `DiagnosticView` decodes stored facts through the family schema for rendering.

Foreign-site references are report-owned frozen identity-context references indexed by
`ForeignSiteRecord`. Producers create typed drafts; freezing writes the compact store and typed
side stores exposed through read-only `DiagnosticView`s.

`DiagnosticRecord` is the durable fixed record stored densely in `DiagnosticStore`.

```rust
#[derive(Debug)]
pub(super) struct DiagnosticRecord {
    pub(super) code: DiagnosticCode,              // 2 bytes
    pub(super) severity: SeverityOverride,        // 1 byte
    pub(super) primary: Option<SourceSpan>,       // 8 bytes; None when absent
    pub(super) facts: [u32; FACT_WORDS],           // 16 bytes; four words
    pub(super) extra: Option<DiagnosticExtraId>,  // 4 bytes
}
```

The common record is exactly 32 bytes. New diagnostic families must fit this shape.

### Diagnostic identity and flags

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticCode(NonZeroU16);
```

`DiagnosticCode` is an explicit internal numeric identity. It is assigned in the diagnostic schema
and never derived from enum declaration order. Its descriptor supplies the external stable code such
as `MOTH-RULE-0044`, category, title and default severity.

The Moth language's runtime `Error.code` is a separate `U32` value with default
`0`. It is a language value, never a `DiagnosticCode`, a compiler diagnostic
code, or the Rust MON `MonErrorCode` enum, and its width or default never drives
diagnostic storage or rendering.

`DiagnosticRecord` stores `code` and `severity` as separate fields. `SeverityOverride::Default` uses
the family descriptor's severity; non-default `Error`, `Warning` and `Note` variants are test-only.
The field widths sum to 31 bytes; one padding byte gives the fixed 32-byte record.

A diagnostic does not store category or default severity redundantly.

### Fact words

The four `u32` fact words are private storage. Compiler stages use typed constructors and accessors
generated or validated by the diagnostic schema.

Allowed fact values include:

- `StringId`
- `PathId`
- `TypeDisplayId` after compaction
- stage-local `TypeId` before compaction where the schema marks the fact for rewriting
- compact reason, context, operator, namespace and access-mode codes
- fixed-width counts and indexes
- two words forming one `u64`
- formally bounded packed pairs

Rules:

- no caller indexes `facts` directly
- no caller writes bit masks directly
- `usize` is converted through checked `u32` or formally bounded `u16` construction
- optional IDs use niche/sentinel codecs owned by the storage module
- two values may share one word only when their domains and overflow behaviour are documented and
  tested
- a constructor that cannot represent a value returns a typed construction/capacity error; it never
  truncates

Example schemas:

```text
TypeMismatch:
  fact 0 = expected TypeDisplayId
  fact 1 = found TypeDisplayId
  fact 2 = TypeMismatchContext
  fact 3 = unused/zero

InvalidGenericInstantiation (ConflictingInference):
  fact 0 = InvalidGenericInstantiationCode reason
  fact 1 = GenericInferenceSubject
  fact 2 = existing TypeDisplayId
  fact 3 = replacement TypeDisplayId
  extra  = parameter name followed by the optional type name, as StringIds

```

The schema tests validate that unused words and reserved bits are zero.

A family whose reasons need different word layouts declares one family-local shape codec rather than
widening the record. The codec writes the reason code into word 0 and reports, for each value, which
words hold a type. The family row records the words that must hold a type and the words that may; a
draft's pending type-slot marks must lie between the two.

### Diagnostic draft

```rust
pub struct DiagnosticDraft {
    core: DraftCore,
    extra: Option<Box<DiagnosticExtraDraft>>,
}
```

`DraftCore` is `Copy`. It holds the family row, the span, the fact words, the pending type-slot marks,
the fault flag and the severity override. `compact` writes the code, span, facts and severity into the
stored record.

The exact private representation may use another one-pointer rare-data owner, but it must remain no
larger than 48 bytes on supported 64-bit targets.

- no `Clone` implementation is provided
- common diagnostics allocate nothing
- rare variable data allocates at most one root draft object; its owned lists may allocate as needed
  before compaction
- draft facts use the producing worker's current identity domain
- deterministic remapping happens before the draft becomes durable
- draft destruction after compaction releases all temporary allocation
Returning an owning pointer for every draft or diagnostic is not part of the compact plain
diagnostic boundary. Keep common diagnosed failures as values and reserve indirection for typed
rare data that the schema explicitly places in a side store.

### Diagnostic IDs and store

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticId(u32);
```

```rust
#[derive(Debug)]
pub(super) struct DiagnosticStore {
    pub(super) records: Vec<DiagnosticRecord>,
    pub(super) extras: Vec<DiagnosticExtraRecord>,
    pub(super) labels: Vec<SecondaryDiagnosticLabel>,
    pub(super) label_data: Vec<[u32; 2]>,
    pub(super) string_ids: Vec<StringId>,
    pub(super) substitutions: Vec<GenericSubstitutionRecord>,
    pub(super) foreign_sites: Vec<ForeignSiteRecord>,
    pub(super) limits: DiagnosticLimits,
    pub(super) full: Option<DiagnosticCapacityResource>,
}
```

The exact list of typed cold stores evolves with the schema. Common rules do not:

- `DiagnosticId` indexes one dense record
- record order is deterministic compiler production order after canonical worker merge
- each auxiliary list uses a typed `start: u32, len: u32` range
- no per-record `Vec`, `HashMap` or trait object is stored
- no renderer mutates or enriches a diagnostic record

### Capacity-exhaustion policy

Compact indexes have explicit limits. User-authored input that exceeds one is not a compiler bug.

- The schema includes the no-extra `COMPILER_CAPACITY_EXCEEDED` diagnostic family with a compact
  resource-kind fact.
- Diagnostic store builders reserve logical room for one terminal capacity record.
- Deltas merge in canonical order. On the first capacity failure, the builder keeps every already
  compacted record, appends one capacity diagnostic at the triggering span (or the compilation-root
  span), rejects the unrepresentable draft and stops accepting later drafts for that compilation.
- The behaviour is deterministic and never wraps, truncates a field or recursively allocates more
  extra data.
- Source-span table exhaustion aborts preparation of that source and emits the corresponding
  source-complexity/capacity diagnostic at the compilation-root or last exactly representable span.
- Source, path, token-side-store and type-display capacity errors follow the same rule at their
  owning boundary.
- A capacity failure caused by a compiler-produced bounded data structure is a compiler bug only when
  the producing stage's documented invariant proves user input could not cause it.

The `COMPILER_CAPACITY_EXCEEDED` schema row currently assigns stable code `MOTH-SYNTAX-0037`.

### Extra-data indirection

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticExtraId(NonZeroU32);
```

Records store it as `Option<DiagnosticExtraId>`, so `None` means no extra data. Other values are one-based indexes.

The baseline extra record is:

```rust
#[derive(Clone, Copy, Debug)]
pub(super) struct DiagnosticExtraRecord {
    pub(super) labels: DiagnosticListRange,         // start: u32, len: u32
    pub(super) foreign_sites: DiagnosticListRange,  // start: u32, len: u32
    pub(super) data: ExtraDataRef,                  // kind: u32, then a DiagnosticListRange
}
```

`ExtraDataKind::None` means the diagnostic has no data list. `ExtraDataKind::StringIds` names a run of
the string-id arena. One extra record can point at its labels, foreign sites and string list together,
so an unusual diagnostic does not force a second word into every common record.

A foreign site is an 8-byte `ForeignSiteRecord`: a selector word (0 for the primary span, related-site index + 1 for a related site) and an index into the report-owned frozen identity-context references. The span stays in the record's primary field or in its related-site record, so the row never carries a span. Equal frozen identities share one entry.

Before creating a new extra kind, the owner must check whether the diagnostic can carry a smaller,
clearer semantic fact instead. Side storage is an escape hatch for genuinely variable or unusual
data, not permission to preserve bloated payloads unchanged.

### Primary and secondary labels

The primary source span is `DiagnosticRecord::primary`. It is not duplicated as a label.
A label may carry a source span, a message, both or neither. A diagnostic with no secondary labels
has no label allocation.

```rust
#[derive(Clone, Copy, Debug)]
pub(super) struct SecondaryDiagnosticLabel {
    pub(super) span: Option<SourceSpan>,
    pub(super) packed: u32,
}
```

`packed` uses this layout:

```text
bits 24..31  LabelMessageCode; 0 is NONE, used only for a label with no message
bits  0..23  zero-based row index into label_data when the code carries data; zero for codes without data
```

The label schema decides which codes carry data. Codes without data use a zero row index; codes with
data interpret the low bits only as a zero-based `label_data` row index, never as an immediate value.

A substitution list does not live inside the label-message enum. It uses the substitution arena and a
label data reference.

The compact store limits `label_data` to at most `2^24 - 1` rows. Exceeding that limit is a typed
diagnostic-capacity failure for user input, not integer wrapping.

A label with a span and no message is stored as code 0 (`NONE`) with zero data. It keeps its span, so
its location still renders.

### Mixed-domain ownership

The frozen identity context carried by a report is the normal identity domain for
both `DiagnosticRecord::primary` and `SecondaryDiagnosticLabel::span`. A compact
span without cold ownership data is interpreted only in that domain. Numeric
source IDs or paths never infer a different owner.

A rare primary or secondary site from another identity domain has typed cold
ownership data in the same report. Its `ForeignSiteRecord` selects the primary
or related site and indexes a report-owned reference to the other
`FrozenIdentityContext`. The span remains in the common record or related-site
record; equal contexts share one reference.

The common `DiagnosticRecord` remains exactly 32 bytes and
`SecondaryDiagnosticLabel` remains exactly 12 bytes. Neither common type carries
an `Arc`, path object or context pointer to implement this ownership. Foreign
related-site ownership is separate from the report's default render ownership
and must never change which normal context resolves the primary span.

Cross-domain sites use explicit report-owned context references. Compact IDs never determine an
owner by themselves.

### Diagnostic places

`DiagnosticPlace` is the typed draft value:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticPlace {
    Local(StringId),
    Path(PathId),
    Unknown,
}
```

`PlaceFact` encodes it in one `u32` fact word:

```text
bits 30..31  place tag: 00 unknown, 01 local StringId, 10 PathId, 11 invalid
bits  0..29  zero-based StringId or PathId index
```

`OptPlaceFact` reserves an absent marker distinct from `DiagnosticPlace::Unknown`. Values that do
not fit the 30-bit index field return `FactCapacityError`; the storage module owns construction,
remapping and decoding.

### Variable lists

Variable diagnostic lists use typed ranges into dense arenas:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticListRange {
    start: u32,
    len: u32,
}
```

`DiagnosticListRange` is the shared range shape; the owning arena determines the element type. The
current store has no per-arena list-wrapper types.

## Single diagnostic schema authority

Every user-facing diagnostic family is declared once in an internal declarative schema.

The schema owns:

- `DiagnosticCode`
- external stable code
- category
- title
- default severity
- semantic name
- the type and meaning of each fact word
- optional/packed fact codecs
- the one permitted extra-data schema, if any
- permitted secondary-label roles
- renderer entry point
- typed draft constructor
- typed durable accessor
- type-display rewrite markers
- test/example registration

Illustrative schema DSL only; this is not a Rust declaration or a literal current schema row:

```text
diagnostic_schema! {
    TypeMismatch {
        code: 17,
        stable: "MOTH-TYPE-0001",
        category: Type,
        default_severity: Error,
        facts: {
            expected: TypeId => TypeDisplayId,
            found: TypeId => TypeDisplayId,
            context: TypeMismatchContext,
        },
        extra: None,
        labels: [],
        renderer: render_type_mismatch,
    }
}
```

The first implementation uses `macro_rules!` plus const tables. It must not add a procedural macro,
build script or external code-generation language.

The schema generates or validates:

- the numeric code table
- descriptor lookup
- typed constructors
- typed fact accessors
- allowed extra-kind checks
- renderer dispatch
- all-code iteration for tests
- duplicate internal and external code detection
- reserved-word/bit validation
- render coverage

Direct construction of `DiagnosticRecord`, raw fact indexing and direct renderer matches on storage
bits are prohibited outside the diagnostic storage/schema modules.

Compaction treats a secondary label whose role the family row does not declare as a compiler bug and
panics, naming the family and the role. Four roles are universal and need no declaration: `NONE` for a
message-less label, and the three generic-instantiation roles, which the generic-function machinery
appends to any diagnostic.

A procedural generator remains deferred unless the declarative schema becomes demonstrably
unmanageable. “More convenient” is not sufficient evidence.

## Diagnostic type-display snapshots

### Purpose

Durable diagnostics do not retain complete module `TypeEnvironment` values. Before the environment
can be dropped, the diagnostic compaction boundary captures only the transitive facts needed to
render referenced types.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TypeDisplayId(NonZeroU32);
```

### Display store

The baseline compact record is:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DiagnosticTypeRecord {
    pub(super) tag: DiagnosticTypeTag,
    pub(super) flags: u16,
    pub(super) data_0: u32,
    pub(super) data_1: u32,
}
```

Typed side ranges supply variable arguments and choice-variant summaries.

The store and its record layout live in `diagnostic_types/type_display_snapshot.rs`, and the builder
lives in `diagnostic_types/type_display_snapshot_builder.rs`. One formatter in
`datatypes/display.rs` renders spellings for both the live and snapshot paths, so user-visible
text stays identical.

The store must represent every current user-visible type spelling:

- builtins
- structs and const records
- choices, including variant names and whether a variant has payload
- options
- growable and fixed collections
- ordered maps
- ordinary aggregate types that remain real semantic values
- callable parameters, zero/one/multiple success slots and the independent error channel
- generic parameters
- concrete generic nominal instances
- external types with useful package/name identity
- unknown external types only where the existing semantic model explicitly permits them

Callable result shape is a signature fact, not a synthetic tuple or fallible-carrier `TypeId`.
Display records preserve slot order, arity and each slot's type separately from the error channel.
They must not reconstruct removed transport types merely to render a diagnostic. Genuine aggregate
types still use their normal type-display records.

### Snapshot algorithm

For each diagnostic draft produced with a module-local `TypeEnvironment`:

1. Inspect the schema and collect every fact marked as a source `TypeId`.
2. Reserve a `TypeDisplayId` for each newly visited type before recursing, so recursive display graphs
   cannot loop.
3. Copy only display-relevant facts into `TypeDisplaySnapshotBuilder`.
4. Recursively intern referenced child types and typed lists.
5. Rewrite the draft fact from `TypeId` to `TypeDisplayId`.
6. Validate that every type-marked fact was rewritten.
7. Freeze the type display store before rendering.
8. Release the complete `TypeEnvironment` when no semantic artefact owner still needs it.

The builder memoizes by source `TypeId` inside one compilation context, so each type is encoded
once and recursive graphs terminate. It does not merge distinct `TypeId`s that share a spelling,
such as two generic parameters both named `T`, because each `TypeId` keeps its own identity. It
does not claim cross-build type identity.

A renderer receiving an unknown `TypeDisplayId` or an unrewritten source `TypeId` is observing a
compiler invariant violation and invokes `compiler_bug!`. It does not fall back to printing an
internal numeric ID.

### Rendering equivalence

Rendering-equivalence tests must compare live and snapshot formatting for every supported type shape and representative nested combinations. The snapshot store preserves user-visible spelling unless a diagnostics change explicitly authorises different wording.

The module-local `TypeEnvironment` can be released once semantic artefact owners no longer need it.
Frozen report views render rewritten type facts through `TypeDisplayId`s.

## Frozen render context and cloning policy

A diagnostic report owns its compact `DiagnosticStore` and `TypeDisplaySnapshot` and shares one lookup-only frozen identity context. `DiagnosticView` borrows those stores to decode family facts. See `Compilation identity context` for the exact split.

A graph-level `DiagnosticReportSet` may carry several module/package reports when they belong to
independent identity contexts. It does not concatenate raw IDs without preserving their context owner.

Rules:

- `StringTable::clone()` is not used to snapshot every failure boundary.
- a finalized string table is shared immutably
- source, string and path lookups share the frozen identity context's lifetime; diagnostic and
  type-display stores belong to the report, not to that context
- renderers borrow the frozen context and the report's stores
- successful warnings and failed diagnostics use the same record/store model
- a report is move-only; a host that genuinely shares one wraps the whole report or set in `Arc`,
  never a per-record or per-side-store `Arc`
- stage-local mutable bags are consumed, not cloned
- no context is kept alive by hidden global state

The existing local fork/delta string-table strategy may be retained and improved. This design does
not require a globally contended string interner.

## MON data handoff

This section owns the MON snapshot, cursor, span and owned-result handoff.
The standalone codec owner lives in `crates/moth-mon/src/`: `reader.rs` owns
the caller-borrowed cursor, `schema.rs` owns preparation and validation,
`budget.rs` owns allocation accounting and `error.rs` owns the public error
projection. `lib.rs` exposes `moth_mon`, with the same API and Rust types
deliberately re-exported as `moth::mon` from `src/lib.rs`.
Rust API signatures and routine type shapes belong to the codec's doc comments.

Compiler-retained numeric text stays in the compiler's numeric token side store.
The MON reader borrows original spelling from caller input and retains a bounded
owned normalised string until the schema selects a destination.
The prepared schema records exact receiver contracts and retains each explicit
width's range/precision limits. It stores no semantic numeric profile and has
no `Schema::with_profile` selection.
Its explicit targets are `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32`, `U64`,
`F16`, `F32` and `F64`, alongside the `Byte` octet shape and the `SchemaType::Decimal { scale }`
contract with validated scales from 0 through 256.
`SchemaType::Integer` is a MON receiver contract, not the removed Moth source
`Int`; lossless `Integer` and `Decimal` values remain owned MON data, not source
type aliases. Shared `moth-lexical` materialises directly to the selected
receiver without a second parser or binary-float intermediate. The buffer's
decoded-byte accounting appears below. Compiler diagnostic storage keeps its
compact architecture, and MON never becomes source compilation.
`docs/src/docs/mon/mon-format.mtf` owns the literal-data format.

### Caller-owned input snapshot and cursor lifetime

The caller owns the UTF-8 input text and its lifetime. Decoding accepts either a
borrowed `&str`, already valid UTF-8, or a byte slice through
`decode_document_bytes`, which reports invalid UTF-8 through the public error
projection instead of the compiler diagnostic lanes. The MON cursor borrows that
input only for the duration of the call. It may borrow internally while parsing,
but no borrow escapes: decoded values outlive and release the input text without
a caller-retained backing buffer, and the default public API has no borrowed
document view.

Input accounting runs before the affected allocation or expansion under checked
arithmetic. Defaults are 1 MiB input bytes, depth 64, 100,000 nodes, 4,096
numeric digits, 16 MiB decoded bytes, 16 MiB output bytes and 10,000 default
expansions. Configurable `max_depth` is capped at `Limits::MAX_SAFE_DEPTH`
(`64`), with larger policies rejected during schema preparation. Accepted
recursion stays bounded so the cursor and owned-tree destruction remain within
the documented depth. Limits are receiver resource policy, not grammar
dialects: a valid document that exceeds a receiver's budget fails distinctly
from syntax and schema errors.

The cursor never constructs `SourceId`, `PathId`, `StringId`, `SourceDatabase`,
AST, HIR or compiler cursors. It performs no file IO, module import, schema
discovery, command execution, source-kind registration, builder registration,
output write, backend path or runtime evaluation. It uses no compiler identity
table: there is no build-lifetime source database, no path interner, no string
table and no frozen identity context on this path. Root elision consumes the
existing cursor with original offsets rather than a copied string with synthetic
parentheses.

### MON spans without compiler identity

`Span` is an exact half-open UTF-8 byte range over the caller-owned input, never
a `SourceSpan` or `LocalSpan` and never an index into compiler-owned source
storage. It carries no path, line or column pair and no syntax-aware end recipe,
matching the durable-span rule that spans stay exact byte ranges. Absence uses
`Option<Span>`, never a magic valid value. Reserved distinctions in the public
error stay semantic codes rather than packed record bits; this section adds no
layout assertion and changes no locked size.

Diagnostic rendering of a `Span` resolves against the caller's retained input,
not against a build-lifetime source database or frozen identity context.
Releasing the input text after a successful decode is the ordinary pattern; a
later render of a retained error needs the input the span was measured against.
Line and column conversion, where a caller wants it, happens at that caller's
rendering boundary and is never stored in the span.

### Retained numeric text and local escape decoding

Whole-number and decimal or exponent spellings stay distinguishable through a
lossless numeric literal representation until the destination is known. The
shared lexical owner in `moth-lexical` supplies grammar, normalisation,
separator/exponent validation and destination-aware materialisation. MON
performs only the bounded literal conversion, range and scale checks the codec
needs and adds no second arithmetic runtime. Compiler-retained numeric text stays
in the source-owned numeric token side store, while MON's bounded normalised
buffer is private to one call and never becomes a compiler token or source record.
Exact `Integer` and `Decimal` values retain their text without routing through
`f64` or display formatting. Explicit binary-float receivers use the shared
numeric authority and reject non-finite source values and conversion results,
emitting `-0.0` for negative zero and `0` for positive zero while decoding
preserves the sign bit.

Materialisation for an explicit fixed-width receiver, `Byte` or `SchemaType::Decimal { scale }` uses
that schema's exact receiving limits and the shared `moth-lexical` policies.
There is no profile-dependent `Int`/`Uint`/`Float` variant or
`Schema::with_profile`; `SchemaType::Integer` remains a MON contract rather than
the removed source `Int`. Whole-number and decimal/exponent categories stay
strict: decimal/exponent spelling cannot satisfy an integer or `Byte` receiver
even when its value is integral. Dec conversion remains exact at its selected
scale.

Numeric normalisation storage is included in the logical `max_decoded_bytes`
budget. The reader charges the unsigned token byte length before
`parse_numeric_literal` reserves that capacity, including for malformed
tokens. Programmatic `Integer`/`Decimal` values and defaults charge the same
scratch before parsing. Explicit-width reader values materialise from the
normalised facts. Exact reader `Integer` and `Decimal` values transfer the
normalised buffer into the result instead of cloning it; a negative sign is
charged before it is inserted. Schema validation charges its caller-owned
output clone separately from normalisation scratch.
Each schema-supplied name is charged once by byte length when the schema is
prepared. Input-derived names are charged before the reader or validator copies
them into an owned path segment or error detail, and this includes rejected
names: a reserved label, qualifier or variant, or a reserved schema field,
reports `DecodedBudget` instead of copying a spelling that exceeds the remaining
budget. Borrowed name recognition that copies nothing is not charged.
Reader and schema validation share one allocation-free borrowed effective-scale
rule; each path keeps its own span, error code and detail context. These are
logical byte charges against `max_decoded_bytes`, not exact allocator-resident
memory measurements: allocator rounding, excess capacity and deallocation
timing are not represented. Bounded codec policy and checked arithmetic do not
promise process-wide out-of-memory recovery or a custom allocator.

MON string and character escapes are decoded by a bounded literal reader owned
by the MON format; no Moth source escape owner is shared. MON's Unicode
`\u{H...}` form accepts one to six ASCII hex digits. Moth source remains limited
to its existing five escapes. Writer output uses uppercase hex; unescaped
newlines and CRLF remain byte-for-byte content because MON is data, not Moth
source. The local decoder adds no source-language change.

### Immutable prepared schema and owned result

The caller builds an immutable `Schema` once and shares the validated
`PreparedSchema` across calls. Preparation validates the static schema and its
prevalidated default data before any document is processed; schema misuse is a
structured failure, never a panic. Preparation rejects a transitive
`SchemaType::Unsupported` member before any document is processed; there is no
public unsupported value. Defaults apply at the exact missing field only, with
no deep merge and no replacement of a supplied `none`. Typed encoding emits all
completed fields in schema declaration order. Public `encode_document` and
`encode_value` emit compact output; pretty formatting is private to writer tests,
not a public mode.

Results are owned trees with no alias or allocation-identity promise: repeated
shared source values encode at each occurrence, and decoding preserves no alias
relationship. The public `Value` model cannot represent cyclic host data. Writers
are deterministic without canonical-byte semantics. Every public encode/decode
operation publishes only a complete successful result: decoding discards partial
values, encoders build a private `String` and discard it on error, and no caller
buffer or partial public result exists on the guaranteed path.

### Public error context beside compiler diagnostics

The public error is `MonError { code: MonErrorCode, span: Option<Span>, path:
Vec<PathSegment>, detail: String }` with no compiler identity table. Its codes
keep input, schema, resource, budget and internal lanes distinct: UTF-8,
literal and syntax, schema and type, duplicate and qualifier and arity, numeric
and Unicode, resource, each budget kind and internal invariant failures each
have their own codes. A missing field points at its containing record rather
than fabricated source text; duplicate map-key errors point at the duplicate
key. Messages render only at the caller's diagnostic boundary.

`MonError` implements `Display` and `std::error::Error` for ordinary Rust error
propagation. Its owned code, detail and value path remain usable after input
release. Source excerpts still require the original input for its byte span.

This public projection stays beside the compiler failure architecture. It does not create a
`DiagnosticRecord`, side store, `InfrastructureFailure` or `compiler_bug!` site, and adds no
competing compiler error taxonomy. Encoding and decoding return the first structured `MonError`
with its stable reason, original input byte range when decoding, and field or element path where
available.

### What this handoff does not use or change

- No `SourceDatabase`, `SourceId`, `PathId`, `StringId`, `PathSyntaxTable`,
  token store, diagnostic store, `TypeEnvironment` or frozen identity context.
- No AST, HIR, TIR, borrow fact, lifetime fact, link fact or module artefact.
- No source tokenizer cursor, header preparation, interface binding, declaration
  ordering, constant folding, template evaluation or backend lowering.
- No locked layout decision: `LocalSpan`, `SourceSpan`, `TokenShape`,
  `DiagnosticRecord` and every hard representation invariant keep their exact
  sizes, ownership and failure-lane contracts.
- No later diagnostic storage work: compact diagnostic records, side stores and
  the failure lanes keep their owners and their deferred-work list.
- No build profile, project configuration or compiler token store: MON schema preparation receives caller-supplied exact receiver contracts, stores no semantic profile and reads no command, builder or source-owned numeric record. There is no `Schema::with_profile` API.

## Failure architecture

Moth compiler failures have exactly three lanes. They are not converted into one another merely
for rendering convenience.

Runtime program `Error` values, implicit checked failures and entry-startup failures sit outside
these three compiler lanes. They use the language's execution contract, not `DiagnosticRecord`,
`InfrastructureFailure` or `compiler_bug!`. Statically invalid numeric work and undischarged
exported failure are source diagnostics. A runtime failure does not turn a completed compilation
into a diagnosed module.

### Lane 1: user-caused diagnostics

User source, project structure, configuration, dependency clauses, type errors, rule violations, borrow errors,
target-contract failures, warnings and deliberate deferred-feature messages use typed
`DiagnosticDraft` values.

Properties:

- recoverable and accumulable
- exact `SourceSpan` wherever source exists
- stable diagnostic code and structured facts
- no panic caused by malformed or adversarial user input
- no infrastructure error variant inside the diagnostic schema
- a diagnosed semantic module exposes no partial public interface

### Lane 2: operational infrastructure failures

Expected environmental and host failures use a separate typed result:

```rust
pub struct InfrastructureFailure {
    pub(crate) kind: InfrastructureFailureKind,
    pub(crate) stage: CompilerStage,
    pub(crate) host_path: Option<Box<Path>>,
    pub(crate) message: Box<str>,
    pub(crate) details: Box<[InfrastructureDetail]>,
}
```

`InfrastructureFailureKind` distinguishes internal, HIR transformation, LIR transformation, Wasm
generation, filesystem and development-server failures. `CompilerStage` records the owning
compiler or host stage, and `InfrastructureDetail` retains only typed primary suggestions and
output-rejection reasons.

An infrastructure failure retains no source span or compact compiler identity. `host_path` is
ordinary owned host-path context, and a missing path is valid for internal or host failures. No
source identity context or cause-to-source pairing is needed.

Examples:

- source or config file cannot be opened (filesystem path and read suggestion)
- output cannot be written (filesystem path and output-rejection reason where applicable)
- permission is denied (filesystem path and OS detail in the message)
- a dev-server port cannot bind (host failure)
- file watching fails (host failure)
- a backend tool or registered provider fails (host/tool failure)
- target runtime resources are unavailable (host failure)
- an invalid host path cannot be represented by the compiler's source identity policy
  (filesystem path or a pathless infrastructure failure)

Properties:

- recoverable by an owning CLI/tooling boundary where retry or a clear exit is possible
- not stored in `DiagnosticRecord`
- no conversion to a diagnostic payload
- typed deterministic details instead of a string-keyed `HashMap`
- carries no source span, `SourceId`, `PathId` or `StringId`
- can be rendered in terminal, terse and dev-server forms through a separate infrastructure
  renderer
- never widened to include arbitrary subsystem objects

An operational failure is not a compiler bug merely because the user cannot repair it in source.

### Lane 3: compiler bugs

A proven compiler invariant violation uses a non-recoverable panic produced by `compiler_bug!`.

```rust
#[track_caller]
#[cold]
pub(crate) fn raise_compiler_bug(
    stage: CompilerStage,
    source: Option<CompilerBugSource>,
    message: fmt::Arguments<'_>,
) -> !;
```

The panic payload is a structured `CompilerBugReport` containing:

- `stage: CompilerStage`
- `location: &'static Location<'static>` for the Rust caller's file, line and column
- `version: &'static str`
- `message: Box<str>`
- `source: Option<CompilerBugSource>`

`CompilerBugSource` owns `logical_display_path`, `byte_range`, `start_line`, `start_column`,
`end_line`, `end_column` and a bounded `excerpt`. `CompilerBugSource::from_span` resolves those
facts while the frozen identity context is available. The report retains no `SourceSpan` and
does not consult compiler tables after unwind; the renderer adds the fixed report request and URL.

Valid panic examples:

- validated HIR references a missing local or block
- a supposedly remapped compact ID is outside its frozen table
- a diagnostic schema accessor observes the wrong extra-data kind
- a trusted source token range is out of bounds
- an AST-to-HIR path receives a semantic state an earlier validated stage guarantees impossible
- a frozen store is mutated or internally inconsistent

Invalid panic examples:

- malformed source
- unsupported source syntax or target feature
- missing file or permission failure
- invalid project configuration
- backend capability rejection
- network/provider/tool invocation failure
- source too large for a documented compact representation

Compiler bug panics should become rarer as invariants are moved earlier and tested. Panic is the final
signal that continuing the owning compilation would be untrustworthy, not a substitute for typed
validation.

### Owning result boundary

Owning compilation boundaries use one three-lane result. User diagnostics and infrastructure
failures remain distinct, and compiler bugs still panic:

```rust
pub type CompilationResult<T> = Result<CompilationOutcome<T>, InfrastructureErrors>;

pub enum CompilationOutcome<T> {
    Success {
        value: T,
        warnings: Option<DiagnosticReportSet>,
    },
    Diagnosed(DiagnosticReportSet),
}
```

`Success` owns a completed value and warnings frozen by the command/report owner after the
project `SourceDatabase` has reached its last use. The warning set is `None` when empty. Inner
success boundaries return `warnings: None` and stage successful batches in the single
command-owned `BoundaryAccumulator`; they do not freeze or clone source databases early.
`build_project` returns an intermediate `BuildResult` retaining the project source database and
staged warning owner until output writing is complete. `BuildResult::finish_output` consumes that
intermediate state: success freezes the warnings once and releases the database, while a late
output failure discards staged warnings and freezes the output diagnostic against the retained
source. `check` and Boracle freeze at their own report boundary; the dev-server build owner waits
until output and site-configuration checks finish. Output-cleanup warnings remain plain
`OutputWriteSummary` data and are rendered separately by the build CLI and dev-server success
owners.

`Diagnosed` owns only the frozen reports that explain the user failure. The outer `Err` owns the
aggregate of every infrastructure cause; when infrastructure failure and user diagnostics meet at
an owning boundary, infrastructure wins and the diagnostic set is discarded. A diagnosed or
infrastructure-failed result never carries unrelated successful warnings. Compiler bugs are not a
`Result` variant.

The compiler may retain successful independent graph branches while diagnosed providers block
only their dependants. The owning frontend boundary turns the completed graph into `Success` or
`Diagnosed`; it does not stop independent work early or expose a partially linkable project.

### Current diagnostic and failure transport

Stage-local code prepares typed `DiagnosticDraft` values and `PreparedDiagnosticBatch` objects.
Inner boundaries stage successful warnings in the command-owned warning store; the command/report
owner freezes reports into `DiagnosticReportSet` after all source-database use and maps the result
to `CompilationOutcome`. Expected operational failures and uncertain internal failures use the
outer `InfrastructureErrors` lane, whose ordered primary and additional causes preserve production
order. Appending an aggregate flattens causes and keeps the first cause primary without merging
their fields.

`BoundaryAccumulator` retains source databases and frozen identities for prepared diagnostic
reports and staged successful warnings. Infrastructure causes need no source owner, source-context
range or identity reconciliation, and the memory probe does not count cause-source owners.

#### Current failure presentation

One owner under `compiler_messages/render/infrastructure.rs` derives presentation exhaustively from
`InfrastructureFailureKind`. `Internal`, `HirTransformation`, `LirTransformation` and
`WasmGeneration` identify compiler/build-system defects. `Filesystem` and `DevelopmentServer`
identify operational failures. Classification follows the cause, not the subsystem or message text.
Every failure carries a typed `CompilerStage` and every view renders a `Stage: <stage name>`
guidance line. Authored configuration and target capability mistakes remain typed diagnostics,
not either introduction.

The shared compiler-defect introduction precedes the unchanged original detail:

```text
COMPILER BUG: Moth encountered an internal compiler or build-system error.
This is a Moth toolchain defect, not a source-code error. Please report it at:
https://github.com/nyejames/moth/issues

Details:
<original failure message>
```

Operational failures use:

```text
OPERATIONAL FAILURE: Moth could not complete a filesystem, host or tooling operation.

Details:
<original failure message>
```

All three views use that shared classification, introduction and guidance policy. Rendering leaves
stored messages unchanged, retains actionable host paths and OS/process details, renders typed
primary suggestions, and performs no network lookup or repository scan. A third-party operation
failure alone supplies no evidence of a Moth defect. Infrastructure failures have no source frames
or cause-to-source pairing.

| View | Contract |
|---|---|
| Terminal | Show every cause in order, with separately labelled additional causes and a neutral filesystem heading for `Filesystem`. |
| Dev-server HTML | Keep all causes in one infrastructure card with `data-diagnostic-code="MOTH-INFRA-0001"`. Escape all error-derived text and use the fixed clickable report URL above for compiler defects. |
| Terse | Emit one `E\|MOTH-INFRA-0001\|path\|\|message` record with exactly five fields. Label and classify every cause in order. The path field belongs only to the primary cause, with additional paths inside their cause sections. Sanitisation removes delimiter pipes and physical newlines. Include the fixed report URL once per record containing a compiler defect. |

An infrastructure result represents one aborted compilation: it returns the ordered aggregate and
discards coexisting diagnostics. A diagnosed result carries only its error reports; successful
warnings are rendered only from `Success::warnings`, never beside a failed result. Command failure
status remains non-zero, and successful warning counts and presentation remain available to hosts.

#### Lane classification

Every failure site belongs to exactly one lane:

- user-caused state is a diagnostic schema entry
- expected operational state, and internal state not yet proven impossible, is `InfrastructureFailure`
- a proven invariant is `compiler_bug!`, citing the guard that makes it unreachable from any authored, layout, provider or builder input

There is no automatic conversion between lanes. Promoting an `InfrastructureFailure` to `compiler_bug!` requires the guard proof; a user-reachable path becomes a diagnostic with an integration case. Infrastructure data never enters the compact diagnostic model.

## Tooling host isolation

A compiler bug is non-recoverable for its owning compilation, but a long-lived host may remain alive
when each request runs in fresh, isolated worker state.

### CLI

The CLI's top-level `build` and `check` boundaries, plus the feature-gated Boracle command boundary,
catch compiler-bug unwinds, render the structured report once and exit with distinct compiler-bug
status 2. They never turn a compiler bug into a successful or ordinary diagnosed build.

A `test_compiler_bug` Cargo feature exposes a deliberate structured-panic trigger only to the
opt-in CLI subprocess lane; ordinary builds contain no trigger, and there is no CLI or environment
switch. Subprocess tests cover diagnosed and infrastructure outcomes, check/build compiler-bug status
2, unknown payload delegation without repeating the prior hook's output, and Boracle's feature-gated
compiler-bug exit. The integration runner uses its per-case worker boundary and returns only frozen
reports and strings, never a `BuildResult`.

### Dev server and future LSP

Each build request creates one fresh worker on the calling thread. Its owned immutable request
contains an absolute entry path, flags, an owned `BuildConfigInputSet`, and the resolved output root
and write policy. The worker owns the fresh `ProjectBuilder`, providers and all mutable compiler,
source and diagnostic state. It returns only after coherent finalization: success with frozen
warnings, a diagnosed result with frozen reports, an infrastructure result, or a compiler-bug report.
An unknown panic payload becomes a generic compiler-bug report. No unfinished build result or
diagnostic/warning accumulator crosses back to the host.

The dev host catches unwind only around the fresh worker request. It drops the entire worker after a
failure; the next request always creates a fresh one. No compiler or output writer runs while
host-state locks are held. After successful compilation and output finalization, the worker writes
outputs directly under the resolved policy, without a staging directory. The host snapshots served
artifact bytes outside the state lock, then briefly locks to publish the completed generation with
its matching metadata and snapshot. A diagnosed, infrastructure, compiler-bug, output-write or
snapshot failure publishes a failure state/page while retaining the last known-good served
artifacts, even if a failed output write partially changed files in the output directory. The
watcher remains active and later builds use fresh workers. A poisoned host-state lock is a host bug,
not state to recover.

This owned request and result boundary can be adopted by a second long-lived host without compiler
API changes. Restart/backoff and cancellation remain future host policy, not additional compiler
failure lanes.

A compiled program returning `Error` from `start` is not a compiler diagnosis, infrastructure failure
or compiler bug. The dev host may present that invocation-local outcome without changing global build
success, poisoning shared compiler state or giving other entries a compile-error page. The report
protocol lives in `docs/build-system-design.md` > `Development entry-error reports`.

### Panic strategy

Long-lived hosts use unwind behavior so they can catch a structured `CompilerBugReport` at the
owning worker boundary and discard that worker's state. The CLI and development server do not
recover poisoned compiler state. Process-isolated workers remain deferred, so current hosts do not
depend on child-process abort behavior.

## Deterministic parallel construction

Compact data does not justify nondeterministic IDs.

For each parallel wave:

1. The build system establishes canonical source/module order.
2. The compilation context freezes immutable base tables.
3. Each worker receives final `SourceId`s and local builders for strings, paths, diagnostics and
   other append-only facts.
4. Workers return deltas plus records expressed in their local identity domains.
5. Deltas merge in canonical order.
6. Merge produces explicit remap tables.
7. Records are remapped exactly once.
8. Diagnostics are appended in canonical production order, not completion order.
9. The next consumer sees only remapped identities.

Source spans need no remapping within a final identity domain. Inventory-backed workers receive
final IDs before tokenization. Traversal-only discovery normalizes its provisional facts once
before this boundary, as specified in `Source identity and database > Private discovery finalization`.

No compact ID is allocated through a timing-dependent global atomic simply because the numeric type
is cheap.

## Performance evidence and acceptance

The compiler's frontend instrumentation and benchmark tooling own memory measurements. The
`data_layout_memory_probe` feature records source-token owners, typed-store bytes and capacities,
transient construction buffers, remap tables and source-owner live/peak bytes. Summarize relevant
results in `benchmarks/frontend-optimization-results.md`; keep raw allocator logs, profiler captures
and per-run data uncommitted.

Before considering a representation change, compare the current representation and candidate with
the same toolchain and corpus. Record:

- size and alignment for every affected record
- source count and retained source bytes
- token count and retained token bytes
- span start and length distributions
- path and syntax counts, clone/remap pressure and string-table clones
- diagnostic, payload, label, reason and variable-list sizes
- full `TypeEnvironment` values retained solely for diagnostics
- peak retained frontend bytes or a repeatable allocation proxy
- compile/check wall time for representative success-heavy and diagnostic-heavy workloads

Use representative workloads covering docs build/check, the focused frontend suite, path-heavy and
token-heavy projects, short and long template spans, generic/type-heavy sources, warning-heavy
success, diagnostic-heavy malformed input, long-line Unicode/CRLF text, and sources beyond each
candidate inline-start threshold. Diagnostic stress measurements supplement normal correctness
tests; they do not own stable diagnostic codes, spans or output.

Run five independent benchmark invocations and compare medians. Use focused profiles when results
are ambiguous. A median wall-time regression greater than 5% in docs, focused frontend or a targeted
workload blocks the change unless an explicitly accepted tradeoff is documented. Reject or revert a
measured optimization that adds substantial complexity without a repeatable memory or throughput
gain. Counter movement and layout reduction alone are insufficient; source, span and token changes
must reduce retained frontend memory on representative projects, while diagnostic changes must
reduce failure-path retention without boxing the common diagnostic.

No speculative percentage target replaces hard layout contracts, but the evidence must show a
material benefit for the affected owner.
## Validation and anti-drift tests

### Layout tests

The `Hard representation invariants` table names every asserted record size and its owning source
module or test. Keep each compile-time or unit-test assertion beside the type it constrains; do not
centralize layout assertions away from their owners.

### Span property tests

Property/table tests cover:

- every boundary around the inline length sentinel
- every boundary around the inline start limit
- zero-length spans
- final byte of a source
- extended-table lookup
- reserved all-ones logical value
- `Option` niche behaviour
- overflow-table capacity errors
- exact round trip for generated ranges
- Unicode, CRLF and multiline line/column conversion
- deterministic sorting and overlap semantics
- malformed/unclosed syntax without span reconstruction

### Token tests

Tests cover:

- every token tag and payload policy
- reserved flag validation
- token shape/span length equality
- simple path records
- numeric side data
- range bounds
- source-owned retained declaration syntax
- no token cloning required by parser APIs
- deterministic token output across serial and parallel preparation

### Diagnostic tests

Tests cover:

- every schema entry has a unique internal and external code
- every schema entry constructs, compacts, accesses and renders
- fact words and reserved bits validate
- permitted/forbidden extra kinds
- common diagnostics allocate no extra record
- primary span is not duplicated in labels
- secondary labels preserve order and messages
- type facts are fully rewritten to `TypeDisplayId`
- rendering equivalence across terminal, terse and dev-server surfaces
- malformed store access triggers `compiler_bug!`
- no infrastructure failure enters the diagnostic store
- no diagnostic record contains target-width `usize`

### Failure-lane tests

Tests distinguish:

- user malformed input returns a diagnosed outcome without panic
- filesystem/tooling/backend operational failures return `InfrastructureFailure`
- proven invariant fixtures panic with a structured `CompilerBugReport`
- dev-server worker isolation discards failed state and keeps host state consistent
- the CLI bug path terminates through the bug report
- no poisoned compiler-state recovery remains

### Full gates

Run the repository's documented validation lanes for code changes. Representation changes also
require focused owner tests, layout/property coverage, formatting and repeatable memory and timing
evidence. Keep the runnable command and resource guidance in the developer validation policy.

## Extension rules

### Adding a source-location consumer

A new consumer receives `SourceSpan` or `LocalSpan` with an explicit source owner. It must not store a
path, line/column tuple or source text copy beside the span.

### Adding a token family

A new token family must:

- receive an explicit `TokenTag`
- define the meaning of flags and `data`
- use a typed source-local side store if one `u32` is insufficient
- update token projection, schema tests and memory accounting
- keep `TokenShape` at 8 bytes

### Adding a diagnostic family

A new diagnostic family must:

- be declared once in the schema
- reuse or receive an explicit stable code
- fit four fact words
- use at most one schema-approved extra-data shape
- simplify its semantic facts before requesting side storage
- identify its secondary-label roles
- add constructor/accessor/render coverage
- keep `DiagnosticRecord` at 32 bytes

### Adding an infrastructure failure

A new operational failure receives a typed `InfrastructureFailureKind` and typed detail records. It
must not create a diagnostic payload or panic.

### Adding a compiler bug check

A new `compiler_bug!` site must name the proven invariant and the stage that promised it. A focused
invariant test should demonstrate the check where practical. “Unexpected input” is not sufficient.

## Deferred capabilities

These capabilities are outside the current architecture. The optional memory-layout reviews remain
deferred because final-corpus evidence did not meet their entry conditions. Reconsider an item only
when its stated evidence exists.

| Capability | Re-entry criterion |
|---|---|
| Process-isolated compiler workers | A separate reliability design is warranted by evidence that owned thread workers cannot meet crash-isolation, cancellation or untrusted-input requirements. |
| Persistent `SourceId`, `PathId`, token, diagnostic and type-display identities | An artifact format defines canonical identities and remapping; process-local IDs are never serialized directly. |
| Procedural-macro or build-script diagnostic schema generation | The current declarative schema becomes demonstrably unmanageable, and generation preserves exhaustive compile-time checks without disproportionate build complexity. |
| Terminator-match span encoding | A candidate is accepted only if decoding returns an exact range without scanning source bytes or tokens; it depends only on an immutable retained match table, not parser or tokenizer state; malformed and unclosed source has a deterministic exact fallback; strings, escapes, comments, templates and nested balanced modes require no duplicate lexical rules; source ordering, overlap checks and LSP conversion remain constant-time after at most one table lookup; the terminator match table plus any fallback spans consumes at least 10% less retained memory than the exact overflow-table design on the measured corpus; median compile time and diagnostic rendering time do not regress by more than 2%; and the implementation is simpler or equally auditable at every consumer boundary. Failure of any condition rejects the experiment. A durable span must never store an instruction that asks a renderer to reparse source. |
| Token records smaller than 8 bytes | Profiles show `TokenShape` is a common-memory hotspot and full-store measurements show a net benefit after indirection and access costs. |
| Broad conversion of `StringId`, `TypeId` and other compiler IDs | Each proposed ID domain has a formal bound and measured material retained-memory pressure, with deterministic merge/remap behavior preserved. |
| Snapshot compression or memory mapping | Retained source bytes are a measured top memory owner, and a separate approved design defines the platform, ownership and lifetime policy and demonstrates a net benefit. |
| Persistent/incremental source databases, token reuse and line-index caches | Source-hash ownership and incremental invalidation are specified, and representative workloads show repeatable savings. |
| Full LSP protocol implementation | A dedicated tooling design defines protocol scope, request scheduling, cancellation and diagnostic ownership over source snapshots, owned workers and frozen reports. |
| Diagnostic deduplication across independent compilation contexts | Repeated frozen reports are a material cost and stable identity scope, equality and invalidation are defined. |
| Cross-build diagnostic and type-display caches | Reuse across builds is measured as material, with stable content keys and invalidation rules. |
| Path-node compression or AoS/SoA layout changes | Representative traversal profiles identify path-node layout as a bottleneck, and end-to-end measurements include merge and remap costs. |
| Extended-span deduplication | Repeated retained overflow ranges are common enough to offset lookup and storage overhead. The current corpus retains 355 extended rows (2,840 bytes, 0.054% of spans), which does not justify the extra index. |
| Hot/cold `SourceRecord` split | Profiles show cold fields materially inflate hot source access, and a split improves retained bytes or access cost without adding lifetime or ownership complexity that outweighs the gain. |
| Token cold-store packing | A named token cold store has formal capacity bounds, and representative aggregate retained-memory measurements show that packing materially reduces total retained memory after indexing and access costs. |
| Diagnostic extra/list packing | A specific extra or list store is a material retained-memory owner, and measured savings exceed indexing/access costs while common diagnostics remain allocation-free. |
| Other path, token or diagnostic cold-store packing | A named store is a material retained-memory owner in a representative profile; measured aggregate savings exceed indexing, indirection and construction costs. |
| Unrelated diagnostic wording and suggestion changes | A scoped diagnostics improvement defines the semantic behavior, expected output and owning tests before implementation. |

## Current implementation map

The following paths are the current owners:

```text
src/compiler_frontend/source/
    mod.rs
    id.rs
    record.rs
    registration.rs
    database.rs
    span.rs
    span_encoding.rs
    line_index.rs
    frozen_identity.rs
    test_support.rs
    tests/

src/compiler_frontend/symbols/path_interner/
    mod.rs
    id.rs
    builder.rs
    fork.rs
    frozen.rs
    remap.rs
    test_exhaustion.rs
    tests.rs

src/compiler_frontend/tokenizer/
    mod.rs
    schema.rs
    tokens.rs
    storage.rs
    cursor.rs
    stream.rs
    lexer.rs
    line_scanning.rs
    newline_handling.rs
    numeric.rs
    text_modes.rs
    tests/

src/compiler_frontend/numeric_text/
    mod.rs
    parse.rs
    token.rs
    store.rs
    tests/

src/compiler_frontend/paths/
    mod.rs
    compile_time_paths.rs
    path_syntax.rs
    path_resolution.rs
    path_normalization.rs
    dependency_resolution.rs
    module_roots.rs
    file_references.rs
    resource_identity.rs
    module_resources.rs
    const_paths/
        mod.rs
        components.rs
    tests/

src/compiler_frontend/compiler_messages/
    mod.rs
    compiler_errors.rs
    compiler_bug.rs
    compiler_stage.rs
    compiler_dev_logging.rs
    diagnostic_bag.rs
    diagnostic_descriptor.rs
    diagnostic_identity.rs
    diagnostic_severity.rs
    diagnostic_token.rs
    display_messages.rs
    prepared_batch.rs
    trait_keyword_diagnostics.rs
    tests/
    diagnostics/
        mod.rs
        boundary_accumulator.rs
        compaction.rs
        diagnostic_string_builder.rs
        draft.rs
        facts.rs
        extras.rs
        foreign.rs
        freeze.rs
        labels.rs
        prepared.rs
        reasons.rs
        reason_keys.rs
        record.rs
        registry.rs
        report.rs
        resource.rs
        schema.rs
        store.rs
        test_support.rs
        view.rs
        families/
        tests/
    diagnostic_types/
        mod.rs
        id.rs
        type_display_snapshot.rs
        type_display_snapshot_builder.rs
        tests/
    render/
        mod.rs
        borrow.rs
        calls.rs
        context.rs
        control_flow.rs
        declarations.rs
        dev_server.rs
        import_config.rs
        infrastructure.rs
        paths.rs
        payload.rs
        suggestions.rs
        syntax.rs
        templates.rs
        terminal.rs
        terse.rs
        tests/

src/compiler_frontend/instrumentation/
    mod.rs
    ast_counters.rs
    frontend_counters.rs
    memory_ledger.rs
    tests.rs

src/bin/data_layout_memory_probe.rs

crates/moth-mon/src/
    lib.rs
    reader.rs
    writer.rs
    schema.rs
    numeric.rs
    model.rs
    map_keys.rs
    budget.rs
    error.rs
```

`SourceDatabase` owns source records, snapshots and line indexes. `SourceTokens` owns immutable
shape/span arrays and typed source-local stores. Path interning owns canonical `PathId` identities;
authored path syntax remains in each source's `PathSyntaxTable`. Core pipeline modules orchestrate
these owners, while codecs, schemas, capacity rules and memory accounting stay in their respective
submodules.

The MON service borrows caller input only while parsing and returns owned values through its public
error projection. It owns no compiler source database, identity table, token store, diagnostic store
or frozen context.

## Current architecture summary

- Exact half-open UTF-8 spans use the asserted 22-bit start/index and 10-bit length encoding.
- `TokenShape` is 8 bytes; `SourceTokens` stores shapes and `LocalSpan`s in separate dense arrays.
- `DiagnosticRecord` is 32 bytes, and schema-approved typed side stores own unusual diagnostic data.
- User mistakes produce diagnosed outcomes; operational failures use `InfrastructureFailure`;
  proven compiler invariant violations use the structured compiler-bug path.
- Owned workers isolate mutable compilation state. Source snapshots and frozen diagnostic reports
  retain the identity context needed by their consumers without sharing live compiler state.
