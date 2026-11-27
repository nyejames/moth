# Documentation

Find inaccurate, incomplete, ambiguous, bloated or difficult-to-read documentation. Moth's documentation should give humans and agents a precise understanding of the language, its design and its implementation without making them reconstruct that knowledge from the codebase.

Prioritise design drift and consequential omissions before editorial improvements. Maximise useful information while reducing the effort needed to read, navigate and maintain it. A longer explanation earns its place when it improves understanding.

Follow the [audit guide](../audit-guide.md) for formal findings, scope, coverage and allowed writes. Read the [codebase style guide](../../developer-docs/style-guide/style-guide.mtf) and use its documentation standards. An audit proposes corrections for separate implementation. It does not rewrite the documents under inspection or approve changes to design.

## 1. Establish purpose, audience and authority

Identify what each document owns, who needs it and what a reader should understand after reading it. Judge the document against that purpose rather than applying one length or level of detail to everything.

- The [compiler design](../../compiler-design-overview.md) and [build-system design](../../build-system-design.md) specify accepted architecture, responsibilities, design rationale and cross-system contracts. They describe the intended end state, including accepted work that remains unimplemented.
- Detailed language, memory and subsystem references own exact contracts, restrictions and edge cases. Follow the [language reference index](../../developer-docs/language/overview.mtf) and the relevant specialist authorities.
- Basic and educational pages teach a truthful subset of the language through explanations and examples. They do not replace canonical references.
- Overview and routing documents orient readers and direct them to the relevant detailed authorities. They should not become another exhaustive specification.
- The [roadmap](../roadmap.md) and its plans record sequencing, implementation work and unsettled design. Preserve unique approved decisions in permanent owners before retiring a plan.
- The [progress matrix](../../docs/progress/@entry.moth) records current implementation, test coverage and backend support, not the accepted end state.
- Contributor and process guides define actionable standards. Agent instructions and skills should route to repository policy rather than maintain competing versions.

Apply the authority order in [AGENTS.md](../../../AGENTS.md). Check explicit approved supersession within its stated scope. A newer plan, passing test or existing implementation does not by itself override a canonical contract.

Check whether documents clearly distinguish accepted design, current behaviour, accepted deferred work, unsettled proposals and excluded features. A bounded V1 design is not a claim about implementation progress.

## 2. Investigate design drift first

A documentation-only reading cannot establish agreement with the compiler. Inspect the actual code behind each significant claim about implemented behaviour, including the producer and relevant consumers. Use `index.md` and module entry points to locate the implementation, then follow the real data and control flow.

For large design documents, choose a coherent section or subsystem and its related handoffs. Finish the drift investigation for that scope before spending time on wording. Expand a broad review through bounded passes rather than recommending a simultaneous rewrite of unrelated sections.

For each contract:

1. Read the canonical authority, relevant companion documents and the applicable status or active plan. Establish what Moth intends to support and what it claims to support now.
2. Locate the implementation and trace the operation from its entry point to its observable result. Check alternate paths, relevant feature gates, builders, targets and failure cases.
3. Compare the actual stage ownership, ordering, data representation, validation and handoff with the documented contract. Read the relevant tests and use a focused reproduction when the outcome is uncertain.
4. Search for recent approved changes that have not reached permanent documentation, including completed migrations, retired terminology and former implementation restrictions described as permanent rules.
5. Establish whether the difference is a defect, a legitimate support gap, an approved design change missing from documentation or a question that still needs a decision.

### Classify the discrepancy

- **Implementation defect:** the code violates accepted design. Record the bug in [bug reports](../../bug-reports.md). Do not change the design to legitimise current behaviour.
- **Outdated design text:** an established, approved decision supersedes the passage. Cite that decision and propose updating the permanent owner and affected references.
- **Incorrect status:** the progress matrix or public availability claim differs from demonstrated support. Propose a status correction with the applicable target and test evidence.
- **Legitimate deferral:** the implementation lacks an accepted future feature. Retain its end-state contract and check that current-use documentation marks the limitation accurately.
- **Conflicting authorities:** two documents specify incompatible rules. Identify both statements, their owners and any explicit supersession. If that evidence cannot resolve the conflict, block the proposal on a design decision.
- **Unsettled behaviour:** code or prose makes a choice that accepted design does not establish. Describe the competing interpretations and their consequences. Seek approval rather than adopting one silently.

