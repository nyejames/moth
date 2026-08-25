# Build configuration values and project globals implementation plan

The former `#Import` spelling is superseded and must not be implemented or retained through
compatibility syntax.

## Purpose

Implement typed project and source build-configuration contracts, immediate primitive CLI and
programmatic input typing, deterministic configuration resolution and the immutable `@project`
interface after grouped project config is complete.

A resolved `#Config` declaration becomes an ordinary folded constant before module AST execution
semantics are finalised. It creates no runtime wrapper, no special HIR category and no second
conditional-compilation system.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/build-configuration-values-and-project-globals-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh CLI, config-field, header-contract and synthetic-interface owners
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: CLI, config folding, header syntax, Stage 0 contract barrier, synthetic project interface
```

Keep this block concise. Git history is the implementation record.

## Hard prerequisites

- accepted canonical module Phase 5 closeout
- anonymous const records, folded and projected through public interfaces
- grouped project config and recursive builder schemas
- Stage 4 static Bool `if` specialisation, selecting one branch before HIR
- stable public-interface provenance and package-boundary ownership

This plan must complete before entry-local config blocks.

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/design-scope/design-principles.mtf`
- `docs/src/developer-docs/language/overview.mtf` and its relevant canonical references
- delivered Stage 4 static Bool `if` specialisation and folded-constant ownership
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`

## Vocabulary and ownership

Keep these concepts distinct:

- **Project config**: the self-contained build-system-owned `config.moth` program.
- **Entry config**: a later root-local `config:` block containing builder and tooling metadata.
- **Build-config contract**: a `#Config of T` declaration resolved from fixed project values,
  explicit command/programmatic inputs, builder globals or defaults.
- **Build input**: the internal transport and resolution vocabulary. Internal types may retain names
  such as `BuildInputType`, `BuildInputName` and `PrimitiveBuildValue`.
- **Package dependency declaration**: future project-level package syntax owned by the package
  dependency plan. This plan does not prescribe its spelling.
- **Source dependency**: an ordinary top-level `.moth` dependency clause.

`#Config` is compiler-owned constant-source syntax. It is not a semantic type constructor. In:

```moth
analytics #Config of Bool = false
```

`analytics` has semantic type `Bool`, not `Config of Bool`.

The word `Import` is freed for ordinary user identifiers. A user type such as this is valid:

```moth
Import = |
    source String,
|
```

The `config` identifier family is reserved through the canonical keyword-shadow policy. This
reserves case and leading-underscore shadows such as `config`, `Config`, `CONFIG` and `_Config` so
the project config file, entry `config:` block and `#Config` source form share one compiler-owned
vocabulary.

## Accepted source surface

### Direct project field

```moth
project #= |
    name = "my_app",
    version #Config of String = "0.1.0",
    entry_root = "src",
|
```

### Source contract

```moth
api_url #Config of String = "http://localhost:8080"
analytics #Config of Bool = false
optional_label #Config of String? = none
```

Accepted contract types are:

- `String`
- `Int`
- `Float`
- `Bool`
- `Char`
- optional forms of those types

No other scalar, aggregate, nominal, generic or path type may be a `#Config` contract.

A direct project contract may appear only on a direct field of `project`. A source contract is a
module-wide top-level compile-time declaration. `#Config` is invalid:

- on nested project fields
- in builder or tooling sections
- inside an entry-local `config:` block
- inside executable bodies
- on runtime or mutable bindings
- on function parameters, returns, fields or type declarations

There is no runtime `Config` wrapper, `DataType::Config`, config-specific AST value or HIR node.

## Ordinary `if` and build specialisation

There is no `#Config if` form.

A `#Config of Bool` value is an ordinary folded Bool constant by the time Stage 4 finalises executable
control flow:

```moth
analytics #Config of Bool = false

if analytics:
    send_analytics()
;
```

The constant-evaluation and static-control-flow plan owns the general rule:

- both authored branches complete syntax, name, visibility, type and generic-evidence validation
- a known Bool selects one branch before HIR
- the inactive branch contributes no generated sidecar work, borrow/lifetime facts, link facts,
  target requirements or backend code
- a runtime Bool remains an ordinary runtime `if`

This plan must integrate through that existing ordinary constant and `if` path. It must not add a
second evaluator, branch pass, config-specific reachability rule or backend optimisation.

Build configuration specialises executable behaviour only. It cannot change:

- dependency clauses or provider graph edges
- package resolution
- source discovery or semantic source sets
- declaration existence
- exported declaration existence
- receiver-method existence
- trait or conformance existence
- module or package facade topology

