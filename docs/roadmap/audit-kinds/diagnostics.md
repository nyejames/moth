# Diagnostics

Audit the quality, coverage and construction of compiler diagnostics. Look for user mistakes that Moth correctly identifies but reports unclearly, incompletely, inaccurately or at the wrong location. Also look for missing diagnostic cases, unhelpful fallbacks, wrong failure lanes and misleading cascades.

A good diagnostic tells the developer **what failed, why it failed and what to do next when the compiler can establish a valid next step**. Its code, source attribution and presentation must remain reliable across compiler stages and commands.

Source that the compiler incorrectly accepts or rejects belongs to **Correctness**. Record that defect in [`docs/bug-reports.md`](../../bug-reports.md) without expanding this audit. Weak or missing test assertions belong to **Tests** and can be logged there as incidental leads rather than counted as separate diagnostic defects without evidence.

**Authorities:** `docs/compiler-data-layout-design.md` owns the three failure lanes, diagnostic schema, physical layout and identity lifetimes. `docs/compiler-design-overview.md` owns semantic stages and recovery. `docs/build-system-design.md` owns graph outcomes, scheduling and command boundaries. The codebase style guide and testing guide govern implementation and fixtures. Follow these owners instead of copying their detailed representation contracts into this guide.

## Diagnostic quality

### Clear and specific messages

- State the actual failure in plain language. Name the violated rule, missing requirement or incompatible operation when known.
- Include the relevant authored name, token, operator, type, value or capability when it clarifies the failure. Distinguish expected from found where the comparison matters.
- Explain the constraint behind the failure. "Invalid syntax", "type error" and "unsupported operation" are inadequate when the producer knows a more precise reason.
- Lead with the problem. Keep messages short, grammatical and direct. Prefer one useful sentence to a paragraph of generic advice.
- Use the same terms for the same Moth concepts. Render source spellings as code and keep compiler IR, internal IDs, private type names and implementation details out of user-facing prose.
- Keep the message, labels and guidance complementary. Do not repeat expected/found types or the same instruction across all three.
- Match the compiler's actual certainty. A conservative analysis may say that legality could not be proven without claiming a definite conflict that has not been established.

Compare:

**Weak:** `Invalid assignment target.`

**Better:** Cannot update `count` because its binding is immutable.

**Related label:** `count` was declared here.

**Guidance:** Declare `count` as mutable if it needs to change.

The example separates the cause, its origin and a supported correction. For a call mismatch, "Type mismatch in call" gives less context than "Cannot pass String to parameter `enabled` (Bool)", assuming the compiler knows the parameter name. Repeating "Expected: Bool" and "Found: String" below that message adds nothing.

### Helpful, trustworthy guidance

- Offer a concrete correction when semantic facts establish one. Prefer a valid Moth form or a precise change over "check your code".
- If multiple remedies are valid and the compiler cannot infer intent, explain the constraint or present concise supported alternatives without arbitrarily choosing one.
- State why an unavailable feature cannot be used when that distinction helps. Suggest an alternative only if Moth currently supports it.
- Never suggest an unaccepted language feature, an invalid rewrite, a potentially behaviour-changing conversion presented as harmless or a fix that only applies to another reason variant.
- Make "did you mean" suggestions only from verified candidates in the applicable name-resolution context. Similar spelling alone does not establish that the candidate is accessible or appropriate.
- Keep automated fixes stricter than prose hints: a machine-applicable edit must preserve the intended source ownership and have an unambiguous, valid replacement. Otherwise provide advice without claiming an automatic fix.
- Avoid placeholder guidance, speculation and generic documentation referrals when the compiler can explain the rule directly.

**Quality rule:** every diagnostic must explain its failure clearly. Actionable guidance is required whenever a supported correction can be established. A clear constraint explanation is preferable to a fabricated suggestion when no trustworthy correction follows from available facts.

### Source spans and related context

- Give every diagnostic owned by authored source an exact `SourceSpan`, with the correct source identity and half-open UTF-8 byte range. Highlight the smallest useful construct, not an entire statement or file when a narrower owner exists.
- Place the primary span where the developer should investigate or change the source first. Use related labels for the declaration, prior assignment, conflicting item, import or call site that establishes the constraint.
- Explain each related location's role. Unlabelled highlights or repeated generic labels often leave the relationship unclear.
- For duplicate declarations, collisions, ambiguous resolution, aliases and cross-file failures, report all materially relevant sites required by the owning semantic contract. Keep their ordering deterministic and their labels concise.
- When one error has many related sites, preserve the semantic completeness required by its contract without flooding the primary message. Use bounded presentation where the architecture allows it, rather than silently choosing an arbitrary conflict.
- Source-less project or host failures may have no span. Never fabricate provenance. A file-level span is acceptable only when no narrower relevant source range exists.
- Retain accurate provenance for provider files, generated inputs and reports whose primary and related sites belong to different identity contexts.

