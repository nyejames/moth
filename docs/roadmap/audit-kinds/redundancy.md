# Redundancy

Find code, state, work and structure that Moth no longer needs or maintains in too many places. Look for a **smaller, more direct design** through deletion, single ownership or simpler handoffs.

Scrutinise bloat left by incremental agent-driven changes, but reject false positives: similar code may serve distinct contracts and fewer lines may be harder to follow. Follow the [audit guide](../audit-guide.md) for formal reports and coverage. Log incidental issues in [`docs/bug-reports.md`](../../bug-reports.md).

## 1. Map the current design before judging it

Read the area's entry points, principal types and `mod.rs` documentation. Trace producers, consumers and current public APIs. Establish:

- What the area owns, what it excludes and where its outputs go.
- Which data is authoritative, derived, cached, copied or target-specific.
- Which paths remain supported and which accepted plans are actively replacing them.
- Which callers, registrations, tests, features and target variants still exercise each path.
- Which existing helper or subsystem already owns a similar rule.

Ask: **Where should a developer change this behaviour once?** Follow actual call graphs and data flow, not just filenames or similar identifiers. Several wrappers may reach one owner, while one shared helper may still cause every caller to repeat the same expensive work.

## 2. Hunt for code that should disappear

Search beyond the changed file. Inspect callers, tests and earlier owners for leftovers from migrations or fixes.

### Obsolete and unreachable paths

- Do old entry points, forwarding wrappers, aliases, adapters or re-exports remain after callers moved to a newer API?
- Can a branch, enum variant, configuration field or fallback still be reached through any supported command, feature or target?
- Does a compatibility layer preserve a superseded interface despite Moth's pre-release policy?
- Did a finished migration leave parallel data types, temporary registries, stale helpers, old fixtures or dead comments behind?
- Does a defensive special case exist solely to keep an obsolete implementation path working?

Named migration bridges are legitimate only when an accepted plan owns their purpose and exact deletion point. No call sites found by text search is a lead, not proof of unreachability. Follow registration, generated dispatch, feature gates and indirect callers before recommending deletion.

### Unnecessary behaviour or scaffolding

- Does each function, field, helper, type or module have a present-day responsibility?
- Could a caller perform one obvious operation directly without losing a meaningful name or boundary?
- Are there speculative extension points, unused options, unused genericity or configurable policies for a single supported behaviour?
- Can two transformations become one operation without retaining both intermediate forms?
- Can a removed feature take its parser exceptions, validators, diagnostic reasons, fixtures and documentation branches with it?

**Deletion comes first** when nothing current depends on the path. Do not replace obsolete code with a new shim.

## 3. Find duplicated ownership and repeated logic

Search for near-duplicate functions, validators, normalisers, predicates, sorting, traversal, serialisation, diagnostics and state transitions. Compare what the paths actually do.

- Does one semantic rule get checked or reconstructed independently in two compiler stages?
- Does a consumer rescan source, reparse syntax or infer a fact that its producer already owns?
- Are there overlapping helpers with unclear responsibility, including local utilities that duplicate a nearby subsystem's API?
- Do multiple call sites copy the same multi-step policy, perhaps with slightly different failure, ordering or fallback behaviour?
- Are there separately maintained representations of the same source of truth, such as parallel registries, summaries or flags?
- Does each consumer repeat a traversal, sorting pass, lookup or conversion because an earlier owner does not publish the needed fact?
- Can a shared diagnostic or validation reason be constructed once with typed facts instead of duplicating prose and matching rules?

Compare inputs, outputs, side effects, errors, owners, ID domains, ordering, lifetimes and target capabilities. Establish equivalent responsibility and its cost, such as coordinated edits, semantic drift or repeated work, before calling similar text redundant.

## 4. Challenge abstraction and indirection

An abstraction earns its place when it names a meaningful operation, establishes ownership or an invariant, shares substantial identical behaviour, or makes a current path easier to understand.

Actively investigate:

- **Thin forwarders:** helpers that rename an operation or immediately delegate without adding useful context.
- **Wrapper types and adapters:** layers that only translate between equivalent shapes or shuttle one field around.
- **Broad context objects:** containers that hide unrelated parameters, mixed lifetimes or dependencies behind a convenient name.
- **Premature traits and generics:** abstractions introduced for hypothetical reuse, two superficially similar callers or a single implementation.
- **Over-structured builders:** multi-step construction for values that could be built directly without violating validation or ownership.
- **Boolean-heavy APIs:** combinations of flags that encode implicit states, require coupled conditions or exist to share code that should remain separate.
- **Scattered logic:** a coherent operation split across tiny helpers or modules whose navigation costs outweigh the separation.
- **Mixed-responsibility modules:** one file orchestrates unrelated stages or owns concepts that change for different reasons.

