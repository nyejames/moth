# Runtime anonymous records

## Status

- Status: queued. The canonical and Basic runtime references are published.
- Current slice: implementation not started.
- Blockers: shared MON syntax with nested const records, accepted numeric semantics and the required ordinary-struct validation paths must be delivered.
- Next action: establish the activation tree and complete Phase 0.

## Purpose and prerequisites

Enable named-only parenthesised MON construction in runtime receiving contexts. Each literal site creates a local hidden nominal struct. Support inline nested anonymous records and explicitly constructed nominal children through ordinary struct semantics.

Reuse the shared argument parser, type environment, struct construction, field access, copy, borrow and lifetime owners. This work introduces no structural typing, anonymous-specific runtime IR or MON serialisation implementation.

Run after shared MON syntax and number/numeric semantics, before the HTML mixed JavaScript/Wasm backend work that consumes this capability. The main roadmap owns ordering. Establish the revision, worktree status and baseline at activation in local working notes, not in this queued plan.

Required capabilities:

- one shared argument-list owner with named-only and compile-time policies, receiving-context diagnostics and recursive value parsing
- parenthesised anonymous const records with folded field projection and public-value handling
- ordinary nominal identity, resolved field types and field lookup in `TypeEnvironment`
- ordinary struct HIR construction, projections, copy, borrow validation and lifetime/escape validation
- public-surface rejection of hidden runtime identities
- accepted numeric type identity, including the delivered `Number` family

Use the activation tree's current APIs. Reuse delivered MON parsing rather than reconstructing a predecessor record parser.

## Required authorities

Read `AGENTS.md`, the full style guide, the language cheatsheet, both compiler/build architecture references and the complete testing guide. Follow the validation guide for each final gate.

Read `docs/src/developer-docs/language/overview.mtf`, `docs/src/docs/language-overview/mon-syntax.mtf`, `docs/src/docs/structs/anonymous-records.mtf` and the routed const-record, struct, function, collection and Design Scope references. Read `docs/src/developer-docs/memory-management/overview.mtf` and the routed reference/access, copy, borrow, lifetime/escape and backend contracts.

The accepted runtime contract is already published in `docs/src/docs/structs/anonymous-records.mtf`, with a Basic companion and a section on the Structs page. Those references own semantics and explicitly label runtime support as deferred. Contract publication is complete, not a task to repeat. The progress matrices continue to report actual support.

## Published contract and implementation decisions

Use the canonical runtime reference for construction, hidden identity, recursive local composition, prohibited boundaries and capability restrictions. Shared grammar is owned by the MON reference. Keep these implementation consequences explicit:

- Each static literal site owns one hidden nominal type per owning compiled body. Repeated execution reuses that type, while different sites remain distinct. Aliases and copies preserve type identity.
- Qualify the source-site key by its module/body and concrete materialisation identity where required. Reuse current retained source-site and instantiated-body identities. Runtime execution counters, field shape and rendered type names are not keys. The exact local Rust representation is an implementation choice, not a new public identity contract.
- Resolve children before registering ordered parent fields. An anonymous parent may store a hidden child directly or by an existing local reference. Explicit nominal children remain ordinary typed values.
- The composition exception applies to compiler-generated anonymous parent fields. It does not permit hidden types in authored nominal declarations, function arguments/returns, exported surfaces, collections/maps or generic arguments.
- Apply those restrictions recursively, including through a containing parent, optional or captured value. An ordinary leaf extracted from a record keeps its ordinary permitted uses.
- Constructing a local record inside an already concrete generic body differs from passing a hidden type as a generic argument. Only the former is permitted.
- Empty runtime records remain invalid. Ordinary constant-looking fields do not select the const-record path. Wire and Route storage restrictions remain independent of shared syntax.

The initial implementation includes nesting. It must not accept only a flat subset and leave parent/child support or recursive escape checks for later.

## Semantic and representation ownership

Keep runtime structs and compile-time records separate after shared syntax parsing:

- A const record uses the existing compile-time marker and folded field store. Its complete value is not an ordinary runtime object.
- A runtime record uses an ordinary hidden `Struct` type with ordered fields in `TypeEnvironment` and ordinary struct expression construction.
- HIR receives the resolved nominal type and fields. No anonymous-specific HIR or backend node is added.
- Public projection rejects hidden runtime identity rather than canonicalising it through the const-record marker path.

Shared parsing retains the authored names, values and locations once. Runtime semantic construction registers field types using those results. No second named-field parser, synthetic callee signature or repeated source scan is needed.

Use ordinary field lookup without repeated shape scans. Diagnostics identify the anonymous record's source site and the first prohibited use. Nested type checks retain useful field/source context without exposing internal numeric IDs as user-facing identity.

Borrow validation and lifetime topology apply to the actual retained graph. Inline nesting creates no escape exemption, copy-on-construction rule, new region or backend-specific legality. Explicit deep copy preserves internal alias topology under the existing copy contract. Backend lowering consumes validated ordinary struct facts.

## Implementation phases

