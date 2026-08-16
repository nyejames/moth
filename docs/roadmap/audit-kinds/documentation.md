# Documentation Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. A documentation audit follows the repository authority hierarchy. It does not treat every document as equally authoritative.

A documentation audit is read-only. It records accuracy, authority, status, routing, terminology, example and navigation findings. It may recommend documentation-only corrections. Any semantic or architectural change remains design-gated.

## Purpose and boundary

Use this audit to answer whether contributors and users can find the correct authority and whether every document accurately reflects accepted design, current implementation status and repository structure.

The audit covers:

- canonical compiler, build, memory and language authorities
- design-scope documents
- progress matrix and roadmap status
- plans and deferred-work ownership
- contributor and agent routing
- implementation maps and `index.md`
- public teaching pages, README and cheatsheets
- examples, links and terminology
- generated documentation ownership
- duplicated or conflicting documentation authority

Route these concerns elsewhere:

- implementation comments and file doc comments -> Comments
- code that contradicts correct documentation -> Correctness
- test coverage that contradicts progress claims -> Tests
- diagnostic wording inside compiler output -> Diagnostics
- implementation readability -> Style
- duplicated production ownership -> Redundancy

## Valid scopes

- One canonical document plus every document that depends on it is a valid scope.
- One code leaf or composite scope plus its implementation maps, progress entries and public docs is valid.
- A contract scope is valid for cross-document ownership such as compiler versus build-system handoff.
- A comparison scope is valid for Basic versus Advanced pages, source versus generated docs or end-state docs versus current status.
- One isolated page may produce a partial finding but cannot prove authority consistency across a wider subject.

A complete documentation audit must identify the authority owner and inspect every in-scope dependent document, not only the most visible page.

## Audit procedure

### 1. Map the authority hierarchy

For the selected subject, identify:

- the canonical architecture or language authority
- any more specific topic authority
- design-scope documents for accepted deferred, open or excluded surfaces
- the progress-matrix row for current implementation
- roadmap or plan ownership for sequencing
- testing and validation standards where coverage or completion is discussed
- public teaching pages and examples
- implementation maps and contributor routing

Record which document owns:

- accepted semantics
- current status
- future sequencing
- contributor process
- public explanation
- generated presentation

A document can explain another authority without becoming its replacement.

### 2. Check authority declarations and routing

Verify that documents clearly state:

- what they own
- what they do not own
- prerequisite reading
- relevant companion authorities
- whether they describe accepted end state or current implementation
- how to route narrower tasks
- where deferred and design-pending work belongs

Check for:

- two documents claiming the same canonical contract
- missing or circular routing
- a teaching page presented as architecture authority
- plans overriding accepted design
- progress pages presented as end-state design
- README or examples used as the only source for semantics
- skills or agent instructions duplicating repository policy instead of routing to it

### 3. Compare canonical documents for consistency

Read all canonical owners touched by the subject.

Check that:

- terminology and stage numbering agree
- producer and consumer boundaries agree on data shape and ownership
- compiler and build-system documents assign each responsibility once
- language and memory semantics agree
- public interface, HIR, borrow, lifetime and backend handoffs agree
- config, graph, package, entry, link and output ownership agree
- target and backend rules do not conflict
- accepted exclusions and deferred surfaces are consistent
- examples do not imply behaviour the prose rejects

When canonical authorities conflict, record the exact incompatible statements. Do not silently choose or rewrite one as if the design decision were obvious.

### 4. Check accepted end state versus current status

Compare canonical design with the progress matrix.

Verify that:

- supported rows describe behaviour the current compiler actually claims
- partial rows name the implemented subset and important gaps
- experimental rows do not imply Alpha stability
- deferred rows do not appear as implemented public behaviour
- accepted deferred features are not mistaken for design-pending work
- outside-scope features are not listed as roadmap gaps
- backend coverage matches actual target support
- test coverage labels match the suite at a useful high level
- known conservative behaviour is described without presenting it as final ideal behaviour

A code defect found during this comparison becomes a linked Correctness finding. A stale status entry remains a Documentation finding.

### 5. Check roadmap and plan ownership

Inspect roadmap references for the selected subject.

Check that:

