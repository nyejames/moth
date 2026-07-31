# Moth Compiler Data Layout Design

> **Repository path:** `docs/compiler-data-layout-design.md`
>
> **Status:** Accepted end-state architecture. Implementation is deliberately scheduled after the
> Compiler Test Suite Hardening plan and before further user-facing diagnostic improvement work.
>
> **Initial repository audit anchor:** `d119988861aad9732c19d945eeabeb249a7e5caa`
> (`docs: start explicit expectation enforcement`). The implementation plan must replace this anchor
> with the final accepted Test Suite Hardening commit before code changes begin.

## Authority and ownership

This document is the canonical authority for the physical representation, ownership, lifetime and
extension rules of Moth compiler source metadata, retained tokens and diagnostics.

It owns:

- build-lifetime source identity and source snapshots
- exact byte-based source spans
- compact local and global span encodings
- true logical-path interning
- source-owned retained token storage
- compact token records and typed token side stores
- compact diagnostic drafts, records, labels, facts and side stores
- diagnostic schema declaration and validation
- reduced type-display snapshots for diagnostics
- build-then-freeze compiler identity contexts
- the boundary between user diagnostics, operational infrastructure failures and compiler bugs
- layout assertions, measurement requirements and anti-drift rules for these systems

It does not override:

- `docs/compiler-design-overview.md` for compiler stage and semantic ownership
- `docs/build-system-design.md` for Stage 0, graph scheduling, project tooling and host orchestration
- `docs/language-overview.md` for source-language behaviour
- the memory-management authorities for Moth program semantics
- `docs/src/docs/progress/@page.moth` for current implementation status
- `docs/roadmap/roadmap.md` and the owning implementation plan for sequencing

Where an implementation slice changes a boundary shared with one of those authorities, both
documents must be updated in the same accepted slice. A roadmap plan cannot silently override this
document.

## Change-control policy

The design deliberately distinguishes locked architecture from measured implementation choices.
This prevents benchmark work from becoming an excuse to weaken the original goals.

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
- diagnostic families are declared through one schema authority.
- user mistakes, operational infrastructure failures and compiler bugs are three separate failure
  lanes.
- only proven compiler invariant violations may panic.
- complete `TypeEnvironment` values are not retained solely to render diagnostics.
- normal compilation and rendering contexts are shared or moved, not deep-cloned.
- process-local compact IDs are not persistent artefact identities.

### Benchmark-selectable decisions

An implementation agent may change the following only after running the required bounded
experiment, recording the result and updating this document in the same slice:

- the final start/length bit split inside `LocalSpan`
- compact array-of-structs versus struct-of-arrays token storage
- the exact internal record shapes of uncommon token and diagnostic side stores
- whether extended spans are deduplicated within one source
- whether simple paths receive an inline token fast path
- a terminator-match encoding, but only if it meets every stop/go rule in this document
- additional packing inside already fixed-width records when every encoded domain has a formal,
  tested bound

Benchmark-selected changes may not alter the locked sizes, exactness, ownership or failure-lane
contracts.

### Repository-discovered corrections

File names, module boundaries and migration order may be updated after the implementation worker
refreshes the repository. Those corrections must keep one current implementation path, remove
superseded owners and update the implementation map in this document.

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

| Type | Required size | Required properties |
|---|---:|---|
| `SourceId` | 4 bytes | non-zero build-lifetime ID, `Copy` |
| `LocalSpan` | 4 bytes | exact source-local byte range, `Copy` |
| `Option<LocalSpan>` | 4 bytes | niche-encoded absence |
| `SourceSpan` | 8 bytes | `SourceId` plus `LocalSpan`, `Copy` |
| `Option<SourceSpan>` | 8 bytes | niche-encoded absence |
| `PathId` | 4 bytes | non-zero path-table identity, `Copy` |
| `Option<PathId>` | 4 bytes | niche-encoded absence |
| `TokenShape` | 8 bytes | fixed tag, flags and one `u32` payload |
| `DiagnosticToken` | 8 bytes | compact diagnostic projection of a token |
| `DiagnosticCode` | 2 bytes | explicit non-zero internal code |
| `DiagnosticExtraId` | 4 bytes | zero means no extra data |
| `SecondaryDiagnosticLabel` | 12 bytes | global span plus compact message/data word |
| `DiagnosticRecord` | 32 bytes | fixed common durable diagnostic record |
| `DiagnosticDraft` | at most 48 bytes | move-only common record plus rare draft indirection |

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

## Current-to-target architecture

| Current shape | Target shape |
|---|---|
| `SourceLocation { InternedPath, start line/column, end line/column }` | `SourceSpan { SourceId, LocalSpan }` |
| source paths cloned into each token and diagnostic | one path and source record in shared tables |
| `Token { TokenKind, SourceLocation }` | source-owned `TokenShape` plus `LocalSpan` |
| `TokenKind::Path(Vec<PathTokenItem>)` | fixed token payload plus typed path-group side store |
| declaration shells clone `Vec<Token>` | `TokenRange` into immutable source-owned tokens |
| `InternedPath(Vec<StringId>)` | `PathId` into a dense path trie/table |
| wide `CompilerDiagnostic` and `DiagnosticPayload` | small `DiagnosticDraft`, 32-byte `DiagnosticRecord`, side stores |
| primary location also cloned into first label | primary span in the record; side store contains secondary labels only |
| full `TypeEnvironment` retained for rendering | minimal immutable `DiagnosticTypeStore` |
| deep-cloned `StringTable` in message containers | frozen shared compilation render context |
| `CompilerError` mixes bugs and expected infrastructure failures | `InfrastructureFailure` plus `compiler_bug!` invariant panic |

