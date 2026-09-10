# Redundancy

Find duplicated ownership, repeated work, obsolete paths and abstractions whose
cost exceeds their value.

Follow the [audit guide](../audit-guide.md) for selection, allowed writes,
findings and coverage. This checklist owns what to inspect and the evidence
needed for a Redundancy finding.

## When this kind fits

Use Redundancy when equivalent responsibilities have several owners, work is
repeated without a justified boundary, or a retained path no longer serves the
current design.

A measured time or memory problem belongs to Performance. A violated semantic
contract belongs to Correctness. Unclear code within an otherwise sound owner
belongs to Style. Independent test-quality and documentation defects belong
to their respective kinds. Follow the audit guide when a discovery crosses
those boundaries.

## 1. Establish the current owner

Start at the area's entry points. Identify the inputs, outputs and facts it
owns, then trace its producers and consumers. Distinguish authoritative data
from derived indexes, cached views and target-specific representations.

Check current support and active work. Record which path is current, which is
being replaced and whether a retained path has a concrete remaining consumer.
An accepted migration boundary needs its own assessment rather than an
assumption that every temporary representation is already obsolete.

The question is: where should a change to this behaviour be made once?

## 2. Find all implementations and callers

Search the owner and adjacent paths for equivalent work, old API names,
forwarders, re-exports, target variants, test helpers and registrations.
Follow indirection to the implementation before counting separate owners.

Compare the actual call graph, not the number of names. Several names may
reach one implementation. One shared function may still repeat work each
time a consumer calls it.

Inspect these four classes where they apply:

- **Repeated logic or work.** Copied validators, normalisers and state machines,
  repeated sorting, reparsing, resolution or traversal over the same inputs.
- **Duplicated state.** Parallel registries, independently mutable summaries,
  copied public surfaces and old and new records kept as competing authorities.
- **Obsolete paths.** Unreachable fallbacks, replaced APIs, compatibility shims,
  stale variants and target paths with no current consumer.
- **Unearned layers.** Forwarders, adapters, contexts, traits and registries that
  add navigation or state without enforcing a useful contract.

For semantic work, trace whether consumers use the owning stage's result or
reconstruct it from source, syntax, an earlier IR or rendered output.

## 3. Prove equivalence or obsolescence

For suspected duplication, compare inputs, outputs, side effects, failure
behaviour, ordering, identity domain, lifetime and ownership. Explain whether
target or lifecycle differences require separate implementations.

Name the maintenance cost. For example, one rule requires coordinated edits
in two owners, an independently mutable copy can drift, or a consumer has to
reconstruct a fact that should have crossed a handoff explicitly.

For a deletion candidate, trace callers and registrations, relevant feature
and target gates, tests and active plans. A text search with no direct caller
is a lead, not proof that a path is unused.

For repeated work, show the repeated execution path. Measurement is needed for
a performance claim, but an evidenced ownership or maintenance defect does
not need an invented timing estimate.

## 4. Test the reason to keep it

Look for a real benefit before recommending removal or sharing:

- Distinct ID domains, capabilities or invariants may justify small wrappers.
- Target-specific lowering may look similar while having different semantics.
- Derived indexes and immutable snapshots may serve different access patterns
  or lifetimes without becoming competing authorities.
- Independent test or oracle logic may deliberately avoid sharing the
  production implementation so it can expose a common-mode defect.
- A narrow local repetition may be clearer than a policy-heavy shared helper.

For each serious candidate, record the strongest reason to retain the current
shape and the evidence that supports or defeats that reason. Useful retained
distinctions belong in the report's checked-and-clean summary.

## 5. Recommend the simplest coherent correction

Consider deletion first when a path is obsolete. For equivalent behaviour,
prefer one existing owner with direct consumers. Use a local helper when the
behaviour is local. Change a handoff when repetition exists because the owner
fails to publish a fact its consumers need.

Compare the whole resulting path. Include any new configuration, branching,
conversion, lifetime coupling and navigation introduced by consolidation.
Fewer lines alone do not establish an improvement.

Keep data ownership explicit. A shared table, immutable result or narrow
policy value may remove repetition more clearly than another layer of
behavioural indirection. Avoid broad utility modules or generic frameworks
without a real shared responsibility.

Give each serious candidate a verdict: retain with a reason, delete,
consolidate locally, use one common owner or restructure the handoff. A
suggested correction remains subject to the audit guide's triage rules.

## Evidence for a finding

Use the common report schema and include the relevant kind-specific proof:

- The implementations, representations or paths compared and their callers.
- Their shared responsibility, or the reason a path is obsolete or unnecessary.
- The concrete maintenance or ownership cost.
- The strongest reason to keep the current shape and why it does not justify
  that cost.
- The proposed owner, required retained distinctions and verification needed
  for a later correction.

One root cause can cover several occurrences. Split findings when the owners
or independently actionable corrections differ, not merely because several
files contain the pattern.

## Coverage and staleness

Record which owners, callers, target variants and relevant feature paths were
inspected. Separate context reading from audited coverage. A search inventory
alone does not establish completeness.

Mark coverage partial when a relevant implementation, consumer or gated path
remains uninspected. Record a major checklist section as not applicable only
when the area's structure makes it irrelevant.

Coverage becomes stale when the area's ownership, important APIs, principal
data structures, passes or producer-consumer handoffs change materially.