Stage 0 graph construction and Stage 3 declaration order are therefore independent of `#Config`
values.

## CLI primitive inference

CLI input syntax remains repeated `--input name=value`:

```bash
moth build . --input analytics=true
moth check . --input retries=4
moth dev . --input ratio=0.75
moth build . --input api_url=https://example.com
moth build . --input "separator=':'"
moth build . --input 'label="true"'
```

The shell quotes in the last two examples preserve Moth literal delimiters inside one command
argument. They are not part of the `--input` grammar.

The command parser splits at the first `=`. The complete remaining text is the value, including later
`=` characters. It infers one concrete primitive value immediately without waiting for a project or
source contract.

Inference order and rules:

1. exact lowercase `true` or `false` -> `Bool`
2. a complete valid signed Moth whole-number literal -> `Int`
3. a complete valid Moth decimal-point or exponent literal -> `Float`
4. a complete valid single-quoted Moth character literal -> `Char`
5. a complete valid double-quoted Moth string literal -> `String`
6. every other value, including the empty value after `name=`, -> `String`

A value starting with an explicit quote must parse as the corresponding complete Moth String or Char
literal. An unterminated or malformed quoted literal is a command-input diagnostic rather than a
String fallback.

Numeric inference uses the compiler's one `numeric_text` grammar and materialisation helpers.
Whole-number overflow, invalid finite-Float materialisation and non-finite results are diagnostics.
Only a complete valid Moth numeric literal is numeric. Text such as `1.2.3`, `+1`, `NaN` or `Infinity`
is otherwise an ordinary String fallback.

The fallback keeps common String inputs concise:

```bash
--input channel=alpha
--input api_url=https://example.com
--input version=1.2.3
```

A String that would otherwise infer as another primitive uses explicit Moth String literal syntax:

```bash
--input 'label="true"'
--input 'build_number="42"'
```

Char never infers from one unquoted character. It requires a valid Moth Char literal:

```bash
--input "separator=':'"
```

Bare `none` has no special CLI meaning and is inferred as String text. Optional absence is represented
by omitting the input and allowing project/default resolution to produce `none`. The CLI does not
construct an untyped or polymorphic `none` value.

Programmatic command APIs construct the same typed primitive carrier directly. They do not define a
second conversion or defaulting policy.

## Contract type checking

The explicit command value is typed before contract discovery. Later config or header processing
checks that typed value against the contract.

Rules:

- exact primitive types match
- a concrete `T` input may satisfy a `T?` contract as a present value
- no other coercion occurs at the build-input boundary
- `Int` does not satisfy `Float`; write a decimal or exponent literal when `Float` is intended
- `Bool`, numeric and Char values do not satisfy String contracts unless explicitly quoted or written
  as String fallback text
- typed builder globals and programmatic inputs follow the same compatibility rule

A mismatch diagnostic reports the inferred/provided primitive type, required contract type and the
source or command location. When useful, it explains how to force String interpretation with a
quoted Moth String literal.

## Resolution rules

### Direct project `#Config` fields

Direct project contracts resolve while config folds:

1. explicit CLI or programmatic input
2. compatible builder-provided primitive global
3. folded declaration default
4. missing-input diagnostic

Resolution completes before Stage 0 applies fields such as `entry_root`.

### Project-wide source contracts

Project-wide source contracts resolve after Stage 0 has collected the selected semantic graph:

1. compatible fixed direct project field, which is authoritative and cannot be overridden
2. resolved direct project `#Config` field
3. explicit CLI or programmatic input for a source-only contract
4. compatible builder-provided primitive global
5. shared source default
6. missing-input diagnostic

Same-name source contracts must agree on:

- primitive type
- optionality
- required or default state
- normalised default value

A direct project `#Config` contract and every same-name source contract must also agree before one
resolved value is supplied to modules.

A fixed direct project field is not a `#Config` contract. When it has the same name, primitive type
and optionality as a source contract, it is authoritative and blocks CLI override. Same-name source
contracts must still agree with each other on required/default state and normalised default value.

Unknown explicit inputs are diagnosed only after every selected project and source contract is known.
Duplicate explicit names are rejected earlier by command parsing.

## Builder-provided values and platform boundaries

A builder may provide typed primitive globals only when they express stable semantic build
configuration understood independently of a physical backend representation.

Builders and compiler bootstrap surfaces must not expose target or platform identity through
`#Config`, including equivalents of:

- `target_os`
- `target_arch`
- `backend`
- `is_wasm`
- `is_javascript`
- `is_browser`