Ask what would actually disappear if the abstraction were removed. Count new branches, conversions, context passing, lifetime coupling, navigation and duplicated state in any proposed replacement. A single-use helper may still be valuable when it clearly names a stage or isolates a real invariant.

## 5. Inspect agent-driven accumulation patterns

Incremental fixes often leave every previous layer intact. Look specifically for these sequences, tracing when and why they arose:

- A bug fix adds another validator or fallback while the original owner remains unchanged.
- New data fields mirror values already stored in a different representation, requiring updates at several sites.
- A new helper duplicates an existing one because the implementation searched only its immediate file.
- An adapter conceals a poor boundary instead of moving the needed fact to the correct producer.
- Several branches handle variants separately even though their difference could be expressed once as data.
- An exceptional case persists after the rule or feature that required it was removed.
- One long function is split into many pass-through functions without clarifying ownership or control flow.
- A dense chain of conversions, clones and builder calls compensates for an unnecessarily indirect API.
- A large new framework appears where one explicit local operation would satisfy the current use case.

These are **search triggers**, not findings. An extra validation may protect a trust boundary and a copy may preserve a necessary lifetime. Trace the requirement first.

## 6. Look for simpler Rust and module structure

Review code presentation where complexity stems from unnecessary structure rather than merely formatting:

- Could a straightforward loop or named sequence replace nested iterators, stateful closures or deeply chained builders?
- Could an existing enum, typed range or narrow state record eliminate boolean coupling or independently mutable fields?
- Does repeated owned-to-borrowed conversion or cloning result from an API that should accept borrowed data or transfer ownership directly?
- Do temporary collections, adapter records or general-purpose registries exist only to pass values to the next step?
- Are files divided by real responsibility, or does the layout force readers through many trivial modules?
- Do test helpers and fixture builders duplicate production setup or each other without improving isolation or coverage?
- Would moving a genuinely shared responsibility to its existing owner let two implementations and their tests disappear?

Follow the [codebase style guide](../../developer-docs/style-guide/style-guide.mtf). Prefer explicit ownership over broad utility modules. Route readability-only problems to Style and measured costs to Performance. Do not claim speedups without measurement.

## 7. Evaluate the reason to keep each candidate

Test the strongest counter-explanation before recommending consolidation:

- Similar target lowering may require different capabilities, failure behaviour or representations.
- Small wrappers may protect distinct ID domains, lifetimes or invariants.
- Immutable snapshots and derived indexes may serve different access patterns without becoming competing authorities.
- Independent test or oracle code may intentionally duplicate an algorithm to catch common-mode defects.
- A short local repetition may be clearer than a policy-heavy helper or shared framework.
- An active migration may temporarily require two forms with an explicit deletion milestone.

Separate **similar appearance** from **equivalent semantics**. If retention wins, explain why, especially for candidates that initially looked like strong findings. Do not force an abstraction for cosmetic deduplication.

## 8. Recommend the smallest coherent correction

Choose a concrete disposition for each serious candidate:

1. **Delete:** remove obsolete code, registrations, tests and associated scaffolding.
2. **Keep local:** retain intentional small repetition with a documented distinction.
3. **Consolidate locally:** share identical behaviour inside the existing subsystem.
4. **Move to the owner:** make both callers consume one existing authoritative implementation.
5. **Restructure data flow:** publish the producer's owned fact once instead of adding another helper or repeated traversal.
6. **Split responsibility:** extract genuinely separate ownership into a focused module, not a broad utility layer.

Prefer deletion and direct data flow. Identify what disappears and which invariants remain. Added structure needs a stronger current justification than a simplification.

## Findings, coverage and staleness

For each formal Redundancy finding, provide:

- Exact paths, implementations and relevant callers or consumers inspected.
- Proof of shared responsibility, unnecessary structure or genuine obsolescence.
- Concrete maintenance, ownership or execution cost, without invented performance numbers.
- Strongest reason to retain the design and why it fails.
- Proposed disposition, existing owner, deletions, preserved distinctions and verification needed.

Group occurrences with one root cause. Split findings when fixes have independent owners. Prioritise removal of competing semantic owners and high-drift duplication over small cosmetic savings. Record checked-and-clean distinctions, target gates and exclusions so later auditors can trust coverage.

Coverage becomes stale when an area's ownership, current APIs, principal structures, passes or producer-consumer handoffs change materially. An unrelated issue discovered during inspection belongs in `docs/bug-reports.md` and does not count as coverage for its kind.