A Documentation finding owns the defect in the prose. Log a separate code or test defect as an incidental lead in `docs/bug-reports.md`, without opening another audit or extending coverage. Distinguish confirmed defects from unproven suspicions. Keep one active tracker per unresolved root cause.

## 3. Look for missing context and ambiguous contracts

Check what a reader can learn from the document, not merely whether its individual sentences are true. A developer familiar with compilers should be able to build a coherent picture of Moth's architecture and understand the decisions that distinguish it from other compilers.

Ask:

- What does the system do, why does Moth use this design and where does the responsibility belong?
- When does the operation run? What precedes it, what does it produce and which consumers depend on its result?
- Are input and output contracts, ownership, identity, lifetime, ordering, failure and publication rules clear wherever they affect the design?
- Are important exclusions, conservative decisions and restrictions explicit? Does a broad statement conceal a meaningful exception?
- Does the text distinguish similar concepts with different meanings, such as source legality and target support, semantic identity and physical representation, or graph membership and executable reachability?
- Does it explain unusual architectural choices and their consequences instead of assuming the reader will infer them from a conventional compiler model?
- Could a reasonable reader interpret an ambiguous passage in two materially different ways? Which interpretation does the accepted contract establish?

Pay particular attention to Moth's early semantic work during AST construction, template composition and folding, generic materialisation, compiler/build orchestration, package boundaries, resource values, lifetime analysis and backend handoffs. Follow the relevant authorities rather than treating these examples as an exhaustive list of subjects.

Check agreement between the producer and consumer descriptions of each boundary. A summary should remain accurate even when it omits the full algorithm. Add a focused explanation or example when a reader needs context that a link alone would not provide.

If the existing authorities do not settle an ambiguity, record the question and the consequences of each plausible answer. Do not invent missing rationale or a new semantic rule to make the text read more confidently.

## 4. Challenge information density and redundancy

Read each passage in its surrounding section and compare it with the documents that own the same subject. Look for opportunities to remove reading and maintenance work without losing technical precision.

- Does each paragraph contribute a fact, rationale, example, constraint, useful distinction or orientation?
- Can several sentences or sections be combined without losing a qualification or making the flow harder to follow?
- Does an overview duplicate an entire detailed contract or internal API inventory that another document owns?
- Has a design guide accumulated obsolete implementation history, migration instructions, temporary limitations or routine Rust details that do not explain its accepted architecture?
- Would a short example, ordered sequence or diagram explain a complex relationship more clearly than repeated abstract prose?
- Could a concise explanation plus a direct link replace duplicated details while keeping the local section understandable?

**Preserve useful repetition.** Architectural invariants often need a compact statement near the start of a document and a contextual explanation at the affected stage or handoff. A repeated rule may explain a different consequence, introduce a useful example or make an independently read section understandable.

For each consolidation candidate, identify the unique facts and context in every passage. Decide what to retain, delete, combine, move to its canonical owner or summarise with a link. Make sure every restriction, exception, rationale and useful example survives where a reader needs it.

Token efficiency matters for agents, but word count is not a quality metric. A dense paragraph can cost more reading effort than a longer, clearer explanation. Avoid stripping design rationale, turning complete prose into shorthand or hiding essential context behind a chain of links.

## 5. Review structure and educational quality

Follow the document as its intended reader. A compiler expert learning Moth, a programmer learning the language and an agent looking up a precise contract need different levels of explanation.