Moth source is deliberately platform-agnostic. Project builders, builder packages, external packages
and backend capability metadata resolve platform-agnostic source semantics into platform-specific
artefacts. `#Config` must not recreate conditional target source through a different spelling.

Build profile or project-domain configuration may still be exposed when its meaning is stable source
semantics rather than a description of the selected implementation platform.

## Source-default restriction

A source default must be one self-contained primitive literal or `none`:

- no name or constant reference
- no template
- no operator expression
- no call or cast
- no field projection
- no collection or record
- no second Stage 0 constant evaluator

The restriction lets header syntax normalise each source contract before AST through a small retained
shape equivalent to:

```rust
pub struct SourceBuildConfigContract {
    pub name: BuildInputName,
    pub value_type: BuildInputType,
    pub required: bool,
    pub default: Option<PrimitiveBuildValue>,
    pub location: SourceLocation,
}
```

Exact names may change. Internal `BuildInput*` vocabulary may remain where it accurately describes
transport and resolution. The source and user-facing spelling is `#Config`.

The resolved value enters module AST as an ordinary folded constant. It creates no runtime wrapper,
HIR category, dependency symbol class or new visibility rule.

## `ProjectGlobalsInterface`

The folded project record produces an immutable synthetic interface at `@project` containing:

- stable field identities
- folded backend-neutral values
- source locations
- field-level fingerprints
- member-level project-context provenance
- no AST, HIR or runtime body

Rules:

- `@project` is permanently reserved.
- Normal project modules and project-owned support packages may bind it explicitly.
- It is never implicitly injected.
- It cannot be directly re-exported.
- Child modules, dependency aliases, Core, Builder and binding-backed packages cannot claim the root.
- The external project package facade rejects public or reachable executable dependence on private
  project configuration.

Project field dependencies are tracked at field granularity. Source `#Config` dependencies are
tracked at config-name granularity. A value change invalidates only public, implementation, root,
generated or link facts that actually depend on that value.

Static branch selection uses existing implementation, effect, root and link fingerprint owners. This
plan must not add a parallel `#Config` branch fingerprint.

## Configuration provenance and package boundaries

Every project or package compilation boundary owns its own configuration namespace.

- Root-project inputs do not implicitly satisfy dependency contracts.
- A dependency resolves only from its own config, defaults and compatible builder globals.
- Qualified dependency overrides and input forwarding remain package-manager design work.
- Input values persist through dev rebuilds inside their owning boundary.
- Resolved direct project and source `#Config` values retain boundary-local configuration provenance.
- Public or reachable executable package-facade facts may not depend on private boundary
  configuration, using the same package-context rule as `@project`.

This prevents a consuming project's unqualified command inputs from silently specialising dependency
source or changing a dependency's public package semantics.

## Non-goals

- no package dependency declaration syntax
- no package aliases, versions, paths, registry or lockfile
- no environment-variable syntax or env file
- no `-D`, `--define`, JSON or aggregate command input
- no build-input aliasing
- no runtime `Config` wrapper or source-visible `Config` type
- no nested project `#Config` fields
- no `#Config` in builder/tooling sections or entry-local `config:` blocks
- no `#Config if` syntax
- no conditional dependencies, declarations, exports, methods, traits or conformances
- no skipped frontend validation for inactive static-`if` branches
- no target, operating-system, architecture or backend introspection in source
- no CLI `none` value; optional absence comes from omission/defaulting
- no contract-directed reparsing of CLI values
- no user-defined input literal, cast or coercion system
- no second constant evaluator before AST

## Implementation phases

### Phase 0: Refresh and baseline

- Record current revision, branch and worktree state.
- Inventory CLI parsing, grouped config fields, canonical reserved-name policy, header syntax, Stage 0
  source graphs, static Bool specialisation, dev rebuild state and synthetic-interface provenance.
- Confirm project config, recursive schemas and Stage 4 static Bool `if` specialisation are accepted.
- Inventory every remaining former build-configuration source spelling and separate source-authority migration from
  executable implementation work.
- Run baseline validation.

### Phase 1: Reserve source vocabulary and add typed input carriers

- Reserve the `config` identifier family through the canonical keyword-shadow owner.
- Confirm `Import` and `import` remain ordinary identifiers outside the targeted removed-import
  migration diagnostic shape.
- Define one primitive/optional `BuildInputType` and one normalised `PrimitiveBuildValue`.
- Define `BuildInputName`, source/command locations, value origin and deterministic input maps.
- Keep typed value, resolution origin and fingerprint inputs together without string-backed fallback
  storage.