## Compilation identity context

One project or package compilation boundary owns one identity context.

```rust
pub struct CompilationContextBuilder {
    sources: SourceDatabaseBuilder,
    strings: StringTableBuilder,
    paths: PathInternerBuilder,
    diagnostic_types: DiagnosticTypeStoreBuilder,
    diagnostics: DiagnosticStoreBuilder,
}

pub struct FrozenCompilationContext {
    sources: FrozenSourceDatabase,
    strings: FrozenStringTable,
    paths: FrozenPathTable,
    diagnostic_types: FrozenDiagnosticTypeStore,
    diagnostics: FrozenDiagnosticStore,
}
```

Exact Rust names may change. The ownership rules may not.

- Mutable builders have one explicit owner.
- Compiler APIs borrow only the tables they can extend or inspect.
- Workers use local append-only deltas rather than mutating shared global tables.
- Deltas merge in canonical source or module order.
- Worker-owned records are remapped once before a later consumer can observe them.
- The completed context freezes into immutable storage.
- Build results and diagnostics that outlive compilation retain an `Arc<FrozenCompilationContext>`
  or an equivalent single shared owner.
- Bulk owning stores are move-only until frozen.
- Compact IDs and records may be `Copy`.
- Broad `Clone` implementations on message sets, diagnostic bags, source databases and owning render
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

`SourceId` identifies one source snapshot inside one `FrozenCompilationContext`.

Rules:

- `SourceId(1)` is a deterministic synthetic compilation-root record with empty source text. It gives
  project-wide user diagnostics an exact context-owned primary span before or outside any physical
  source. It is not a fake filesystem path and its provenance is `CompilationRoot`.
- Physical and other synthetic IDs begin after that root record.
- IDs are assigned deterministically before tokenization begins.
- Physical sources are sorted by canonical logical source order, never filesystem iteration or
  worker completion order.
- Compiler-known synthetic sources are registered in a deterministic category and owner order before
  source IDs freeze.
- A source adapter may produce synthetic token content, but it must retain provenance to the authored
  source record when diagnostics should point to authored bytes.
- No parallel stage may allocate final `SourceId`s from a shared atomic counter.
- If a future stage genuinely discovers new synthetic source after the initial registration barrier,
  it must return a deterministic source delta that is merged before any persistent record using that
  ID escapes the stage. Late IDs may not leak directly from workers.
- `SourceId` values are not serialized. Persistent artefacts store canonical source identity plus
  exact byte ranges and remap into a new context.
- Absence uses `Option<SourceSpan>`, never `SourceId(0)` or a fabricated source.

### Source records

```rust
pub struct SourceRecord {
    logical_path: PathId,
    canonical_os_path: Option<Box<Path>>,
    text: Box<str>,
    line_starts: Box<[u32]>,
    extended_spans: Box<[ExtendedSpan]>,
    kind: SourceKind,
    provenance: SourceProvenance,
}
```

The final record may split hot and cold fields into parallel arrays after measurement. Its semantics
are fixed:

- `logical_path` is the compiler-visible logical path.
- `canonical_os_path` exists only for filesystem-adjacent operations and is absent for synthetic
  sources.
- `text` is the exact UTF-8 source snapshot compiled.
- `line_starts` contains byte offsets into `text`; the first entry is always `0`.
- `extended_spans` owns exact ranges that do not fit inline in `LocalSpan`.
- `kind` identifies Moth, Moth template, Markdown, config or another registered source kind.
- `provenance` distinguishes authored physical source from synthetic or adapted source and points to
  its owning source where needed.

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

`LocalSpan` uses one fixed split selected during the implementation's span-census phase:

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
  - upper payload = zero-based `ExtendedSpanIndex`
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

### Final bit-split selection

The architecture has a deterministic selection procedure rather than an unresolved design choice.
The implementation phase must evaluate these candidates over the committed representative corpus:

| Length bits | Inline start range | Inline maximum length |
|---:|---:|---:|
| 8 | below 16 MiB | 254 bytes |
| 9 | below 8 MiB | 510 bytes |
| 10 | below 4 MiB | 1,022 bytes |
| 11 | below 2 MiB | 2,046 bytes |
| 12 | below 1 MiB | 4,094 bytes |

For each candidate, record:

- total spans
- inline spans
- start-overflow spans
- length-overflow spans
- combined extended spans
- maximum extended entries in one source
- estimated extended-table bytes
- span construction time
- span resolution time for representative rendering and tooling queries

Selection rules, in order:

1. Reject a candidate that cannot index the maximum observed extended-span count with at least a
   four-times safety margin.