Each code-bearing phase includes its focused tests and `AGENTS.md` Slice review. Commit accepted phases separately. Keep diagnostics and status truthful while backend support is incomplete.

### Phase 0: Refresh ownership against the published contract

1. Verify all prerequisites in the activation tree and record the baseline locally.
2. Read the shared MON owner and existing const-record deferral boundary, ordinary nominal registration, HIR struct paths and recursive escape validators.
3. Read the published runtime reference and confirm the current owner map. Identify remaining architecture, cheatsheet and status edits without rewriting the source contract or claiming runtime support.
4. Select the existing source-site/body identities that implement the key above, including nested sites and concrete materialisations. Record the mapping locally. Inventory every prohibited boundary and its current recursive validation owner.
5. Confirm ordinary struct representation can express nested children and all required access/copy/lifetime relationships without new runtime IR.

Exit: one identity rule, one shared parse path and explicit recursive boundary coverage.

### Phase 1: Construct typed nested local records

1. Enable the runtime context in the shared MON argument path with named-only entries and ordinary runtime values.
2. Register hidden nominal fields after child values resolve. Produce ordinary struct expressions for both outer and inner records.
3. Support local binding, projection, compatible assignment and explicit copy, including mixed nominal children and references to existing anonymous children.
4. Replace runtime deferral coverage with acceptance cases while retaining the complete const-record boundary and empty-runtime rejection.
5. Enforce prohibited uses before a completed semantic artefact can be published. Implement construction and necessary rejection checks together rather than exposing unchecked intermediate support.

Exit: nested local runtime values type-check correctly and unsupported escapes receive source diagnostics.

### Phase 2: Complete recursive boundaries and ordinary lowering

1. Check hidden identity transitively at calls, returns, exports, aliases, authored fields, collections/maps, generic requests and capability-storage boundaries.
2. Preserve legal extraction and use of ordinary leaves. Permit hidden child fields specifically in hidden anonymous parents.
3. Lower through ordinary struct HIR and validate field/type relationships. Reuse existing source identity and remap machinery.
4. Exercise nested mutable access, aliases, final use, deep copy and lifetime containment through the existing analyses.

Exit: anonymous syntax produces no anonymous-specific HIR/backend representation and has no path around ordinary access or escape rules.

### Phase 3: Backend coverage, documentation and pruning

1. Validate supported targets through ordinary struct lowering. Assert an explicit target/deferred rejection where a required ordinary capability is unavailable, rather than claiming executable parity from frontend-only tests.
2. Add or consolidate end-to-end cases for nested output, mutation/copy, existing-child aliasing, duplicate inner labels, distinct-site mismatch and each materially different prohibited boundary.
3. Keep grammar/separator tests with the shared MON owner. Runtime tests cover runtime policy, identity, value behaviour and escapes instead of duplicating that grammar suite.
4. Update support notices in the published structs references and teaching page, then reconcile the cheatsheet, compiler/build authorities, Design Scope, progress matrices, comments and queued consumers. Preserve the precise local-composition boundary. Shared grammar remains with the delivered MON owner.
5. Remove obsolete deferral paths, fixtures and temporary construction helpers. Rebuild generated docs.

### Phase 4: Final validation and Slice review

Run the final gates and a focused read-only review of identity, nested fields, recursive escape, copy/borrow/lifetime facts and backend handoff. Review all touched and adjacent owners for duplicated grammar, stale comments and obsolete tests.

Update source locators and stale audit-log entries under repository policy. Delete this completed plan and its roadmap entry in the completion commit. Keep genuine remaining format, collection or public-type proposals in their durable owners rather than leaving a completed plan as documentation.

## Required test distinctions

Tests should prove:

- nested literal construction and an existing child reference both work
- identical labels in different records are legal, while a duplicate within one record is diagnosed at the inner site
- distinct sites remain different types and repeated execution does not create new semantic identities
- copies preserve types and required internal alias topology without sharing mutable storage with the source
- retained aliases constrain nested mutation under ordinary borrow rules
- prohibited wrappers do not hide an anonymous identity from escape checks
- ordinary extracted leaves retain their normal permitted uses
- compile-time records remain compile-time-only and named nominal construction stays explicit
- supported backend execution agrees with ordinary struct semantics, while unsupported targets reject explicitly

Use one primary owner per contract, structured diagnostic reasons/spans and observable results. Unit coverage is reserved for hidden identity, type-graph and handoff invariants that output cannot expose. Keep tests out of production implementation files.

## Stop conditions and validation

Revisit ownership before continuing if the implementation needs shape unification, public hidden identities, the const-record marker for runtime values, anonymous-specific HIR/backend nodes, repeated field-shape scans or weaker borrow/lifetime rules. Resolve the cause instead of adding a compatibility layer or silently widening the feature.

Run `cargo fmt` and `just validate` for every accepted code-bearing phase. Run `moth build docs --release`, or the equivalent Cargo command, for documentation changes. Follow the current validation guide and report checks that failed or could not run.

Completion requires recursive local records, explicit nominal construction, transitive rejection at every prohibited boundary and reuse of ordinary struct semantics from AST through supported backends.
