# Runtime anonymous records

## Status

- Status: queued.
- Current slice: not started.
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

Read `docs/src/developer-docs/language/overview.mtf` and its published MON syntax, const-record, struct, function, collection and Design Scope references. Read `docs/src/developer-docs/memory-management/overview.mtf` and the routed reference/access, copy, borrow, lifetime/escape and backend contracts.

The accepted runtime contract is published in a canonical unsuffixed structs reference before implementation. That reference owns semantics. MON's shared grammar belongs to its language reference rather than this plan. The progress matrices report actual support.

## Accepted runtime model

```moth
Size = |
    width Int,
    height Int,
|

show_window ||:
    window ~= (
        title = "Strategy",
        bounds = (
            size = Size(width = 1280, height = 720),
            visible = true,
        ),
    )

    window.bounds.size.width = 1440
    independent ~= copy window
    independent.title = "Overview"
;
```

### Construction and identity

A runtime receiving context selects a runtime record. Fields are ordinary expressions and need not fold. Every field has a unique name and an initializer. At least one field is required. Parentheses around a single unnamed expression remain grouping and empty `()` is not a runtime record. Compile-time contexts retain their separate const-record behaviour.

Each static literal site owns one hidden nominal identity within its owning compiled body. Repeated loop executions create values of that same site type, not new types. Distinct literal sites remain distinct even when their field names and types match. Copies and aliases preserve type identity.

Phase 0 establishes a deterministic source-site key qualified by its owning module/body and any concrete generic materialisation needed to avoid collisions. Runtime executions, rendered names and structural shape are not identity keys. These identities remain module-local and require no persistent public identity scheme.

Fields resolve in source order under normal expression and access rules. Field labels introduce no local bindings. Initializers run once using ordinary evaluation and error order. Mutable roots permit field mutation through the ordinary place model. A mutable record binding does not copy or grant new authority over an existing retained child.

The hidden type has no source-visible type name, named constructor, receiver methods or conformance. Compatible reassignment uses the same resolved identity, not a new literal of matching shape. Nominal construction remains explicit: `(width = 1, height = 2)` is not an implicit `Size(...)`.

### Recursive local composition

An anonymous record may contain another anonymous record directly or through an already bound local. It may also contain ordinary supported values, including explicitly constructed named structs and choices. Nesting is part of the initial runtime implementation, not a follow-up restriction to remove later.

```moth
show_state ||:
    child = (count = 3)
    state = (
        saved = child,
        fresh = (enabled = true),
    )

    selected = state.fresh
    count = state.saved.count
;
```

The parent hidden struct's field stores the child's resolved hidden type identity. Recursive registration follows resolved child types without shape interning. An anonymous child's field may itself contain another anonymous record.

The compiler-generated fields of hidden anonymous parents are the permitted composition boundary. This does not permit hidden types in authored nominal struct/choice declarations or allow anonymous values to satisfy named fields by shape. A named struct may be a child of an anonymous parent, but an unnamed child cannot bypass the named struct's declared field types.

Extraction into another local preserves the child's type and ordinary alias relationship. Reading an ordinary named or scalar leaf is an ordinary value use. Reject prohibited hidden identities, not every value that once passed through an anonymous record.

### Local-only boundary

| Use | Initial runtime support |
|---|---|
| Local binding, compatible assignment, projection and explicit copy | Supported under ordinary struct rules |
| Anonymous child inside an anonymous parent | Supported, recursively |
| Explicit named struct or choice inside an anonymous record | Supported when ordinary field/value rules permit it |
| Ordinary leaf extracted from a record | Ordinary rules for the leaf's type |
| Function argument or return carrying a hidden identity | Rejected, including private functions |
| Authored signature, alias, nominal struct/choice field, receiver method or trait evidence exposing a hidden identity | Rejected |
| Exported value/interface exposing a hidden identity | Rejected |
| Collection/map storage or a generic argument carrying a hidden identity | Rejected in this implementation |

Apply these restrictions transitively. A wrapper, nested parent, optional or captured value cannot conceal a hidden identity at a prohibited boundary. Reuse ordinary recursive semantic type inspection rather than checking only the outer type. This is a bounded local-value feature, not shape polymorphism or type extraction.

A generic body may construct a local record from its already resolved concrete values if ordinary compilation supports that body. It may not pass a hidden identity as a generic argument. Keep these cases distinct.

Wiring capabilities retain their own storage restrictions. A shared argument parser does not make a wire or route storable in an anonymous field. MON syntax likewise grants no serialisation support to runtime records.

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

### Phase 0: Refresh ownership and publish the contract

1. Verify all prerequisites in the activation tree and record the baseline locally.
2. Read the shared MON owner and existing const-record deferral boundary, ordinary nominal registration, HIR struct paths and recursive escape validators.
3. Publish the runtime contract in the canonical structs reference and reconcile related architecture and Design Scope text.
4. Decide the source-site identity key, including nested sites and concrete body materialisations. Inventory every prohibited boundary and its existing validation owner.
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
4. Update the structs reference, teaching pages, cheatsheet, compiler/build authorities where affected, Design Scope, progress matrices, comments and queued consumers. Replace blanket nesting/aggregate bans with the precise local-composition boundary.
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