2. Reject a candidate that makes a representative source impossible to encode exactly.
3. Choose the candidate with the fewest total extended spans across the weighted corpus.
4. If candidates differ by less than 0.1% of total spans, choose the candidate with the larger inline
   start range.
5. If still tied, choose the 22/10 split (`LENGTH_BITS = 10`).
6. Record the chosen constants and evidence in this document and in the benchmark report before the
   old location model is removed.

The initial design default is therefore 22 start/index bits and 10 length bits. Benchmark evidence
may select another listed split, but it may not invent an unreviewed format.

### Extended-span insertion

Each mutable source record owns its extended-span builder. A source tokenizer or parser can append to
its own table without synchronization.

The default implementation is append-only. Deduplication is allowed only if measurement shows that
repeated extended ranges are common enough to offset the hash lookup and storage cost. Whether or not
entries are deduplicated is not observable through `LocalSpan`.

Construction returns a typed capacity error when the exact range cannot be encoded. The caller maps
that to the source-size/source-complexity diagnostic lane for authored input.

### Terminator-based encoding experiment

A terminator recipe such as “continue until the next balanced template close” is not part of the
canonical format. It may be prototyped once during the span-selection phase because the design goal is
aggressive memory efficiency.

It is accepted only if all of the following are true:

- decoding returns an exact range without scanning source bytes or tokens
- it depends only on an immutable retained match table, not parser or tokenizer state
- malformed and unclosed source has a deterministic exact fallback
- strings, escapes, comments, templates and nested balanced modes require no duplicate lexical rules
- source ordering, overlap checks and LSP conversion remain constant-time after at most one table
  lookup
- the terminator match table plus any fallback spans consumes at least 10% less retained memory than
  the exact overflow-table design on the measured corpus
- median compile time and diagnostic rendering time do not regress by more than 2%
- the implementation is simpler or equally auditable at every consumer boundary

Failure of any condition rejects the experiment. The result is recorded as a deliberately deferred
or rejected optimisation in the roadmap. A durable span must never store an instruction that asks a
renderer to reparse source.

### Span API

The span module exposes named operations rather than public fields or raw bit functions:

```rust
impl LocalSpan {
    pub fn exact(start: u32, length: u32, extended: &mut ExtendedSpanBuilder)
        -> Result<Self, SpanCapacityError>;

    pub fn resolve(self, source: &SourceRecord) -> ResolvedByteRange;
    pub fn is_empty(self, source: &SourceRecord) -> bool;
}

impl SourceSpan {
    pub fn source(self) -> SourceId;
    pub fn byte_range(self, sources: &SourceDatabase) -> ResolvedByteRange;
    pub fn start(self, sources: &SourceDatabase) -> u32;
    pub fn end(self, sources: &SourceDatabase) -> u32;
    pub fn overlaps(self, other: Self, sources: &SourceDatabase) -> bool;
    pub fn contains(self, other: Self, sources: &SourceDatabase) -> bool;
}
```

Comparing ranges from different sources is never an overlap. Deterministic display ordering compares
`SourceId`, then start, then end. The current location `PartialOrd` behaviour must be replaced by
explicitly named operations so overlap and ordering cannot be confused.

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

### Dense path table

The baseline representation is a parent-linked path trie:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct PathNode {
    parent_raw: u32,
    component: StringId,
}

pub struct PathInternerBuilder {
    nodes: Vec<PathNode>,
    depths: Vec<u32>,
    lookup: FxHashMap<(PathId, StringId), PathId>,
}
```

Rules:

- `PathId(1)` is the root path.
- the root node's component field is not interpreted
- every non-root node names one parent and one component
- interning `(parent, component)` returns the existing child or appends one node
- complete path equality is `PathId` equality
- appending a component is one lookup
- rendering walks parents into reusable scratch storage, then reverses the components
- prefix checks walk ancestors or use depth-guided traversal
- no path operation clones a component vector

A structure-of-arrays variant may be selected after profiling, but it must preserve one compact ID
and one canonical node per unique path prefix.

### Parallel path interning

Normal compilation does not put a lock around one mutable global path interner.

- The build context freezes an immutable base before a parallel wave.
- Each worker owns a `PathInternerDelta` and may reference base paths plus worker-local paths.
- Deltas merge in canonical module and source order.
- Merge returns a compact `PathIdRemap`.
- Worker outputs are remapped before consumers observe them.
- Numeric `PathId` assignment never depends on worker completion order.

Source logical paths known at Stage 0 are interned before source IDs are assigned.

### Migration rule

`InternedPath` may exist only as a temporary migration adapter inside one not-yet-migrated slice. It
must not remain in durable source spans, tokens, diagnostics, type definitions or public interfaces at
plan completion. No compatibility wrapper may preserve `InternedPath(Vec<StringId>)` as a second
canonical path representation.

## Compact source-owned tokens

### Common token shape

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenTag(u16);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenShape {
    tag: TokenTag,
    flags: u16,
    data: u32,
}
```

`TokenTag` values are explicit internal constants. They are not inferred from Rust enum ordinals.
Reserved token flag bits must be zero.

Payload policy:

- punctuation, keywords and structural tokens use `data = 0`
- symbols, directives and ordinary string-like literals may store a compact ID directly
- simple unaliased paths may store a `PathId` directly when the measured fast path is retained
- numeric details, grouped paths and other complex lexical facts use a typed source-local side-store
  ID
- adding a new token family may not widen `TokenShape`

### Source-owned token store

```rust
pub struct SourceTokens {
    source: SourceId,
    shapes: Box<[TokenShape]>,
    spans: Box<[LocalSpan]>,
    numeric_literals: NumericLiteralStore,
    path_groups: PathGroupStore,
    token_stats: TokenStats,
}
```

`shapes.len() == spans.len()` is a hard invariant. A token index addresses both arrays.

During construction, vectors may be used with measured capacities. Freezing converts them to boxed
slices or another immutable dense owner. An array-of-structs experiment may place shape and local
span together, but only if it materially outperforms the split representation and still preserves the
8-byte/4-byte contracts.

### Token references and ranges

```rust
#[repr(transparent)]
pub struct TokenIndex(u32);

#[derive(Clone, Copy)]
pub struct TokenRange {
    source: SourceId,
    start: TokenIndex,
    end: TokenIndex,
}

pub struct TokenRef<'a> {
    tokens: &'a SourceTokens,
    index: TokenIndex,
}
```

- ranges are half-open
- `TokenRef` borrows a canonical token; parser APIs do not clone complete tokens
- declaration shells, import shells and retained syntax store `TokenRange` or typed range wrappers
- no retained syntax structure owns `Vec<Token>`
- a range is valid only with the `SourceTokens` for its `SourceId`
- bounds violations in trusted internal records are compiler bugs
- malformed user source produces diagnostics before an invalid range can be constructed

### Path token side stores

A simple ungrouped path should use the token's own `LocalSpan` and direct `PathId` payload when the
fast path survives profiling.

Grouped paths and aliases use source-local stores:

```rust
#[repr(C)]
struct PathGroupRecord {
    item_start: u32,
    item_count: u32,
}

#[repr(C)]
struct PathTokenItemRecord {
    path: PathId,
    path_span: LocalSpan,
    alias: PathAliasId,
    flags: u32,
}

#[repr(C)]
struct PathAliasRecord {
    name: StringId,
    span: LocalSpan,
}
```

`PathAliasId(0)` means no alias. The grouped-origin bit belongs in item flags. The exact cold-store
record layout may be tightened after measurement, but a path item never owns a vector-backed path or
a global source span when the enclosing source is already known.

### Numeric token side store

Numeric token records retain only facts required by later semantic parsing and diagnostics. At
minimum they preserve authored/normalised text identity, numeric kind and lexical validation facts.
They do not widen every token. Counts and flags use fixed-width integer fields, never `usize`.

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
- a grouped or simple path normally projects to the path category and, only when useful, a `PathId`
- numeric literals preserve the authored text or kind needed by the diagnostic, not the complete
  tokenizer record
- a diagnostic must remain renderable after the source token side store is dropped or independently
  frozen

The global token redesign is preferred over maintaining a second wide diagnostic token enum. The
projection remains necessary because source-token and diagnostic retention policies are different.

## Compact diagnostic architecture

### Two-stage model

Diagnostics have one local construction model and one durable storage model:

```text
compiler stage
-> typed DiagnosticDraft
-> DiagnosticBag
-> identity remap and diagnostic compaction
-> DiagnosticStore
-> DiagnosticId + FrozenDiagnosticStore
-> terminal, terse, dev-server and tooling renderers
```

`DiagnosticDraft` is move-only. It is small enough to travel through local `Result` paths without
boxing every diagnostic. Rare draft data may use one heap allocation because only the exceptional
case pays for it.

`DiagnosticRecord` is the durable fixed record stored densely in `DiagnosticStore`.

```rust
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DiagnosticRecord {
    primary: SourceSpan,          // 8 bytes
    code_and_flags: u32,          // 4 bytes
    facts: [u32; 4],              // 16 bytes
    extra: DiagnosticExtraId,     // 4 bytes
}
```

The common record is exactly 32 bytes. New diagnostic families must fit this shape.

### Diagnostic identity and flags

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticCode(NonZeroU16);
```

`DiagnosticCode` is an explicit internal numeric identity. It is assigned in the diagnostic schema
and never derived from enum declaration order. Its descriptor supplies the external stable code such
as `MOTH-RULE-0044`, category, title and default severity.

`code_and_flags` uses this fixed layout:

```text
bits  0..15   DiagnosticCode
bits 16..17   severity override
bits 18..31   reserved, must be zero
```

Severity override values:

```text
00  use descriptor default
01  Error
10  Warning
11  Note
```

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

GenericInferenceConflict:
  fact 0 = parameter name StringId
  fact 1 = existing TypeDisplayId
  fact 2 = replacement TypeDisplayId
  fact 3 = packed subject + generic parameter ID
  extra  = optional previous-evidence secondary label

LargeTrackedAsset:
  fact 0 = path StringId or PathId
  facts 1..2 = byte size u64
  fact 3 = unused/zero
```

The schema tests validate that unused words and reserved bits are zero.

### Diagnostic draft

```rust
pub struct DiagnosticDraft {
    record: DiagnosticRecord,
    extra: Option<Box<DiagnosticExtraDraft>>,
}
```

