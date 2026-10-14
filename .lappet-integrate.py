from pathlib import Path


def replace(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    assert text.count(old) == 1, (path, old[:120], text.count(old))
    p.write_text(text.replace(old, new))

replace('Cargo.toml',
    'members = [".", "crates/moth-lexical", "crates/moth-mon", "xtask"]',
    'members = [".", "crates/lappet", "crates/moth-lexical", "crates/moth-mon", "xtask"]')
lock = Path('Cargo.lock')
text = lock.read_text()
assert 'name = "lappet"' not in text
import re
matches = list(re.finditer(r'\[\[package\]\]\nname = "([^\"]+)"', text))
position = next(m.start() for m in matches if m.group(1) > 'lappet')
text = text[:position] + '[[package]]\nname = "lappet"\nversion = "0.1.0"\n\n' + text[position:]
lock.write_text(text)

roadmap = Path('docs/roadmap/roadmap.md')
text = roadmap.read_text()
anchor = 'This is the main todo list and future design / implementation roadmap for Moth.\n'
note = '''
[Lappet](../../crates/lappet/README.md), Moth's native runtime and future package
infrastructure, has its own [roadmap](../../crates/lappet/roadmap.md),
[runtime design](../../crates/lappet/docs/design.md) and
[package-manager design](../../crates/lappet/docs/package-manager.md). Its work
happens in a separate isolated worktree and is not sequenced by this roadmap.
Compiler integration consumes delivered capabilities without resuming paused
compiler work. All user-facing tooling remains under `moth`.
'''
assert text.count(anchor) == 1
text = text.replace(anchor, anchor + note)
line = '- [Package dependency declarations and package-manager foundations](./plans/package-dependency-declarations-and-manager-foundations-plan.md)\n\n'
assert text.count(line) == 1
text = text.replace(line, '')
start = text.index('## Package manager ideas\n')
end = text.index('## Possible additional constraints\n', start)
text = text[:start] + text[end:]
old = '- remote package registries, fetching, version solving, lockfiles, publishing and package-manager policy beyond the design-gated package dependency foundations plan'
assert old in text
text = text.replace(old, '- package acquisition, admission and registry work is tracked only in the [Lappet roadmap](../../crates/lappet/roadmap.md)')
roadmap.write_text(text)

replace('docs/compiler-design-overview.md',
    '- `docs/compiler-data-layout-design.md` owns source, token and diagnostic representation, identity lifetimes and the three failure lanes.',
    '- `docs/compiler-data-layout-design.md` owns source, token and diagnostic representation, identity lifetimes and the three failure lanes.\n'
    '- [Wasm optimisation](src/developer-docs/wasm-optimisation/overview.mtf) routes to the authority for backend optimisation goals, pass placement and the policy shared with Lappet. Language semantics and validated memory contracts remain with their existing owners.')
replace('docs/compiler-design-overview.md',
    '| Reachability, target checks and backend inputs | `Per-function link facts`, `Target-contract validation` and `Backend-facing compiler handoff` | Build entry/package and builder sections |',
    '| Reachability, target checks and backend inputs | `Per-function link facts`, `Target-contract validation` and `Backend-facing compiler handoff` | Build entry/package and builder sections |\n'
    '| Wasm optimisation and native compilation policy | `Backend-facing compiler handoff` | [Wasm optimisation authority](src/developer-docs/wasm-optimisation/wasm-optimisation.mtf) and the affected semantic or memory owner |')

old = 'Canonical package identity stays separate from a local alias. Aliases affect binding/diagnostics, not artefact or resource-output identity, and collisions are checked before source compilation. The declaration carrier and resolver/catalog handoff remain design-gated under `docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md`. No config-import preamble is implied. Remote fetching, registries, version solving, lockfiles and publishing remain open design.'
new = '''Canonical package identity stays separate from a local alias. Aliases affect binding/diagnostics, not artefact or resource-output identity, and collisions are checked before source compilation.

### Project dependency declarations and resolver input

Each declaration names one direct canonical external package and may assign one
project-local root alias. Retain authored order for diagnostics. Reject duplicate
canonical declarations and colliding visible roots before resolution or source
compilation. Core, Builder and ProjectLocal packages are not acquired through
this external declaration surface. Project globals, project modules and relative
or parent paths are not canonical external package names.

Declarations establish availability before the project's source graph is built.
They do not bind package symbols into config, expose transitive dependencies or
share consumer build inputs with dependency graphs. A source clause only binds
an already registered facade. Changing an alias never changes package, artefact
or resource identity. Resource prefixes remain injective over the complete stable
package-instance identity, including the future version-instance policy.

The resolver supplies explicit, immutable records for source or compatible
precompiled packages, their dependency metadata and their compatibility facts.
Missing, duplicate and incompatible packages have distinct failures. Resolution
and compilation use deterministic canonical order. Lookup maps may accelerate
construction without becoming a second retained graph authority.

The compiler and build system consume that catalog. They do not fetch packages,
probe undeclared filesystem locations or recover missing roots through fallback
search. Config import preambles, `package_folders` and implicit `/lib` discovery
remain excluded. Exact numeric, ABI and capability checks use the existing
compatibility owners. Incompatible artefacts are rebuilt or rejected, not
repaired through guessed numeric or layout adapters.

The declaration carrier, canonical name grammar, alias replacement policy and
first local/programmatic catalog require design approval in the
[Lappet roadmap](../crates/lappet/roadmap.md). The
[package-manager design](../crates/lappet/docs/package-manager.md) owns acquisition,
managed-package classification, official/custom admission and future security
policy. Moving that work does not implement it or change compiler acceptance of
otherwise valid local packages. Remote fetching, version solving, lockfiles and
publishing remain separately gated.
'''
replace('docs/build-system-design.md', old, new.rstrip())
replace('docs/build-system-design.md',
    'The capability surface contains registered directive signatures/contexts, source-backed Core/Builder packages, binding-backed packages, template directives, import providers, runtime requirements, supported source kinds, primitive build globals and target-affinity/capability metadata.',
    '''The capability surface contains registered directive signatures/contexts, source-backed Core/Builder packages, binding-backed packages, template directives, import providers, runtime requirements, supported source kinds, primitive build globals and target-affinity/capability metadata.

Official executable environments provide the canonical Core catalogue through
official native or browser implementations, with documented host-capability gates
and unsupported outcomes. Custom builders may provide any subset for local code
without changing Core meanings or reserved names. Official certification and
managed Core-dependent package admission are separate package-manager policies,
not a consequence of a matching capability surface. The
[Lappet design](../crates/lappet/docs/design.md) owns native implementation policy.
Tooling overlays inspect the selected environment rather than owning another
runtime. The [Wasm optimisation authority](src/developer-docs/wasm-optimisation/overview.mtf)
owns optimisation goals and the separate Moth-profile/native-strategy choices.''')

replace('docs/src/developer-docs/@page.moth',
    '    - @./project-builders (Rust project builders): public SDK tutorial, host bindings, capabilities and application output',
    '    - @./project-builders (Rust project builders): public SDK tutorial, host bindings, capabilities and application output\n'
    '    - @./wasm-optimisation (Wasm optimisation): shared compiler and native execution policy\n'
    '    - @../docs/lappet (Lappet): planned native runtime, embedding and package infrastructure')
replace('docs/src/docs/@page.moth',
    '    - @./async (Async)',
    '    - @./async (Async)\n    - @./lappet (Lappet native runtime and package infrastructure)')
replace('docs/src/developer-docs/overview.mtf',
    'For build-system or project orchestration questions, read `docs/build-system-design.md`.',
    '''For build-system or project orchestration questions, read `docs/build-system-design.md`.

For backend optimisation goals and the division between Moth and native code
generation, read `docs/src/developer-docs/wasm-optimisation/overview.mtf`.
Lappet's runtime and package-manager designs live in `crates/lappet/docs/`, with
independent implementation checkpoints in `crates/lappet/roadmap.md`.''')
replace('docs/src/developer-docs/project-builders/overview.mtf',
    'configuration, assembly, lifecycle inputs and output ownership.',
    '''configuration, assembly, lifecycle inputs and output ownership.

`crates/lappet/docs/design.md` owns the planned native Wasmtime implementation of
the host boundary. It reuses this SDK rather than defining another registration
system. `crates/lappet/docs/package-manager.md` owns official/custom package
admission. Using Lappet never certifies a custom builder. The shared optimisation
policy is routed by `docs/src/developer-docs/wasm-optimisation/overview.mtf`.''')

p = Path('docs/src/developer-docs/compiler-design/overview.mtf')
s = p.read_text()
heading = '## Where authority lives\n'
assert s.count(heading) == 1
s = s.replace(heading, heading + '''
`docs/src/developer-docs/wasm-optimisation/overview.mtf` routes to the permanent
authority for optimisation goals and the Moth/native-code boundary. It keeps
pass policy separate from compiler-stage and memory-model contracts.
''')
p.write_text(s)

replace('docs/src/docs/packages/package-origins-and-backing.mtf',
    'The builder decides which packages exist. Unsupported packages are rejected with an unsupported-by-builder diagnostic. The HTML builder opts into `@html`, `@web/canvas`, `@web/graphics` and current optional Core packages.',
    '''Builders provide implementations and additional packages without redefining
Core semantics. Official browser and native environments provide the canonical
Core catalogue, including its documented host-capability gates. Custom builders
may provide a subset or no public Core implementation. Reserved Core identities
and the `io` prelude keep their meaning even when an operation is unavailable.
The progress matrix records current implementation gaps separately from this
design contract.

The future official package manager classifies managed Moth-source packages as
no-core or Core-dependent over their complete dependency closure. Official
certified environments may consume both. Custom builders receive no-core
packages only, even when they use Lappet or implement all Core APIs. Certification
is a Moth project decision, not inferred from matching signatures.

Compiler-internal helpers and the unused `io` prelude do not make a package
Core-dependent. No-core still requires support for the language features it
executes. Managed Builder-dependent and foreign-code packages are deferred.
Local and builder-supplied packages retain their normal compiler rules. See
@../../lappet (Lappet) for scope and the package-manager design.''')
replace('docs/src/docs/packages/core/@page.moth',
    '''    Builder and backend support can differ. The @../../progress/packages-and-builders
    (packages and builders progress matrix) records current package availability and
    important accepted follow-ups.''',
    '''    Official environments target the complete Core catalogue with documented
    host-capability gates. Custom builders may supply only the implementations
    they need. The reserved `io` namespace alone does not guarantee that a custom
    host implements its operations.

    The @../../progress/packages-and-builders (packages and builders progress
    matrix) records current availability and implementation gaps. @../../lappet
    (Lappet) describes planned native Core support and official package admission.''')

p = Path('index.md')
s = p.read_text()
anchor = '## Standalone lexical and MON crates\n'
assert s.count(anchor) == 1
s = s.replace(anchor, '''## Lappet runtime and package infrastructure

- [Lappet crate](crates/lappet/README.md) and [library entry](crates/lappet/src/lib.rs): dependency-free initial workspace library, with no runtime API yet.
- [Runtime design](crates/lappet/docs/design.md), [package-manager design](crates/lappet/docs/package-manager.md) and [independent roadmap](crates/lappet/roadmap.md): accepted contracts and isolated-worktree implementation checkpoints.
- [Wasm optimisation reference](docs/src/developer-docs/wasm-optimisation/overview.mtf) and [detailed authority](docs/src/developer-docs/wasm-optimisation/wasm-optimisation.mtf): shared compiler/native-code policy. Its [page](docs/src/developer-docs/wasm-optimisation/@page.moth) composes the site.
- [Lappet introduction](docs/src/docs/lappet/lappet.mtf) and [page](docs/src/docs/lappet/@page.moth): application and embedding overview, with implementation status explicit.

''' + anchor)
p.write_text(s)

Path('docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md').unlink()