## Coverage and specialisation

Auditors must inspect the **conditions handled by a diagnostic**, not only the wording of its current renderer. A well-written generic message can still hide missing cases.

Look for differences in:

- The violated language rule or semantic cause.
- The operation, declaration kind, mutability or access context.
- The expected token, type, arity, receiver or capability.
- Local, imported, generated and cross-module provenance.
- The relevant declaration or other source location needed to explain the failure.
- The supported correction and whether the suggested fix is valid in each case.
- Proven-invalid behaviour versus behaviour that conservative analysis cannot prove legal.
- Recognised but deferred functionality versus functionality deliberately outside Moth's design.

Apply the smallest effective change:

1. **Improve rendering** when a diagnostic already carries the facts needed to explain the case.
2. **Extend typed facts or a reason enum** when one durable semantic family needs distinct explanations, labels or guidance. Capture the missing information at the semantic owner.
3. **Introduce a separate diagnostic family and stable code** when cases represent materially different rules or durable semantic identities, not merely different wording.
4. **Keep a generic fallback** only for conditions whose cause genuinely cannot be determined more precisely. It must still explain what the compiler knows.

Do not create a new code for every sentence, duplicate constructors to vary prose or infer the missing distinction from rendered text. Equally, do not force unrelated corrections into one catch-all reason. Review every caller and reason branch when changing a shared family so guidance remains valid for all of them.

### Finding missing cases

- Trace validation branches that return no diagnostic, reuse a broad "invalid" variant or collapse different conditions into one fallback.
- Compare related syntax and semantic paths: ordinary versus generic calls, mutable versus immutable access, local versus imported names, direct versus generated code and supported versus unsupported targets.
- Review common malformed-input permutations: missing, extra, misplaced and wrong-kind tokens, plus nearby valid spellings likely to produce the mistake.
- Check whether an earlier failure removes the semantic context needed for a useful later message. Improve the owning producer or recovery boundary instead of reparsing source in the renderer.
- Test invalid inputs adjacent to legal boundaries, including empty constructs, nested syntax, conflicts across files and capacity limits.
- Distinguish unreported invalid source from poor reporting. Wrong legality belongs in the Correctness bug queue even when adding a diagnostic forms part of its fix.

## Compiler construction and ownership

### Failure lanes

- Source, project, configuration, dependency, type, borrow, rule and target-contract failures use typed `DiagnosticDraft` values. Warnings and deliberately deferred features use the same diagnostic machinery.
- Expected filesystem, host, provider, output and tooling failures use typed `InfrastructureFailure` causes in the outer `InfrastructureErrors` lane. Classify by the cause, not by which subsystem detected it.
- An internal state that has not been proven impossible also uses `InfrastructureFailure`. Only a genuinely proven invariant violation calls `compiler_bug!`, with the guard and owning stage identified.
- User-controlled malformed input never reaches `panic!`, `todo!`, `unreachable!`, an unjustified `.unwrap()` or a compiler-bug path. Representable capacity exhaustion from user input is a diagnostic.
- Classify missing or invalid user-authored dependency targets by their language/build contract. A physical failure to read an otherwise valid source or write output belongs to the operational lane.
- Keep compiler diagnostics separate from runtime Moth `Error` values, entry-startup failures and the standalone MON service's public errors.
- A diagnosed module never publishes a partial semantic interface. Infrastructure failure and compiler bugs follow the owning outcome contract rather than being converted to ordinary source diagnostics.

A wrong lane that changes compilation outcomes belongs in the Correctness bug queue.

### Typed facts, identity and rendering