The exact private representation may use another one-pointer rare-data owner, but it must remain no
larger than 48 bytes on supported 64-bit targets.

- no `Clone` implementation is provided
- common diagnostics allocate nothing
- rare variable data allocates at most one root draft object; its owned lists may allocate as needed
  before compaction
- draft facts use the producing worker's current identity domain
- deterministic remapping happens before the draft becomes durable
- draft destruction after compaction releases all temporary allocation

Returning `Box<DiagnosticDraft>` or `Box<CompilerDiagnostic>` from every validation helper is not an
accepted solution.

### Diagnostic IDs and store

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticId(NonZeroU32);

pub struct DiagnosticStore {
    records: Vec<DiagnosticRecord>,
    extras: Vec<DiagnosticExtraRecord>,
    labels: Vec<SecondaryDiagnosticLabel>,
    string_ids: Vec<StringId>,
    path_ids: Vec<PathId>,
    type_ids: Vec<TypeDisplayId>,
    substitutions: Vec<GenericSubstitutionRecord>,
    diagnostic_tokens: Vec<DiagnosticToken>,
    // Additional typed cold stores declared by the schema.
}
```

The exact list of typed cold stores evolves with the schema. Common rules do not:

- `DiagnosticId` indexes one dense record
- record order is deterministic compiler production order after canonical worker merge
- each auxiliary list uses a typed `start: u32, length: u32` range
- extra data is immutable after store freeze
- no per-record `Vec`, `HashMap` or trait object is stored
- no renderer mutates or enriches a diagnostic record

### Capacity-exhaustion policy

Compact indexes have explicit limits. User-authored input that exceeds one is not a compiler bug.

- The schema includes one no-extra `CompilerCapacityExceeded` diagnostic family with a compact
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

The exact external stable code is assigned through the diagnostic schema during implementation.

### Extra-data indirection

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticExtraId(u32);
```

`0` means no extra data. Other values are one-based indexes.

The baseline extra record is:

```rust
#[repr(C)]
struct DiagnosticExtraRecord {
    labels: LabelRange,     // start: u32, length: u32
    data: ExtraDataRef,     // kind/flags plus one u32 index
}
```

`ExtraDataRef::NONE` means the diagnostic has labels only. A diagnostic that needs both labels and a
variable fact list uses one extra record that points to both. An unusual diagnostic does not force a
second word into every common record.

Before creating a new extra kind, the owner must check whether the diagnostic can carry a smaller,
clearer semantic fact instead. Side storage is an escape hatch for genuinely variable or unusual
data, not permission to preserve bloated payloads unchanged.

### Primary and secondary labels

The primary source span is `DiagnosticRecord::primary`. It is not duplicated as a label.
A diagnostic with no related source site has no label allocation.

```rust
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SecondaryDiagnosticLabel {
    span: SourceSpan,
    message_and_data: u32,
}
```

`message_and_data` uses:

```text
bits  0..7    LabelMessageCode; 0 means no message
bits  8..31   24-bit immediate value or one-based LabelDataId
```

The label schema decides whether the upper field is immediate or indexes typed label data.
Examples include previous declaration, conflicting access, move site, generic instantiation site and
immutable binding declaration.

A substitution list does not live inside the label-message enum. It uses the substitution arena and a
label data reference.

More than `2^24 - 1` label-data entries in one compilation is a typed diagnostic-capacity failure for
user input, not integer wrapping.

### Diagnostic places

`DiagnosticPlace` is one `u32`, not an enum widened by a path owner.

```text
bits 30..31   place tag
bits  0..29   compact value
```

Initial tags:

```text
00  unknown
01  local/rendered StringId-domain value
10  PathId-domain value
11  reserved for a future typed place table
```

If an ID domain cannot fit 30 bits, the place uses the reserved tag and a side-table ID. It does not
widen the fact word. The storage module owns construction and decoding.

### Variable lists

Variable diagnostic lists use typed ranges into dense arenas:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct DiagnosticListRange {
    start: u32,
    length: u32,
}

struct StringIdList(DiagnosticListRange);
struct PathIdList(DiagnosticListRange);
struct TypeDisplayIdList(DiagnosticListRange);
struct SubstitutionList(DiagnosticListRange);
```

Typed wrappers prevent a renderer from interpreting one arena as another.

Lists currently embedded in reason enums or payload variants—known fields, available variants,
missing variants, generic parameters, candidates and substitutions—must migrate to these stores.

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

Conceptually:

```rust
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

A procedural generator remains deferred unless the declarative schema becomes demonstrably
unmanageable. “More convenient” is not sufficient evidence.

## Diagnostic type-display snapshots

### Purpose

Durable diagnostics do not retain complete module `TypeEnvironment` values. Before the environment
can be dropped, the diagnostic compaction boundary captures only the transitive facts needed to
render referenced types.

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeDisplayId(NonZeroU32);
```

### Display store

The baseline compact record is:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct DiagnosticTypeRecord {
    tag: u16,
    flags: u16,
    data_0: u32,
    data_1: u32,
}
```

Typed side ranges supply variable arguments and choice-variant summaries.