- Introduce the concept and its purpose before relying on unfamiliar terminology or internal details. Define an abbreviation when readers need it.
- Present a process in dependency or execution order when the order matters. Keep related facts and qualifications together.
- Break difficult concepts into meaningful steps, short paragraphs, lists or worked examples. Avoid both dense walls of text and excessive fragmentation into tiny sections.
- Prefer concrete examples that expose a useful distinction, restriction or interaction. Use analogies only when they clarify the concept without implying false semantics.
- Keep headings descriptive and navigation predictable. A reader entering through a linked section should find enough local context to understand the rule and locate its prerequisites.
- Check that public explanations build understanding progressively. Simplification must remain truthful, including when an edge case affects ordinary use.
- Keep diagrams and other supporting material aligned with the text. Check arrow directions, ownership, stage order and labels rather than treating visual polish as proof of accuracy.

For Moth documentation sources, follow the existing division between `@entry.moth` for website introductions, navigation and composition, `overview.mtf` for compact routing and contract references, and topic-named `.mtf` files for detailed content. An overview is not a public page import. Topic files should remain useful when read directly.

## 6. Review navigation, links and terminology

A reference should help readers reach the right information, not merely resolve to an existing file.

- Verify paths, headings, fragment IDs, named reading routes, imports and relevant rendered links. Check the destination still explains the referenced subject.
- Search for removed language forms, old filenames and extensions, renamed modules or stages, outdated diagnostic identities and obsolete commands, not only visible broken links.
- Link architectural explanations to relevant public language or project documentation under `docs/docs/`, where examples and accessible explanations help establish context.
- Link teaching pages and compact references to their detailed semantic authority. Avoid forcing readers to traverse several unrelated pages to find the complete rule.
- Identify important implementation entry points, types or module owners where they clarify a design boundary. Verify those locations against the current repository. Leave routine source inventories to `index.md` and local module documentation.
- Check task-reading guides for missing or outdated routes, overbroad reading requirements and headings that no longer own the task.
- Use one name for each technical concept. Preserve the distinction between concepts whose names happen to be similar.

Check links in the authored format, including Moth Markdown link syntax and explicit section IDs where relevant. Generated content under `docs/_release/**` is evidence of presentation, not a canonical source. Never edit it directly.

## 7. Audit writing quality

Use Moth's documentation standards and the [Rust Book](https://doc.rust-lang.org/book/) as a model for clear technical explanation. Write for a technically curious reader without adopting corporate language or unnecessary formality. Follow the established tone of the surrounding resource.

### Language and tone

- Use British English, descriptive technical terms and natural, grammatical sentences.
- Prefer active constructions, concrete verbs and short familiar words. Name the responsible system or actor when that clarifies ownership.
- Remove filler, hollow intensifiers, stacked auxiliaries, vague qualifiers, unnecessary hedging, nominalisations and generic transitions.
- Remove redundant introductions, recaps and meta-commentary about the document. Move on once a point is established.
- Avoid jargon for its own sake, marketing language, cliches, strained analogies and distracting cultural references.
- Avoid semicolons, Oxford commas and em dashes in prose. Use straight apostrophes and basic ASCII punctuation. Use natural contractions and vary sentence length without sacrificing clarity.
- Preserve exact identifiers, code syntax, quotations and necessary source spellings. Link or cite external technical sources where they supply evidence or further useful context.

### Precision takes precedence

Apply the writing rules to improve meaning, not mechanically eliminate words. Definitions may need forms of "is". Passive voice may suit a contract where the actor is irrelevant. Keep qualifications that express actual uncertainty, conservative analysis, permitted alternatives or limited support. Use `must` and `must not` for binding requirements rather than softening them into preferences.

For example, "It is important to note that the AST stage is responsible for the resolution of expression types prior to HIR lowering" can become "AST resolves expression types before HIR lowering". The shorter sentence keeps the owner and ordering. Removing a qualification such as "cannot prove legal" from a conservative validation rule could instead change its meaning.

Check each proposed editorial improvement for a concrete gain in precision, comprehension or information density. Do not file a finding merely because a different synonym or sentence rhythm is possible.

