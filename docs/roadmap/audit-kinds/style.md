# Style

Looking for: code that behaves correctly but is hard to read, review, extend or delete.

The [style guide](../../src/docs/codebase/style-guide/style-guide.mtf) is the authority. This is the procedure for auditing against it.

**Evidence bar:** concrete code plus a named cost - slows review, obscures data flow, makes an API easy to misuse, or hides a state transition. "Cleaner" and "more idiomatic" are not justifications. Do not record formatting `rustfmt` already decides.

## Ownership

Open the module entry point first. Identify the single responsibility, its input and output data, and its neighbours. Names and structure should make that visible before any implementation is read. Record files whose name no longer matches what they own.

Do not infer a new architecture because the layout is untidy - route ownership changes to Redundancy or Correctness.

## Module and file structure

Entry points act as a structural map, not a storage place. Files group one coherent task or data owner. Related private behaviour deepens into submodules rather than scattering across broad utility files. Files are not split so aggressively that basic control flow needs constant navigation. Re-exports expose a narrow intentional surface. Test code stays out of production files.

When proposing a split, name the responsibility that moves and the data boundary it receives and returns. Never split only to reduce line count.

## Data shape

- Structs represent data records or stage results, not objects with broad behavioural ownership.
- Passes operate over explicit inputs, stores, arenas, tables, side tables or immutable artefacts.
- Data used together is stored and passed together; data with different lifetimes is not hidden in one broad context.
- Enums represent meaningful states instead of boolean clusters.
- Named result structs replace tuple-heavy returns when field meaning matters.
- IDs make their owning table obvious; parallel structures have an explicit alignment invariant.
- Trait objects and generic surfaces do not obscure concrete stage ownership.

Data-oriented design follows access patterns. Do not convert every struct into parallel arrays.

## API shape

Descriptive input and result types. Narrow functions exposing one operation. Explicit state transitions rather than hidden mutation. No boolean-heavy call sites whose meaning needs the signature to decode. No defaulted parameter preserving an obsolete shape. No broad trait bound existing for one call site. Borrowed views for read-only queries. Consistent naming across a producer-consumer pair.

Internal pre-release API compatibility is not a goal. Structural removal of the old path belongs to a linked Redundancy finding.

## Functions and control flow

The name matches the complete responsibility. The main path reads as named steps. Early returns clarify terminal paths. Complex validation uses explicit control flow rather than nested combinator chains. Large matches group by meaning. Named intermediates expose data flow. Closures stay small and local. Deep nesting usually means a missing helper or state enum.

Prefer the shortest form that stays obvious under review, not the fewest lines.

## Naming, imports, layout

Full domain terms over unexplained abbreviations. Similar names have distinct roles. Stage names match canonical terminology. Imports keep long paths out of bodies without flooding the namespace. Blank lines reveal logical steps. Macros are smaller and clearer than what they replace. Section banners mark real boundaries, not an oversized file.

## Ownership noise

Without making performance claims: `.clone()` used to dodge a clearer borrow, owned `String` or `PathBuf` where interned identity exists, whole-collection cloning for one lookup, repeated conversion between equivalent representations, wrappers introduced only to satisfy an awkward API, defensive copies signalling unclear mutation ownership.

Any runtime claim routes to Performance; any cross-stage consolidation routes to Redundancy.

## Errors and lints

User paths free of `panic!`, `todo!`, unchecked indexing and unjustified `.unwrap()`. Internal failures visibly distinct from user-facing ones. Lint suppressions narrow, documented and still needed. No `allow(dead_code)` preserving forgotten implementation. No leftover debug printing.

## Pick the smallest valid action

Rename locally; simplify without moving ownership; introduce a narrow named type inside the same owner; split a mixed-responsibility file inside the same owner; route a cross-owner concern elsewhere; or leave it because the explicit form is clearer than the abstraction.

## Stale when

Module organisation, important APIs, principal data structures or the control-flow shape of substantial functions change materially.
