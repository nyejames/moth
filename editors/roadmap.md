# Editor support roadmap

Status: preparation. The syntax migration is the activation prerequisite. This programme is not yet in the main project roadmap. The sequence below is milestone-based rather than a calendar estimate because the migration and multi-editor prototypes determine the critical path.

Read [design.md](design.md) first. Its accepted goals and ownership constrain every slice. Each milestone ends in a coherent commit and a review of correctness, redundancy, test ownership and scope. Record the actual commands, editor/runtime versions and results. Do not claim unexecuted checks or mark a package complete from a generated grammar alone.

## M0 - Record the design and prepare the branch

- [x] Define the shared syntax and editor integration direction.
- [x] Establish the editor directory boundaries and document intended ownership.
- [x] Review this design with the project lead.
- [x] Inventory the existing VS Code extension: `nyejames/moth-vscode-highlighting`, publisher `Nye`, extension name `moth-highligher` (retain the existing spelling for upgrade continuity).
- [x] Record the required highlighting vocabulary, ObVer policy and approved multi-root grammar organisation.
- [x] Add the initial settled grammar subset, corpus and injection experiments in `tree-sitter-moth/`. This is candidate source until the generator and corpus pass.
- [ ] Run parser generation and tests on the pinned toolchain. Commit reproducibly generated files only after verification.
- [x] Import and update the existing TextMate fallback candidate and add focused tokenizer tests.
- [ ] Validate the ported TextMate candidates with the real tokenizer, then decide their long-term role at M1.
- [ ] Expand the settled subset only when its tests run. Preserve the remaining syntax and host-specific work for M1/M2.
- [ ] Verify that every preparation commit changes only `editors/`. Keep root manifests, docs, roadmap, CI and compiler code unchanged.

Exit: approved design, ready-to-use local foundation and a narrowly scoped PR. No claim of highlighting support.

## M1 - Activate and de-risk the architecture

Prerequisite: completed syntax migration with settled canonical language documentation and executable source/codec fixtures. Rebase onto the resulting main, check current branch scope and record the actual baseline. Add root roadmap/index integration only when explicitly authorised at activation. Decide how to track subsequently delivered closure/channel grammar without delaying syntax support for implemented features.

Read the final source authorities, progress matrix, lexical inventory and relevant compiler ownership. Use the repository style, testing and validation routes. Reconcile draft decisions with the delivered grammar before freezing node names or fixtures.

Verify and extend the initial M0 multi-root Tree-sitter prototype. Include nested Moth brackets, an implicit template body, a MON root record, code versus body comments and a malformed source witness. Try native and Wasm parser generation using pinned compatible tools. Establish stable node shapes only after tests prove they express the required boundaries.

Use [shared integration fixtures](tests/README.md) to prototype embedded HTML, CSS including inline mode, Moth Markdown and nested Moth code listings. Exercise all four editors at this checkpoint, especially VS Code with semantic highlighting on and off. Verify how each host loads guest parsers and how ranges, query predicates and nested captures behave. Establish supported versions and installation paths. Resolve the release packaging question for multi-root grammars and guest dependencies.

Exit: demonstrated architecture in every required editor, with a documented choice for external scanner, Moth Markdown, injection grouping, CSS declaration parsing and VS Code fallback. Any unsatisfied required feature blocks the design choice. Do not expand production grammar until the bottleneck is understood.

## M2 - Complete the shared grammar and syntax queries

Implement only the delivered canonical syntax. Use test-first vertical slices and keep the cheapest correct rule.

- Establish common lexical rules, exact words and identifier/number conformance, strings and mode-specific trivia.
- Add Moth type syntax, declarations, expressions, blocks, arms, dependency clauses, annotations, template head/body forms, slots and control flow. Include the post-migration closure and channel forms according to the activation inventory.
- Complete Moth template file root mode, Moth Markdown, formatter selection and nested boundaries.
- Complete MON's explicit/implicit record roots, literal values, record/map entry spacing, collections, nominal/choice forms and data-only rejection boundaries.
- Add highlighting, injection, folding and structural navigation queries. Make editor-specific variants only where the host's API or capture vocabulary requires it.
- Exercise all compiler-supported code-listing languages and aliases, generic/plain cases, `$note`, `$raw`, `$escape_html`, HTML, CSS and Moth Markdown.

Reuse selected positive compiler and codec fixtures and authoritative examples rather than copying the language suite. Add editor-specific incomplete and incremental edit witnesses. Keep separate syntax acceptance, highlighting and compiler validation expectations.