- Use existing numeric text materialisation for `Int` and `Float`.
- Reject out-of-range Int and non-finite Float values.
- Preserve authored String and Char content without backend conversion.

Review gate: one reserved-name owner and one typed primitive vocabulary exist.

### Phase 2: Parse command inputs with immediate primitive inference

- Add repeated `--input name=value` to build, check and dev.
- Split each argument at the first `=` and preserve the complete value remainder.
- Validate lower_snake_case input names.
- Implement the accepted Bool, numeric, Char, quoted String and String-fallback inference order.
- Parse explicit String and Char literals through the ordinary Moth literal grammar.
- Treat malformed explicit quotes as diagnostics.
- Treat bare `none` as String and omission as optional absence.
- Preserve explicit typed inputs in programmatic command options rather than global state.
- Reject duplicate explicit names deterministically.
- Delay unknown-input diagnostics until selected source contracts are known.
- Do not wait for or use a source contract to decide the CLI value's primitive type.

Review gate: build, check and dev share one parser and one typed carrier. No command-specific
conversion or defaulting path exists.

### Phase 3: Implement direct project `#Config`

- Allow `#Config of T` only on direct `project` fields.
- Reject nested project, builder and tooling occurrences.
- Treat `#Config` as a declaration qualifier whose semantic type is `T`, not as a generic type.
- Resolve explicit input, builder global, default or missing diagnostics while config folds.
- Apply exact primitive compatibility plus concrete-to-matching-optional promotion only.
- Keep fixed direct fields separate from configured fields so fixed values block later overrides.
- Preserve field locations, resolution origin, fingerprints and configuration provenance.

### Phase 4: Emit source contract shells

- Parse module-wide source `#Config` declarations during header syntax preparation.
- Normalise each declaration into one retained source build-config contract.
- Reject body-local, nested, runtime and unsupported-type declarations before AST.
- Restrict defaults to the accepted primitive literal/`none` forms.
- Keep contract shells out of structural provider references and dependency-bound symbol bindings.
- Keep contract shells out of local declaration-ordering edges except where the later resolved
  ordinary constant participates normally.
- Prove contract collection does not change Stage 0 provider or module topology.

### Phase 5: Build the project-wide contract barrier

- Collect contracts from the selected semantic graph before any module AST uses them.
- Validate same-name compatibility once in deterministic source order.
- Resolve one value per config name through the accepted order.
- Compare already-typed explicit values with contract types without reparsing command text.
- Diagnose unknown explicit inputs only after the complete selected contract set is known.
- Run the barrier separately for every project or package boundary.
- Record name-granular fingerprints and boundary-local configuration provenance.

Review gate: audit boundary isolation, stable source graphs and the absence of a second evaluator.

### Phase 6: Feed resolved values into AST and ordinary static `if`

- Convert each resolved source contract into an ordinary folded constant fact.
- Create no runtime node, wrapper type, config-specific AST branch or HIR category.
- Preserve source declaration and command locations for diagnostics.
- Ensure build, check and dev produce identical frontend values.
- Verify a `#Config of Bool` condition uses the already-accepted ordinary static Bool `if` path.
- Verify both authored branches retain frontend diagnostics.
- Verify the inactive branch contributes no HIR, generated work, borrow/lifetime facts, link facts,
  target requirements or backend code.
- Verify runtime Bool conditions retain ordinary runtime control flow.
- Verify `#Config` values cannot alter dependency, declaration or public-surface topology.

Review gate: there is one static-control-flow owner and it has no knowledge of `#Config` origins.

### Phase 7: Implement `ProjectGlobalsInterface`

- Build stable field identities from project identity and field path.
- Project folded values, locations, fingerprints and provenance.
- Register the reserved `@project` synthetic interface in Stage 0 visibility.
- Bind explicit source dependencies through the ordinary dependency-binding boundary.
- Track dependencies at field granularity.
- Keep project-global and source-config name dependencies distinct but compatible with one
  invalidation model.

### Phase 8: Validate configuration provenance

- Carry direct project and source `#Config` provenance through constant folding, public facts, HIR,
  generated functions and active static-branch selection.
- Reject direct or transitive private configuration dependence selected by an external project or
  dependency package facade.
- Allow private internal project use where no external package export can reach it.
- Ensure inactive static branches do not retain executable provenance or generated requests.
- Ensure provenance never changes declaration identity or source graph identity.

### Phase 9: Thread command and dev state