The store must represent every current user-visible type spelling:

- builtins
- structs and const records
- choices, including variant names and whether a variant has payload
- options
- growable and fixed collections
- ordered maps
- fallible carriers and multi-success signatures
- tuple/internal multi-value display where required by an existing diagnostic
- function parameters, returns and error return
- generic parameters
- concrete generic nominal instances
- external types with useful package/name identity
- unknown external types only where the existing semantic model explicitly permits them

### Snapshot algorithm

For each diagnostic draft produced with a module-local `TypeEnvironment`:

1. Inspect the schema and collect every fact marked as a source `TypeId`.
2. Reserve a `TypeDisplayId` for each newly visited type before recursing, so recursive display graphs
   cannot loop.
3. Copy only display-relevant facts into `DiagnosticTypeStoreBuilder`.
4. Recursively intern referenced child types and typed lists.
5. Rewrite the draft fact from `TypeId` to `TypeDisplayId`.
6. Validate that every type-marked fact was rewritten.
7. Freeze the type display store before rendering.
8. Release the complete `TypeEnvironment` when no semantic artefact owner still needs it.

The store deduplicates equivalent display records inside one compilation context. It does not claim
cross-build type identity.

A renderer receiving an unknown `TypeDisplayId` or an unrewritten source `TypeId` is observing a
compiler invariant violation and invokes `compiler_bug!`. It does not fall back to printing an
internal numeric ID.

### Rendering equivalence

Before full `TypeEnvironment` retention is deleted, tests must compare the old and new rendering for
every supported type shape and for representative nested combinations. The new store must preserve
user-visible spelling unless an explicitly authorized diagnostics-improvement slice changes it.

## Frozen render context and cloning policy

A diagnostic report carries one immutable context owner:

```rust
pub struct DiagnosticReport {
    context: Arc<FrozenCompilationContext>,
    diagnostics: DiagnosticRange,
}
```

A graph-level result may carry several module/package reports when they belong to independent
identity contexts. It does not concatenate raw IDs without preserving their context owner.

Rules:

- `StringTable::clone()` is not used to snapshot every failure boundary.
- a finalized string table is shared immutably
- source, path, type-display and diagnostic stores share the same context lifetime
- renderers borrow the frozen context
- successful warnings and failed diagnostics use the same record/store model
- a report can be cheaply cloned by cloning one `Arc` plus a range, not by copying tables
- stage-local mutable bags are consumed, not cloned
- no context is kept alive by hidden global state

The existing local fork/delta string-table strategy may be retained and improved. This design does
not require a globally contended string interner.

## Failure architecture

Moth compiler failures have exactly three lanes. They are not converted into one another merely
for rendering convenience.

### Lane 1: user-caused diagnostics

User source, project structure, configuration, imports, type errors, rule violations, borrow errors,
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
    kind: InfrastructureFailureKind,
    stage: InfrastructureStage,
    source: Option<SourceSpan>,
    message: Box<str>,
    details: Box<[InfrastructureDetail]>,
}
```

Examples:

- source or config file cannot be opened
- output cannot be written
- permission is denied
- a dev-server port cannot bind
- file watching fails
- a backend tool or registered provider fails
- target runtime resources are unavailable
- an invalid host path cannot be represented by the compiler's source identity policy

Properties:

- recoverable by an owning CLI/tooling boundary where retry or a clear exit is possible
- not stored in `DiagnosticRecord`
- no conversion to an `InfrastructureError` diagnostic payload
- typed deterministic details instead of a string-keyed `HashMap`
- may have no source span
- can be rendered in terminal, terse and dev-server forms through a separate infrastructure renderer
- never widened to include arbitrary subsystem objects

An operational failure is not a compiler bug merely because the user cannot repair it in source.

### Lane 3: compiler bugs

A proven compiler invariant violation uses a non-recoverable panic produced by `compiler_bug!`.

```rust
#[track_caller]
pub fn raise_compiler_bug(
    stage: CompilerStage,
    source: Option<SourceSpan>,
    message: fmt::Arguments<'_>,
) -> !;
```

The panic payload is a structured `CompilerBugReport` containing:

- compiler stage
- optional Moth source span
- Rust caller file, line and column from `#[track_caller]`
- compiler version and commit identity where available
- a precise invariant message
- a clear request to report the compiler bug

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

### Result boundary

Conceptually:

```rust
pub type InfrastructureResult<T> = Result<T, InfrastructureFailure>;

pub enum ModuleCompilationOutcome {
    Success(CompiledModuleArtifact),
    Diagnosed(ModuleDiagnosticReport),
}

pub fn compile_module(...) -> InfrastructureResult<ModuleCompilationOutcome>;
```

Compiler bugs are not another `Result` variant. They panic.

### Current `CompilerError` migration

Every current `CompilerError` construction site must be audited and assigned to one lane before the
type is deleted:

- user-caused state becomes a diagnostic schema entry
- expected operational state becomes `InfrastructureFailure`
- a proven invariant becomes `compiler_bug!`

No broad automatic conversion is allowed. The audit records the old site, chosen lane and reason.
`CompilerError`, `ErrorType`, metadata maps and the `DiagnosticPayload::InfrastructureError` bridge are
deleted only after the inventory reaches zero.

