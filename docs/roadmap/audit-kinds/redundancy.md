# Redundancy

Looking for: the same fact, transformation, validation or policy owned more than once, and layers that do not earn their cost.

**Evidence bar:** equivalent behaviour *and* ownership. Textual similarity is not enough, and two functions sharing a few lines is not an abstraction opportunity.

## Map owners first

Identify the facts the area owns, its producer and consumers, which data is authoritative and which derived, and any active plan already replacing part of it. Duplication only means something relative to ownership.

## Duplicated work

Functions with equivalent signatures, repeated match arms and validation branches, copied state machines, target-specific functions differing only by a policy value, test and production helpers independently implementing the same normaliser, code copied during a refactor and never deleted.

For each, compare inputs, outputs, side effects, failure lane, owner, lifecycle, ordering and likely future divergence before proposing anything.

## Repeated semantic work

Beyond duplicate functions, look for work done twice:

- source rescanned or re-parsed after the owning parse phase
- visibility, dependency or project topology reconstructed downstream
- constants, templates, types or traits resolved more than once
- public surfaces copied into consumers instead of bound by stable identity
- AST or HIR reopened to recreate link, effect, borrow or lifetime facts owned elsewhere
- reachability recalculated by several consumers
- diagnostics independently reconstructed from the same semantic condition
- output dependencies scanned from rendered strings after semantic path tracking

A repeated cheap operation can still be right when the alternative is worse ownership. Explain the trade-off before filing.

## Duplicated state

Parallel structs for old and new API shapes; both local and global registries owning one identity; derived summaries stored as if authoritative; booleans encoding a state an enum already represents; shadow maps that can drift from their owner; parse, AST, HIR and backend representations retaining facts past their stage; compatibility aliases; separate error types carrying identical structured data.

Prefer one authoritative record plus narrow derived indexes. Do not collapse semantically distinct lanes to reduce type count.

## Obsolete paths

Compatibility wrappers and forwarding functions, deprecated entry points, stale structs, fields and variants, fallbacks whose callers are deleted, obsolete `cfg` branches, unjustified `allow(dead_code)`, old target paths kept beside the current one, stale TODO implementations, fixtures for removed behaviour.

Production deletion belongs here. Removing tests or correcting docs needs a linked finding in that lane.

## Layers that do not earn their cost

Trace important calls from entry point to real work. One-line forwarders with no policy; wrappers around a single value with no semantic identity; contexts passing another context unchanged; adapters between equivalent shapes; traits with one implementor and no justified boundary; registries with one hard-coded entry; builder layers that only rename compiler-owned data; error conversions adding no information.

Keep a wrapper that enforces a real boundary, identity, capability or invariant - and record that reason if similar wrappers are being removed elsewhere.

## Abstraction ownership

Before sharing anything, ask whether the behaviour is genuinely identical, whether one clear owner exists, whether callers depend on it naturally, whether it reduces total control-flow complexity rather than relocating it, and whether the generic surface is larger than the real use set.

When consolidation is right, prefer explicit data ownership - one table or arena plus stable IDs, one immutable artefact, one pass producing all related side-table facts, one policy table consumed by target-specific code - over object-style indirection. Do not force structure-of-arrays or arena storage on a tiny data set.

## Target and source-kind specialisation

Comparing JS, Wasm, builder and provider paths, decide whether a similarity is language-owned behaviour that should move before lowering, compiler-owned policy both backends should consume, builder orchestration no backend should duplicate, or genuinely target-specific lowering that must stay separate.

Do not create a shared backend abstraction that erases a real target difference.

## Every similarity gets a verdict

Leave local; extract locally; move to a common owner; restructure so the repetition disappears; delete; merge parallel APIs; or split a broad owner before deciding. Say why the chosen action beats the alternatives.

Do not optimise for the smallest diff or fewest lines. A named intermediate or narrow type earns its lines.

## Stale when

Module ownership, public or internal APIs, principal data structures, or the passes and representations in the area change materially.
