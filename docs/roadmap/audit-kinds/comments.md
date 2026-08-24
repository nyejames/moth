# Comments

Looking for: missing, stale, misleading or noisy local intent.

The [style guide](../../src/docs/codebase/style-guide/style-guide.mtf) is the authority for comment form. This kind is code-neutral - it changes no executable tokens.

**Evidence bar:** a specific intent that is missing, wrong or buried. "Add comments" is not a finding; name the intent that needs preserving.

## What needs explaining

Map the area first: what it owns, its exclusions, its stage position, and the invariants a local reader cannot infer from types. That map decides what needs a comment. Do not copy architecture documents into code.

## Module entry points

States the single responsibility, names important input and output data, explains how files divide the work, identifies exclusions and neighbours, and describes the main flow in execution order when it orchestrates steps. Uses current file and type names. Does not claim authority over behaviour owned elsewhere.

File a finding when a reader cannot tell where to start or what the module owns.

## File headers

Concise WHAT/WHY where the role is not obvious from module and filename. States what the file owns, important exclusions, and how it fits the wider stage. Names downstream consumers when they constrain the data shape. Does not repeat the filename or list every type.

A tiny file whose purpose is obvious needs no header. Do not require boilerplate.

## Types and data structures

Comments on important structs, enums, IDs, tables and result types should explain semantic ownership, lifecycle and mutability, identity domain and valid comparisons, alignment between parallel structures, whether the data may cross a module, stage, thread or persistence boundary, whether it is authoritative or derived, and why an unusual representation is safe.

Reject comments that restate field names.

## Complex flow

Landmarks explaining the overall operation, major phases and their required order, why one phase precedes another, where data changes authority or representation, why a fallback exists, why independent branches may or may not continue, where deterministic ordering is restored after parallel work, and why a no-op branch is intentional.

Comments support a readable flow. They do not compensate for an unreadable function - route that to Style.

## Non-local intent

The highest-value comments. Where correctness depends on facts outside the block: producer and consumer contracts, stable identity and remapping requirements, why source is not rescanned, why data cannot cross a boundary, why validation is deliberately conservative, why optional proof falls back rather than rejecting, why a path is serial despite available parallelism, why a clone is currently necessary, why similar-looking code stays separate.

Give the local reason. Do not write history essays - Git has those.

## Invariants and failure paths

Non-obvious invariants around unchecked indexing, `.unwrap()`, `unreachable!`, unsafe code, table alignment, graph acyclicity, phase transitions, cache keys, remapping, ownership facts and target assumptions.

A comment does not legalise a weak invariant. Where the code cannot enforce what the comment claims, file a linked Correctness or Style finding.

## Noise to remove

Comments narrating the next line, paraphrasing a descriptive name, repeating a type signature, labelling trivial getters or loops, restating canonical docs without local relevance, banner-separating minor blocks, preserving history Git records, TODOs with no owner or current relevance, and vague claims - "for safety", "for performance", "temporary" - that never name the constraint.

Prefer deletion when the code already says it. Prefer a rename when the comment exists only because the code is obscure.

## Staleness

A stale comment is a finding even when the code is correct, because it actively misleads. Look for renamed types, stages or modules; descriptions of deleted passes; stale ordering claims; ownership assigned to the wrong stage; claims that a value is copied, cached or immutable when it no longer is; and references to old paths or plans.

## Stale when

Module ownership, principal control flow, stage ordering, important types or invariants, or the terminology the comments use changes materially.