- Declare each user-facing diagnostic family once in the diagnostic schema. Keep its internal identity, external stable code, category, default severity, constructor, fact contract, permitted labels and renderer aligned.
- Treat one code as one durable semantic family. Preserve the code when improving wording or specialising reasons within that family. Never reuse it for an unrelated failure.
- Construct `DiagnosticDraft` at the owner with the best semantic facts. Share a typed constructor or narrow helper across callers rather than copying diagnostic prose through low-level code.
- Retain `TypeId`, symbols, `StringId`, `PathId` and relevant semantic reason/context enums as structured facts until the owning capture and render boundaries. Avoid pre-rendered types, cloned `PathBuf`s, arbitrary strings and full type environments retained only for diagnostics.
- Accumulate local drafts through `DiagnosticBag` and the established prepared-batch path. Freeze compact reports at the owning boundary with the necessary type-display snapshot and immutable source/identity context.
- Remap worker-local identities before downstream access. Preserve exact source ownership when a related site comes from a different report context. Never use numeric ID equality to infer shared ownership.
- Renderers decode typed facts without guessing semantic meaning from message text, rescanning source, reopening an earlier IR, changing report data or branching on text to recover lost reasons.
- Preserve the compact common diagnostic representation. Unusual or variable-length data belongs in schema-approved typed side storage, not a widened common record or an extra wrapper for every diagnostic.
- Retain each occurrence required by the semantic contract. Do not silently deduplicate independent failures, and do not repeat a single diagnosed provider failure at every blocked consumer.

### Recovery, budgets and warnings

- Continue after a diagnostic only where remaining work has trustworthy prerequisites. Ordinary concrete function bodies fail after their first primary error. Later independent bodies may be checked through the established Stage 4 recovery boundary. Other stages retain their specified fail-fast rules.
- Avoid cascades that report secondary consequences of one malformed construct as independent causes. Preserve genuinely independent errors instead of broadly suppressing later work.
- Respect the strict per-module error allowance across its owned compilation work and the project-level soft scheduling cutoff. Count primary errors only. Keep related labels and diagnostic groups intact.
- Preserve stop metadata separately from diagnostics. A stopped request never reports unchecked work as successful or manufactures errors for unvisited modules.
- Keep warning identities and order stable. Only the command/report owner freezes warnings into a successful `CompilationOutcome`, including successful check-only work without an artefact. Diagnosed or infrastructure-failed outcomes discard unrelated staged warnings.
- Merge errors, labels, identity deltas and report groups in canonical order. Hash-map iteration or worker completion timing must not change their presentation.

### Commands, targets and presentation

- `check`, `build` and `dev` use the same semantic family and code for shared validation. Build-system context may add relevant project details but must not replace the compiler's diagnosis.
- Reject unsupported reachable target capabilities through frontend or link validation before lowering. A lowerer must not silently substitute behaviour or turn a known user limitation into an internal failure.
- Keep semantic content consistent across terminal, terse, dev-server and tooling renderers. Respect each format's source-location, escaping and machine-readable code contract.
- Escape user-controlled text in HTML output and sanitise delimiter or newline characters in machine-readable output without changing the underlying semantic facts.
- Do not report internal facts, speculative origins or unsupported remedies to make one presentation more descriptive than the underlying typed record allows.

## What bad diagnostics look like

Treat these as investigation triggers, not automatic findings without reproduction:

- **Vague:** "Invalid syntax" when the parser knows the required delimiter or token.
- **Incomplete:** a type error omits known expected/found types, relevant context or the declaration imposing the requirement.
- **Unhelpful:** a known, valid correction exists but the message gives no practical next step.
- **Misleading:** a suggestion uses unsupported syntax, assumes user intent or only works for some reason variants.
- **Overgeneralised:** one catch-all covers immutable bindings, missing mutable access and operations that never support mutation.
- **Poorly located:** the span points at an enclosing function instead of the erroneous operator, or a collision omits its original declaration.
- **Redundant:** message, guidance and labels repeat the same facts without explaining relationships.
- **Technically exposed:** printed `TypeId` values, IR node names, raw fact indexes or renderer fallback placeholders appear in user-facing output.
- **Wrong lane:** a failure is classified or presented as the wrong kind. If malformed source panics or an operational failure becomes a source rejection, log a Correctness lead rather than treating the changed outcome as a diagnostic-only finding.
- **Poorly constructed:** `format!` strings sit in semantic payloads, callers duplicate similar prose, paths are cloned into records or renderers reinterpret codes to determine the underlying reason.
- **Unstable:** report order changes across runs, identities are misremapped or one failed dependency generates repeated downstream errors.
- **Missing:** a known invalid case receives only a generic diagnostic, or a recovery boundary loses independent diagnostic context. If a validator silently accepts invalid source, log it under Correctness instead.

## Audit workflow