## 8. Verify examples and claims of support

Read examples against the exact unsuffixed language reference and current status. Check imports, file and module roles, syntax, receiving types, mutability, output, command flags and target support where relevant.

- Positive examples should exercise the feature they teach. Show the expected result when it helps a reader understand the behaviour.
- Label intentionally invalid examples, accepted but unimplemented features and target-limited behaviour clearly. Check the reason an invalid example fails, not just that it fails.
- Check supported status rows against the implementation and relevant tests or backend lanes. Partial rows should name the implemented subset and important gaps. Use `tests/cases/manifest.toml` to locate integration coverage, then inspect the selected assertions rather than inferring coverage from case names.
- Distinguish executable documentation from quoted code and non-compiled examples. A successful documentation build does not prove that a fenced snippet compiles or behaves as described.
- Use current compiler and focused test commands when practical. Preserve the minimal reproduction and actual result when a finding depends on execution. Report which examples were read, checked, compiled or executed.
- Check that the rendered page presents the intended headings, links, examples and supporting material.

Follow the [validation guide](../../developer-docs/style-guide/validation.mtf) for current commands and gate scope. During discovery keep tracked source, fixtures and generated documentation unchanged. Use existing evidence or non-writing checks, and isolate any disposable reproduction. The separate correction task runs the documentation release-build gate at the required checkpoint and reviews the generated output. Never claim validation that did not run.

## 9. Recommend bounded, conservative revisions

A documentation audit proposes individually reviewable improvements. It does not authorise broad rewriting or changing accepted design under the label of cleanup.

Prefer a small correction to the owning passage, consolidation of demonstrably overlapping material or a targeted addition of missing context. Restructure a section only when its current organisation creates a concrete comprehension, navigation or maintenance problem.

For large architecture documents, group related findings by section or contract. Keep editorial changes separate from semantic corrections and unresolved design proposals. The later implementation task should apply small coherent slices, check the affected companion documents and compare the revised passage with its original meaning.

Before recommending deletion or replacement, account for the existing technical information and state where it will survive. A clearer explanation must retain meaningful conditions, ordering, exceptions, restrictions, guarantees and deliberate exclusions. An example that carries unique context should remain unless another example or explanation conveys it better.

## 10. Evidence, findings and coverage

Use the formal report schema. Name the exact document, heading and affected passage, the reader or contract affected, the concrete problem, the smallest plausible correction and how to verify it. Include the strongest reason to keep the current wording or structure and why it fails.

Supply evidence appropriate to the finding:

- **Drift or factual error:** conflicting claims, canonical authority, implementation locations and relevant status, decision or reproduction evidence.
- **Ambiguity or omission:** competing interpretations or missing information, why they matter and the authority that establishes the correction.
- **Redundancy or verbosity:** overlapping passages or unnecessarily dense wording, the facts that must survive and a proposed consolidation or clearer replacement.
- **Structure or explanation:** a specific obstacle to understanding or navigation, with a concrete alternative suited to the reader.
- **Example or reference:** the inaccurate example or destination, expected meaning and the verified replacement or actual observed behaviour.
- **Design proposal:** the precise unresolved question, conflicting evidence and consequences of the available choices. Keep it blocked until approved.

For straightforward wording findings, a short before-and-after example may provide all the evidence needed beyond its location and the reason for the change. Preference alone is not a finding. Group recurring problems with the same cause instead of cataloguing every minor stylistic variation.

Record the sections, companion authorities, implementation paths and examples actually inspected. Distinguish context reading from audited coverage. Mark broad reviews partial when relevant contracts, implementation paths or examples remain unchecked. Prioritise substantive drift and missing design constraints over low-value editorial polish.

## Stale when

Documentation coverage becomes stale when accepted contracts, support status, implementation ownership or data flow described in the text, terminology, referenced paths, examples, document structure or contributor routes change materially. A production refactor that leaves the documented claims, status and navigation intact does not invalidate coverage.