## Tooling host isolation

A compiler bug is non-recoverable for the owning compilation, but a long-lived host may remain alive
if compiler work is isolated correctly.

### CLI

The normal CLI installs the structured compiler-bug panic hook. A compiler bug prints the report and
terminates the process. The CLI does not convert the panic into a successful or ordinary diagnosed
build.

### Dev server and future LSP

Each build or analysis request runs in a worker that uniquely owns its mutable compilation context.
The host owns only immutable inputs and receives a completed success, diagnosis or infrastructure
failure after the worker finishes.

At the worker boundary:

- the host may catch unwind solely to identify a compiler bug and keep the outer service alive
- the failed worker's complete mutable state is discarded
- no partially mutated compiler context is reused
- no compiler panic may occur while holding shared dev-server/LSP state locks
- host state is updated only after the worker has joined
- unknown panic payloads are rendered as generic compiler bugs and the worker is discarded
- operational failures continue through `InfrastructureFailure`, not panic catching

The existing poisoned-lock recovery pattern is removed once the compiler no longer mutates shared
host state during a build. Recovering and continuing with potentially inconsistent compiler-owned
state is prohibited.

### Panic strategy

Thread/ownership isolation requires unwinding. Until compilation runs in an isolated child process:

- release and profiling configurations used by the CLI/dev server/LSP must use `panic = "unwind"`
- tests cover the worker boundary and state discard
- `panic = "abort"` is not used for a host that promises to survive compiler bugs

A process-isolated compiler worker is deliberately deferred. If implemented later, the child may use
`panic = "abort"` because the host discards the process. That is a host architecture change and must
not alter the three failure lanes.

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

Source spans do not need remapping because final `SourceId`s are assigned before tokenization.

No compact ID is allocated through a timing-dependent global atomic simply because the numeric type
is cheap.

## Performance evidence and acceptance

### Evidence owner

This architecture uses the existing frontend instrumentation and benchmark tooling. New counters and
reports belong under those owners rather than being scattered through source, token or diagnostic
modules.

The implementation plan creates:

```text
benchmarks/compiler-data-layout-results.md
```

Raw allocator logs, profiler captures and per-run data remain uncommitted.

### Required baseline measurements

Before changing representation, record with the exact CI toolchain:

- `size_of` and alignment for every hard-layout type and current predecessor
- total sources and source bytes
- total tokens and current token bytes
- current `TokenKind` and `PathTokenItem` sizes
- total source-location instances where measurable
- current diagnostic, payload, label and reason sizes
- diagnostic counts, secondary-label counts and variable-list counts
- span start and length histograms
- path count, unique path count, component count and clone/remap pressure
- string-table full clones
- full `TypeEnvironment` values retained solely for diagnostics
- peak retained frontend bytes or a repeatable allocation proxy
- compile/check wall time for representative success-heavy and diagnostic-heavy workloads

A counting allocator may be added behind a benchmark-only feature. Normal compiler builds must not
pay for it.

### Representative workloads

The corpus must include:

- docs project release build/check
- existing focused frontend benchmark suite
- path/import-heavy multi-file project
- token-heavy large source
- template-heavy source with short and long spans
- generic/type-heavy source
- warning-heavy successful build
- diagnostic-heavy malformed source through a dedicated diagnostic benchmark harness
- long-line, Unicode and CRLF source
- sources larger than each candidate inline-start threshold

Diagnostic stress benchmarks are performance evidence, not correctness owners. Stable diagnostic
codes, spans and output remain owned by normal integration and unit tests.

### Repetition and regression rules

- run five independent benchmark invocations and compare medians
- use focused profiles when a result is ambiguous
- a median wall-time regression greater than 5% in docs, focused frontend or a targeted workload is a
  blocker unless the user explicitly accepts a documented tradeoff
- a measured optimisation that adds substantial complexity but does not produce a repeatable memory
  or throughput gain is rejected or reverted
- counter movement alone is not a win
- layout-size reduction is necessary evidence but not sufficient evidence for an optional complex
  encoding
- source/span/token phases must demonstrate lower retained frontend memory on representative projects
- diagnostic phases must demonstrate lower failure-path retention and remove the Clippy large-error
  condition without boxing the common diagnostic

No speculative percentage memory target is required beyond the hard layout contracts. The report
must still demonstrate that the overall compiler actually retains less memory.

## Validation and anti-drift tests

### Layout tests

A dedicated layout test module asserts at least:

```rust
assert_eq!(size_of::<SourceId>(), 4);
assert_eq!(size_of::<LocalSpan>(), 4);
assert_eq!(size_of::<Option<LocalSpan>>(), 4);
assert_eq!(size_of::<SourceSpan>(), 8);
assert_eq!(size_of::<Option<SourceSpan>>(), 8);
assert_eq!(size_of::<PathId>(), 4);
assert_eq!(size_of::<Option<PathId>>(), 4);
assert_eq!(size_of::<TokenShape>(), 8);
assert_eq!(size_of::<DiagnosticToken>(), 8);
assert_eq!(size_of::<SecondaryDiagnosticLabel>(), 12);
assert_eq!(size_of::<DiagnosticRecord>(), 32);
assert!(size_of::<DiagnosticDraft>() <= 48);
```