1. **Establish the contract.** Read the relevant language, stage, diagnostic-schema and build authorities. Identify which stage owns each rule and whether the issue affects legality, presentation or both.
2. **Inventory the family.** Find the schema entry, constructor, reason variants, render function and every producer. Include nearby catch-all cases and diagnostic-free rejection paths.
3. **Reproduce the output.** Use the smallest source or project fixture. Capture the exact code, message, guidance, primary span, relevant labels and command/target context. Record actual behaviour before proposing a replacement.
4. **Probe permutations.** Exercise distinct causes, context kinds, boundaries, related declarations, malformed near-misses and supported alternatives. Compare the result against the quality and specialisation rules.
5. **Locate the missing fact.** Decide whether the renderer can use existing data, the typed schema needs a reason/fact or an earlier semantic owner must capture information. Do not move semantic analysis into presentation code.
6. **Classify the finding.** Group related message, coverage and construction problems under one root cause. Record separate wrong-legality or test-coverage leads in `docs/bug-reports.md` without creating linked formal findings or expanding audit scope.
7. **Propose the smallest fix.** Prefer refining prose or existing facts before expanding a reason enum, and expand a reason enum before introducing a new family when the semantic identity remains unchanged. Remove obsolete branches and redundant helpers touched by the change.
8. **Specify verification.** Name fixtures and assertions that establish the corrected behaviour without relying only on "must fail" or a generic snapshot.

### Evidence required for each finding

- Minimal input and the command or API used to reproduce it.
- Actual code, message, guidance, primary location and important related labels, or an explicit statement of what is missing.
- Expected diagnostic behaviour with a concrete example where useful. Separate proven requirements from proposed wording.
- The semantic condition, its owner and the relevant constructor/schema/renderer sites.
- Why the current output violates a stated criterion, plus the smallest plausible fix.
- Missing permutations and focused regression tests. For new reason branches, list the distinct causes and expected guidance.

Prioritise incorrect lanes, provenance, false explanations, invalid suggestions and cascades before cosmetic wording. A concise useful report names the defect and its correction, not every occurrence of the same underlying pattern.

## Acceptance checklist

Every applicable item must pass. Mark a criterion not applicable only with a concrete reason.

**Message and guidance**

- [ ] Names the actual failure and violated rule in clear, concise user-facing language.
- [ ] Uses available semantic facts without exposing internal representation.
- [ ] Distinguishes expected and found where useful and explains the relevant constraint.
- [ ] Offers a supported, specific next step when the compiler can establish one.
- [ ] Makes no speculative claims or misleading suggestions.
- [ ] Keeps tone, terminology and code formatting consistent without redundant message, label or guidance text.

**Location and coverage**

- [ ] Uses the correct, narrow primary span whenever authored source owns the failure.
- [ ] Includes meaningful related locations and complete conflict context where the contract requires them.
- [ ] Preserves source identity across files, generated inputs and identity domains.
- [ ] Handles materially different causes with appropriately specialised explanations and corrections.
- [ ] Covers relevant invalid permutations without unnecessary new codes or generic fallback overuse.

**Compiler integration**

- [ ] Uses the correct diagnostic, infrastructure or compiler-bug lane.
- [ ] Preserves stable semantic identity and the schema-owned typed construction contract.
- [ ] Captures facts at the semantic owner and keeps rendering free of semantic reconstruction.
- [ ] Preserves deterministic order, multiplicity, context lifetime and remapping.
- [ ] Respects recovery, dependency blocking, budgets, stopping and successful-warning ownership.
- [ ] Keeps shared validation and user-visible meaning consistent across commands, targets and output formats.

**Verification**

- [ ] Includes focused reason and rendering tests for changed messages, labels and guidance.
- [ ] Includes end-to-end fixtures for affected source cases, near-misses and independent failures.
- [ ] Asserts stable code, exact relevant spans and related-site roles, not merely failure status.
- [ ] Checks suggestion validity, diagnostic multiplicity, cascade behaviour and deterministic presentation where affected.

## Tests and regression policy

- Use one focused case per distinct reason or correction, plus representative valid counterparts that must not trigger it.
- Test syntax and semantic boundaries through the owning compiler stage. Unit-test typed construction and rendering when changes concern only the schema or message.
- Assert the stable code, relevant facts, message or guidance, primary span, related-label roles and ordering where those details form the contract. Use byte-exact spans when source positions matter.
- Include cross-file ownership, source-less operational cases, generated inputs and alternate command/target surfaces when affected.
- Prove a suggested correction is accepted or supported by the relevant language contract. Do not lock tests to incorrect or speculative prose.
- Check one malformed input does not create artificial cascades and that independent failures remain visible within recovery and budget rules.
- When changing a shared family, cover every affected reason branch. Remove outdated expectations and redundant tests rather than leaving contradictory fixtures.

## Revisit this guide when

Diagnostic identity, failure lanes, schema ownership, source attribution, type rendering, warning finalisation, recovery budgets, output protocols or the accepted language correction policy changes materially.