- Thread typed inputs and `ProjectGlobalsInterface` through build, check, dev and benchmark entry
  points.
- Preserve values and their typed origins across every dev rebuild.
- Invalidate only facts that depend on a changed value.
- Remove duplicated defaulting, reparsing or String conversion logic from command-specific paths.
- Reject attempts by builder bootstrap surfaces to expose backend or platform identity flags.

### Phase 10: Migrate fixtures, scaffolds and documentation

- Rename the roadmap plan to `build-config-values-and-project-globals-plan.md` once all repository
  references can move atomically.
- Replace accepted former source examples and wording with `#Config`.
- Add migration diagnostics only if current implemented syntax requires them when this plan lands.
- Add integration cases for project, source, CLI inference, static-`if` integration and boundary
  isolation contracts.
- Update scaffolds and project-structure/build-config documentation.
- Update the compiler and build-system authorities, design-scope constraints, cheatsheet, entry-config
  plan, package plan, roadmap and progress matrix.
- Rebuild generated documentation.

## Stop conditions

Stop for review when:

- CLI value typing depends on loading a source contract
- a second primitive, numeric or quoted-literal parser is proposed
- `#Config` begins changing provider graphs, declarations or exports
- a config-specific `if`, reachability or HIR path is proposed
- an inactive branch skips frontend syntax/name/type validation
- builder values expose target, operating-system, architecture or backend identity
- dependency inputs inherit unqualified values from the consuming project
- configuration provenance requires a second package-facade policy
- a runtime `Config` wrapper or semantic type appears
- a phase crosses more than two unlisted ownership boundaries

## Required tests

Cover:

### Source vocabulary

- `Import` as a valid user type and `import` as a valid ordinary identifier
- `config`, `Config`, case variants and leading-underscore shadows rejected by the canonical reserved
  name policy
- `#Config of T` treated as a qualifier with semantic type `T`
- no runtime or HIR `Config` representation

### CLI inference

- exact `true` and `false` Bool inference
- positive and negative Int inference
- decimal and exponent Float inference
- Int range and non-finite Float rejection
- valid Unicode Char literals and invalid Char syntax
- unquoted String fallback, including URLs, version-like text and empty values
- explicit quoted String values that look like Bool, Int, Float or Char
- malformed explicit String and Char quotes
- bare `none` as String rather than optional absence
- splitting only at the first `=`
- repeated and duplicate CLI inputs
- exact primitive mismatch diagnostics and matching optional promotion
- no implicit Int-to-Float or primitive-to-String coercion

### Contract and boundary behaviour

- required and defaulted project fields
- fixed project field authority
- source contract compatibility and conflicts
- restricted source defaults
- unknown explicit inputs
- module-wide-only declaration placement
- build/check/dev parity
- dev rebuild retention and targeted invalidation
- `@project` explicit dependency and collisions
- field- and config-name-level fingerprints and dependencies
- no direct re-export
- facade provenance rejection
- consuming-project input isolation from dependencies
- no config-dependent dependency, declaration or export topology

### Static-control-flow integration

- configured Bool `true` and `false` use the ordinary static `if` path
- both branches retain syntax, name, type and generic-evidence diagnostics
- selected branch preserves lexical scope
- inactive branch emits no generated requests, HIR, borrow/lifetime facts, link facts, target
  requirements or backend code
- runtime Bool conditions still lower as runtime control flow
- no target/platform config globals in built-in builder surfaces

## Validation

Every code-bearing phase runs:

```bash
cargo fmt --all
just validate
```

Run focused CLI, header, config, static-control-flow and package-boundary tests after their owning
phases. Run the documentation release build when source docs change.

## Final audit

Verify:

- the accepted source spelling is `#Config` everywhere and the former spelling has no implementation
  path
- `Import` is free for users and the `config` identifier family is reserved once
- one typed primitive input vocabulary exists
- one CLI parser infers values before contract discovery
- one CLI/programmatic resolution path exists
- String fallback and explicit quoted String forcing are deterministic
- optional absence comes from omission/defaulting, not an untyped CLI `none`
- source contracts resolve before AST without a second evaluator
- resolved config inputs become ordinary folded constants
- configured Bool values use the ordinary static Bool `if` owner
- source graphs, declarations, exports and package topology never depend on config values
- both static branches remain frontend-valid while inactive executable work is absent downstream
- no platform or backend identity enters Moth source through builder globals
- `@project` is immutable, explicit and permanently reserved
- project and dependency configuration namespaces remain isolated
- private configuration provenance cannot leak through package facades
- package dependency declarations remain outside this plan