Exit: generated native and Wasm parser artifacts from the same grammar sources, exact corpus trees and capture tests, grammar drift checks, malformed input recovery and incremental equivalence against fresh parsing. No compiler dependency for highlighting.

## M3 - Deliver native editor integrations

Use Zed as the first complete reference integration. Neovim and Emacs can proceed in parallel after the shared parser and query contract is established.

- Zed: manifest, language configs, native queries, parser packaging and sample workspace.
- Neovim: file detection, native parser installation/registration, queries and buffer-local setup.
- Emacs: Tree-sitter major modes, font-lock, guest parser ranges, indentation and Imenu/navigation.

Each integration must cover all three file formats, the same formatter inventory and editor-native editing features. Test clean installation, missing parser behaviour, themes, file association, opening incomplete files, nesting and commands. Keep adapters small and avoid global configuration side effects.

Exit: all three native integrations pass the common release fixtures and their own host-specific tests on their declared supported versions.

## M4 - Replace VS Code integration

Build from the successful prototype. Ship parser assets, document lifecycle, highlight and guest capture mapping, folding, symbol and selection providers, language configuration and snippets. Establish bounded work and cancellation/staleness at the editor boundary. Maintain a small fallback TextMate grammar only for its demonstrated role and prove it does not drift into a second feature-complete grammar.

Test native extension host and remote-workspace cases that the declared installation supports. Verify disposal and re-opening, unsaved edits, Unicode token positions, missing parser assets and theme behaviour. Verify how semantic-token preferences affect the promised highlighting experience. Do not claim browser-host support unless it is tested and packaged.

Exit: VS Code satisfies the same three-format and embedded-language contract as the native editors. Retire the old rough extension only when the replacement is verified and its publication identity is agreed.

## M5 - Lightweight compiler tooling

This milestone may proceed in parallel with M3/M4 after M1 and approval to change compiler areas. Keep the preparatory branch confined until that approval. Compiler work follows the normal routed implementation plan and validation gates.

- Specify and implement `moth check . --mon` through the existing structured check report and `moth-mon` codec. Include valid outcome envelopes, diagnostic coverage, exact source ranges, related labels, deterministic serialization, exit semantics and malformed/operational failures. Test stdout purity and special characters, Unicode and paths.
- Decide between a minimal diagnostics-only LSP and a native subprocess integration using measured complexity and real supported editor APIs. Reuse one decoder/producer, not editor-specific parsers or duplicated diagnostics semantics.
- Implement saved-source diagnostics with explicit project, builder and input selection, workspace trust, stale-result rejection and single in-flight work. Keep compiler absence non-fatal.
- Provide explicit check/build/dev command integration via native tasks or equivalent. Verify argument quoting, working directory, custom binary and nonzero process exit. No implicit build/dev startup.

Exit: all supported editors can surface compiler diagnostics from saved source and run explicit commands. Full LSP semantics remain deferred. Syntax functionality still works with no installed compiler.

## M6 - Qualify and publish V1

Validate all required editors and file types, nested formatter coverage, every literal supported code-listing language, structural features and saved-source diagnostics. Check clean installation, updates and offline use after parser assets are installed, supported platform packaging, attribution and version compatibility. Include at least macOS and Linux for the shared parser and declared supported host packages, and Windows wherever support is advertised.

Run the shared conformance corpus, capture and edit/recovery tests, native and Wasm parity, adapter tests and real-host integration checks. Use a representative large Moth document and MTF page for cold/warm parsing and incremental responsiveness evidence. Record baseline, fixture size and machine/runtime configuration instead of inventing time targets. Confirm that syntax updates do not launch a compiler or reparse unrelated buffers.

Review the complete branch for duplication, unused scaffolding, stale source examples, unnecessary abstraction and test gaps. Check the normal repository gates once the branch integrates compiler changes. Establish actual package publishing rights, artifact provenance and installation instructions. Update canonical project navigation and status only as part of the authorised integrated work. Do not claim release readiness until the applicable gates pass.

Exit: publish the supported packages and make the shared grammar accessible to downstream consumers without requiring duplicate source repositories.

## Later - Semantic language server

Create a separate design before expanding LSP. Reuse the source snapshot/position and structured report contracts. Design unsaved-document overlays, project graph invalidation, cancellation, request isolation, diagnostic publication, definition/reference identities and a semantic occurrence index before introducing compiler service APIs. Do not retain the entire AST or invent a second type system just to implement an editor feature.

Start with diagnostics if not already delivered through a small LSP. Grow into hover, completion, definition, references, rename, signature help, semantic highlighting and inlay hints from actual resolved facts. Formatting has its own design and tests. Keep local Tree-sitter tooling as the responsive syntax layer even with an LSP installed.