The transition retains `size_of::<CompilerDiagnostic>() <= 128` until the old type is removed.

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
- simple and grouped path records
- alias spans
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

Every code-bearing accepted slice runs the focused tests for its owner, formatting where Rust changed,
the required benchmark check and the repository's full `just validate` gate. Each phase also performs
a manual architecture and style-guide review.

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

## Deliberately deferred work

These items are compatible with the architecture but not part of the initial implementation unless a
phase's explicit stop/go gate accepts them:

- process-isolated compiler workers
- persistent serialization and remapping of `SourceId`, `PathId`, diagnostic IDs and type-display IDs
- procedural-macro or build-script diagnostic schema generation
- terminator-match span encoding when the required experiment does not beat exact overflow storage
- token records smaller than 8 bytes
- global conversion of all `StringId`, `TypeId` and unrelated compiler IDs to non-zero or packed forms
- memory mapping or compression of retained source snapshots
- persistent/incremental source databases and line-index caches
- broad LSP protocol implementation
- global diagnostic deduplication across independent compilation contexts
- cross-build diagnostic/type-display caches
- further path-node compression beyond the true-interner design
- additional cold-store packing without measured retained-memory pressure
- unrelated user-facing diagnostic wording and suggestion improvements

Rejected optional experiments must be recorded with their evidence and re-entry condition rather than
left as vague future work.

## Documentation and policy synchronization

Implementation of this design requires synchronized changes to:

- `AGENTS.md` — add this document to the reading list for source/token/diagnostic/failure work
- `docs/compiler-design-overview.md` — source context, token ownership, diagnostics and failure lanes
- `docs/build-system-design.md` — deterministic source registration, compilation contexts and tooling
  worker boundaries
- `docs/src/docs/codebase/style-guide/style-guide.mtf` — hard layout and failure-lane rules; remove
  boxing as the normal `result_large_err` answer
- `docs/src/docs/codebase/style-guide/testing.mtf` — layout/property/failure-lane ownership
- `docs/src/docs/codebase/style-guide/validation.mtf` — updated manual architecture audit
- `docs/src/docs/progress/@page.moth` — current implementation status during and after migration
- `docs/roadmap/roadmap.md` — sequencing and measured deferrals
- `docs/roadmap/plans/compiler-diagnostics-improvement-plan.md` — dependency and post-layout resume
  capsule
- `docs/roadmap/plans/frontend-arena-semantic-invariant-optimization-plan.md` — remove overlapping
  ownership and point source/token/diagnostic layout work here
- `index.md` — final module/file map

Generated documentation under `docs/release/**` is rebuilt, never edited directly.

## Final implementation map

Exact subfile names may be refined during implementation, but ownership should converge on this
shape:

```text
src/compiler_frontend/source/
    mod.rs
    id.rs
    span.rs
    span_encoding.rs
    database.rs
    line_index.rs
    provenance.rs
    tests/

src/compiler_frontend/symbols/path_interner/
    mod.rs
    id.rs
    builder.rs
    frozen.rs
    delta.rs
    remap.rs
    tests/

src/compiler_frontend/tokenizer/
    tokens/
        mod.rs
        tag.rs
        shape.rs
        store.rs
        cursor.rs
        ranges.rs
        numeric_store.rs
        path_store.rs
        tests/

src/compiler_frontend/compiler_messages/
    diagnostics/
        mod.rs
        code.rs
        schema.rs
        draft.rs
        record.rs
        facts.rs
        labels.rs
        extras.rs
        store.rs
        compaction.rs
        token.rs
        tests/
    diagnostic_types/
        mod.rs
        id.rs
        record.rs
        builder.rs
        render.rs
        tests/
    infrastructure/
        mod.rs
        failure.rs
        render.rs
        tests/
    compiler_bug/
        mod.rs
        report.rs
        hook.rs
        tests/
    report.rs
    render/

src/compiler_frontend/context/
    mod.rs
    builder.rs
    frozen.rs
    worker_delta.rs
    tests/
```

Core pipeline and `mod.rs` files remain orchestration maps. Bit codecs, capacity formulas, schema
internals and benchmark-only accounting do not accumulate in broad pipeline files.

## Completion definition

This architecture is implemented only when:

- every hard layout assertion passes on supported targets
- source spans are exact compact byte ranges across the compiler
- retained source text and line indexes render all diagnostics
- `InternedPath` is no longer a durable compiler identity
- retained syntax uses source-owned compact tokens and ranges
- the 32-byte diagnostic store replaces the wide payload model
- complete type environments are no longer retained solely for diagnostics
- frozen contexts replace deep-cloned render tables
- every former `CompilerError` site has one explicit failure lane
- only proven invariant bugs panic
- long-lived tooling isolates compilation state and no longer recovers poisoned compiler state
- CI Clippy passes without `result_large_err` boxing or lint suppression
- representative memory measurements improve and timing stays within accepted bounds
- the authority documents, progress matrix, roadmap and codebase index describe the final owners
- no compatibility adapter preserves the old source-location, token, diagnostic or error path