- active, queued and deferred work is placed in the right section
- plan order reflects the accepted dependency chain
- design-gated work is clearly blocked
- completed plans no longer appear active
- a plan does not redefine canonical semantics
- one plan owns each implementation sequence
- historical plans are labelled as such when retained for architecture context
- deferred follow-ups link to their current owner
- removed or split plans no longer leave duplicate roadmap entries
- speculative syntax is not promoted by roadmap wording

Do not turn a documentation audit into plan design. Record design proposals as blocked findings.

### 6. Check implementation maps and repository paths

Verify every in-scope path and module reference.

Check that:

- implementation maps point to current owners
- moved or renamed modules are updated
- `index.md` remains a locator rather than a competing design authority
- file and directory names match the repository
- links use the correct relative path
- anchor names still exist
- examples of module layout match current structural rules
- generated output paths are not cited as source owners
- old language, extension, diagnostic or package names are removed

Where possible, search the repository for old paths and terminology rather than checking only visible links.

### 7. Check public teaching pages

For Basic, Advanced and topic pages, verify that:

- they teach the canonical contract accurately
- they clearly mark accepted deferred or target-limited behaviour
- they do not invent design-pending syntax
- examples use current Moth syntax and project structure
- restrictions and deliberate limits are not hidden when they affect ordinary use
- simpler pages link to detailed owners instead of becoming incomplete authorities
- advanced pages cover non-obvious edge cases without duplicating entire canonical references
- terminology remains consistent with compiler diagnostics and the cheatsheet
- examples are self-contained enough to understand
- example names and prose follow current documentation style

A teaching simplification is valid only when it remains true.

### 8. Check the language cheatsheet and compact references

When the scope includes compact references, verify that:

- every accepted syntax form is represented accurately
- status labels distinguish accepted deferred from design pending
- no old, fake or speculative syntax remains
- common invalid translations are correct
- unusual semantics such as reference behaviour, mutability, errors, templates and module structure are explicit
- examples cover complex forms that are not obvious from isolated tokens
- compact wording does not erase important restrictions
- repeated explanation can be condensed without losing the rule
- links point to detailed authorities
- the token-efficient form remains searchable and structured

The cheatsheet describes accepted end state and must direct readers to the progress matrix for current support.

### 9. Check README and project-level summaries

Verify that project summaries:

- describe Moth at the right level
- do not claim unsupported production readiness
- use current command, extension and package names
- link to canonical compiler, build, memory and language docs
- distinguish goals from implemented features
- avoid becoming the only description of an important contract
- keep install and command examples current
- reflect the current backend focus and development status
- avoid stale marketing claims contradicted by progress status

README prose may be selective. It may not be false.

### 10. Check examples and code blocks

For every in-scope example, check:

- syntax against the canonical language reference
- current versus accepted-deferred status
- imports, paths, module roots and config shape
- mutability, access, error and return syntax
- target and builder assumptions
- omitted declarations that make the example misleading
- expected output where one is claimed
- invalid examples are clearly labelled invalid
- code fences and language tags are correct
- old names or compatibility forms are absent

Run executable examples through the documentation gate where the documentation system owns compilation. Do not claim an example compiled when it was only inspected.

### 11. Check terminology and naming drift

Search for:

- old project or language names
- old file extensions
- removed keywords and syntax
- renamed stages, IRs, types and modules
- inconsistent use of module, package, binding, builder and backend
- "library" used where a strict package category matters
- old diagnostic prefixes or command names
- obsolete memory terms such as explicit moves or lifetimes
- current implementation names presented as permanent architecture
- different names for the same semantic fact across producer and consumer docs

Prefer canonical domain terms. Keep implementation-specific names labelled as current navigation aids where they may change.

### 12. Check restrictions, exclusions and open questions

Verify that documents distinguish:

- accepted and implemented
- accepted but deferred
- experimental
- design pending
- deliberately outside scope
- possible future follow-up

Check that:

- outside-scope features are not suggested as ordinary roadmap work
- design-pending areas contain no invented examples
- accepted deferred syntax has one canonical owner
- unresolved questions are not written as settled rules
- conservative current limitations are not promoted to permanent design unless accepted
- future notes do not contradict the language's design principles

### 13. Check progress coverage claims

When coverage is discussed, compare with:

- `tests/cases/manifest.toml`
- integration-suite audit output when available
- subsystem unit and harness tests
- backend expectations
- recent accepted test findings

Verify that Broad, Targeted, Thin and None labels are defensible. Do not copy long fixture lists into the matrix when a concise contract summary is enough.

A missing test becomes a linked Tests finding. The documentation finding owns only the inaccurate coverage claim.

### 14. Check documentation ownership and duplication

Look for the same detailed rule repeated in several places.

For each repetition, decide whether to:

1. keep a compact summary plus link
2. move the detailed contract to the canonical owner
3. retain separate teaching examples with no duplicated authority claim
4. remove obsolete copies
5. leave repetition because each audience needs a complete local rule

Check that:

- overview files route rather than duplicate every detail
- topic files remain useful when read directly
- `@page.moth` owns presentation rather than detailed design
- audit skills route to audit documents rather than copying their policy
- generated documentation is produced from source and not edited directly

### 15. Check links, navigation and generated output

Verify:

- relative links resolve from the source document
- headings and anchors are stable
- navigation includes new canonical pages where required
- circular links do not trap readers
- generated release files are not manual authorities
- source changes produce the expected generated route
- tables, code blocks and templates render correctly
- hidden or orphan pages remain intentionally accessible or are linked
- redirects or old paths do not preserve obsolete structure without reason

### 16. Review prose quality without changing meaning

Check that prose:

- uses direct active language
- states rules before caveats
- separates semantic contracts from examples
- avoids filler, repeated conclusions and historical narration
- uses hard `must` only for real requirements
- avoids vague qualifiers and unsupported claims
- remains concise without dropping restrictions
- uses British English and current documentation conventions
- does not over-explain trivial syntax while skipping unusual behaviour

A substantial rewrite is valid only when the same meaning can be verified against the owner.

### 17. Classify the finding

Every documentation finding must be one of:

- **Accuracy correction** - prose or example conflicts with the accepted contract.
- **Non-semantic clarification** - the contract is correct but ambiguous or hard to apply.
- **Status correction** - progress, backend or coverage state is stale.
- **Navigation or ownership correction** - routing, links, paths or authority claims are wrong.
- **Design proposal** - the accepted contract itself may need change.

Design proposals remain blocked until explicitly approved. Do not implement them as documentation cleanup.

### 18. Form the finding

A documentation finding must state:

- affected documents and authority levels
- the canonical source of truth
- exact conflicting, missing or ambiguous content
- current implementation status where relevant
- proposed documentation-only correction
- whether examples or generated output need rebuilding
- linked Correctness or Tests findings
- whether the issue is design-gated

## Valid findings

Valid documentation findings include:

- conflicting canonical contracts
- stale progress or roadmap status
- wrong ownership or task routing
- inaccurate examples or unsupported syntax
- broken links and stale implementation maps
- teaching pages that omit a critical restriction
- compact references that over-compress the rule
- duplicated authority that can drift
- old terminology or project names
- generated output edited or cited as the source owner

## Kind-specific preservation rules

A documentation fix must preserve:

- accepted language, compiler, build and memory design
- production code, tests and artefacts
- current implementation status unless the finding proves the status text stale
- roadmap sequencing unless explicitly approved
- authority hierarchy

A documentation change cannot make incorrect code correct or settle an open design question.

## Freshness invalidators

Mark a documentation audit stale when the scope receives material changes to:

- canonical design or terminology
- progress-matrix status or coverage
- roadmap sequencing or plan ownership
- file, module or directory structure
- public teaching pages, examples or cheatsheet content
- contributor or agent routing
- generated documentation structure

A production refactor that leaves paths, status and documented contracts unchanged does not automatically stale documentation.

## Completion checklist

A complete documentation audit confirms that:

- authority ownership was mapped
- canonical documents were checked for consistency
- current status and roadmap state were checked
- implementation maps, links and paths were verified
- teaching pages, README, compact references and examples were checked where relevant
- terminology, restrictions, deferrals and open questions were checked
- progress coverage claims were compared with real test ownership
- duplicate authority and generated-output ownership were reviewed
- prose corrections preserve meaning
- every design proposal was separated and blocked
- implementation and test defects were routed into linked findings
