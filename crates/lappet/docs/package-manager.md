# Lappet package-manager design

> Accepted policy direction with explicit design gates. The initial crate has
> no package acquisition, resolver implementation or security assessor. The
> [roadmap](../roadmap.md) owns delivery. Registry protocols, manifest syntax,
> thresholds and assessment algorithms require later review.

The official package manager manages Moth-source dependencies through the `moth`
CLI. Its library is usable without the Lappet runtime or Wasmtime. It deliberately
adds friction to dependency selection: uncertain security evidence or excessive
complexity causes rejection rather than an optimistic installation.

## Ownership

The [build-system authority](../../../docs/build-system-design.md) owns
project dependency declarations, canonical package graphs, namespace rules,
resolver inputs and output placement. The
[compiler authority](../../../docs/compiler-design-overview.md) owns semantic
identities, types, public interfaces and target validation.

This document owns acquisition and admission policy, package classification and
the future registry direction. It replaces the former package-manager
foundations work item and the roadmap's exploratory package-manager notes.
Retirement transfers that work. It does not claim the old plan was implemented.

The manager resolves and obtains explicitly declared dependencies, then supplies
the canonical build-system catalog. It consumes compiler-produced compatibility
facts rather than parsing Moth clauses, guessing types or maintaining another
semantic resolver. Source clauses bind available roots and never download code.

## Dependency categories

Keep three implementation categories distinct:

| Dependency | Implementation and distribution |
|---|---|
| Moth-source package | Compiled through Moth's ordinary separate package graphs |
| Rust host package | Part of the selected builder's trusted Cargo dependency graph and explicitly registered host API |
| C library | Vendored or linked through a Rust wrapper owned by the builder or a selected Lappet host implementation |

Initial managed packages use the first category only. They cannot install Rust
or C libraries, run native build scripts or extend an already compiled host.
Matching Moth-facing signatures do not make native code sandboxed.

V1 managed packages also exclude dependencies on public Builder packages,
project-authored JS/WAT bindings and foreign Wasm components. Builder-specific
Moth libraries remain valid local or builder-supplied code. Their managed
publication and capability negotiation are deferred, not prohibited forever.
Trusted Core implementation files are part of a platform implementation, not
foreign dependencies added by an ordinary Moth-source package.

Precompiled Moth Wasm remains MothSource semantically. The shared compiler
boundary must leave room for compatible precompiled descriptors, but initial
manager distribution is source-only. Persistent artefact formats, native AOT
caches and precompiled distribution remain separate later work.

## Two Core-compatibility categories

The manager derives a category from the complete declared dependency closure and
compiler-owned Core-use facts. Authors cannot certify it through a manifest
flag. Package admission is conservative and independent of one application's
final executable reachability.

**No-core** means the package and every declared Moth dependency contain no
public Core import or use. The implicit reserved `io` namespace alone does not
count. An actual `io` call, a public Core type or a folded Core constant does
count. Compiler primitives, intrinsic collection operations and private runtime
helpers are not public Core dependencies.

**Core-dependent** means a public Core dependency occurs anywhere in that
closure. It propagates through private helpers, re-exports and transitive
packages. A declared dependency still counts when one consuming application
does not execute its functions. An unused explicit Core import cannot be used
to claim no-core status.

Record the exact canonical interfaces and target requirements beside the coarse
category. No-core means no public Core host API requirement. It does not promise
that a tiny interpreter supports every language feature, allocation operation
or numeric representation used by the package.

A package depending on a Builder API is outside these two managed categories.
It must not be misclassified as no-core simply because that API is not Core.
Classification examines semantic dependencies and approved provider facts,
not source-text searches or a label supplied by the publisher.

## Official environments and custom builders

Official Core compatibility is certification by the Moth project. It belongs
to the deliberately supported official builder/environment set, not to any
builder that happens to expose matching functions.

| Selected environment | No-core managed package | Core-dependent managed package | Builder-dependent managed package |
|---|---|---|---|
| Official certified browser or native builder | Eligible | Eligible subject to language and host capability checks | Deferred |
| Custom builder, including one using Lappet | Eligible | Rejected | Deferred |

Custom builders may implement no Core, some Core or all Core for their own code.
They keep canonical Core meanings and reserved names, including `io`. This
never grants permission to acquire Core-dependent packages through the official
manager. Perfect API coverage and Wasmtime presence are not certification.

The official builder catalogue and its trust mechanism need a reviewed design.
A public Rust setter, free-form builder name or package metadata claim must not
be sufficient to select the official admission path. Extending the official set
requires an explicit Moth project decision, Core conformance evidence and a
published support matrix.

The compiler still validates local code against the actual selected builder.
Manual dependency arrangements are outside the manager's admission guarantee,
not outside compiler safety, namespace or target rules. This policy cannot
prevent users modifying or replacing the open tooling. It defines the supported
managed workflow rather than an unforgeable language security boundary.

Official implementations provide the complete Core catalogue with the canonical
host-dependent gates and permitted unsupported outcomes. Certification does not
make a headless server supply a window, grant runtime permissions or guarantee
success for a fallible operation. Tooling checks the selected environment's
contract instead of pretending to supply its own runtime.

## Dependency complexity

Keep dependency topology and security assessment separate. A Golden package can
contain unsafe or malicious behaviour. A Lead package can be small, well reviewed
and acceptable. Neither label is a security certificate.

### Depth tiers

Calculate depth over the resolved graph of external Moth-source package
instances. A dependency-free package has depth zero. Otherwise its depth is one
plus the maximum depth of its direct dependencies. Reject cycles separately.
Core packages do not add tier depth.

| Tier | Depth | Meaning |
|---|---|---|
| Golden | 0 | No external Moth-package dependencies |
| Silver | 1 | Dependencies are Golden |
| Bronze | 2 | Dependencies are Golden or Silver, including at least one Silver |
| Lead | 3 or more | A deeper dependency chain exists |

Assign the lowest applicable tier. Public Core use is independent: a Golden
package can be Core-dependent. Use canonical resolved identity and version,
never consumer aliases or the number of import sites, to identify graph nodes.
The version-instance policy itself remains part of resolver design.

Lead does not cause rejection by itself. A future registry may list Lead
packages, and the manager can admit one whose complete evidence satisfies
policy. Additional friction is justified by the actual graph and risk, not by
its name alone.

### Initial metrics basis

Retain these independent measurements rather than collapsing them into an
unexplained weighted score:

| Metric | What it exposes |
|---|---|
| Maximum dependency depth and tier | Indirect layering |
| Direct external dependency count | Immediate coupling |
| Unique transitive external dependency count | Breadth hidden by a shallow tree |
| Candidate's own uncompressed source bytes | A large dependency-free package |
| Unique closure source bytes, including the candidate | Total source burden |
| Largest individual package in the closure | A large dependency hidden behind a small facade |

Measure both the candidate closure and the resulting project graph. Also report
the incremental packages and bytes introduced by the proposed change. A shared
dependency is counted once per resolved instance, not once per path to it.
Changing an alias does not lower the count.

Keep absolute size in the analysis. Inlining or vendoring dependency code does
not make that code disappear. Include generated and vendored source rather
than rewarding a superficial reduction in package count. Separately report
archive expansion and non-source asset weight so opaque payloads cannot hide
behind a source-only metric. Source size and transfer size describe different
costs and should remain distinguishable.

This is the starting basis for review, not a settled scoring algorithm. A later
checkpoint must define file inclusion, canonical byte accounting, version
multiplicity, shared closures, symlinks, generated files, archives, resource
bounds and reproducible metadata. It must calibrate warning and rejection
thresholds using representative small and large packages. Do not invent numeric
limits or promote line count into proof of quality.

Registry metadata can advertise depth, breadth and size before source download.
Treat it as a bounded preflight input, then verify it against the staged source
and actual resolved graph. Inconsistent or incomplete metadata cannot silently
pass admission. Source bytes counter some incentives to copy code for a better
tier, but provenance and assessment still need independent checks.

## On-demand admission

A security assessment is performed when the user opts to add a package. It is
about the exact candidate and dependency closure, not a permanent public
"safe" badge. An eventual hosting service can display topology and size without
claiming that an add-time security assessment has already occurred.

The future managed workflow is:

1. Resolve the proposed dependency transaction from bounded metadata. Check
   category, official/custom eligibility, complete dependency topology and
   preliminary size limits before fetching source archives.
2. Reject a disallowed or excessive proposal with the blocking reasons. A
   rejected proposal does not proceed to an override confirmation.
3. Obtain the required source bytes in isolated staging and verify their
   integrity. Downloading for assessment is not installing a project dependency.
4. Reconcile source content, dependency declarations and metadata. Inspect the
   exact closure through isolated compiler and security checks. A changed graph
   or identity requires a new assessment, not reuse of stale approval.
5. Report the tier, breadth, sizes, findings, checks completed, checks unavailable
   and evidence limits. Evaluate the resulting project graph as well as the
   individual candidate.
6. Reject by default when a required check fails, cannot run or supplies
   insufficient evidence. Also reject policy violations and unacceptable risk.
7. For an admissible proposal only, show the report and ask for explicit user
   confirmation. Commit the exact approved dependency change transactionally.

Keep the manifest, lockfile and project dependencies unchanged until both policy
admission and confirmation succeed. Cancellation or failure leaves the prior
project intact. Approval applies to the reviewed versions, content identities,
policy and environment. Concurrent project changes or a different resolved
closure invalidate it. The detailed identity and transaction format is a later
checkpoint.

A normal managed command has no force-install bypass for failed checks. Users
can make manual arrangements outside the manager, but those gain no admission
or official Core guarantee. In unattended operation, missing approval is not
approval. Any reusable or non-interactive approval contract requires separate
review before it is implemented.

### Assessment boundary

The manager favours false rejection over false assurance. It must not convert
unknown provenance, unsupported analysis or missing checks into a favourable
score. A report may eventually include an ordinal rating, but concrete findings
and evidence coverage remain necessary. Passing the policy is not proof that
source is harmless.

Analysis itself processes hostile input. Its future threat model must cover
compiler/parser vulnerabilities, oversized or deceptive archives, path escapes,
resource exhaustion, evaluator access, dependency substitution and misleading
reports. Stage and inspect without ambient credentials or network access in the
analysis environment. Fetching has a separate controlled role.

No native build scripts or installation code run as part of inspection. Tests
or other package code can run only after an explicit bounded dynamic-analysis
policy has been approved, with its own isolation. Merely calling something a
compiler check does not establish a sandbox.

The required checker set, isolation implementation, risk scale, resource limits,
policy maintenance and handling of stale evidence are deliberately undecided.
Remote automatic addition remains unavailable until the necessary checks and
confirmation transaction are implemented. No permissive fallback substitutes
for a missing assessor.

## Registry and publishing

The future registry is curated around small, inspectable Moth-source packages.
Publication may require compiler validation, tests, source integrity, provenance
and quality checks. Immutable version/content identities and explicit dependency
metadata are design goals, with their exact protocol still open.

Publication and add-time admission have different responsibilities. A registry
listing does not exempt the selected closure from inspection when a user adds
it. Dependency tiers can be public metadata. Lead packages remain eligible for
review rather than facing an automatic depth-only ban.

## Foundations retained from the retired plan

Permanent declaration, namespace and resolver invariants now live in
[Project and package topology](../../../docs/build-system-design.md#project-and-package-topology).
In particular, declarations stay separate from file-local clauses, aliases never
change canonical identity and each source dependency owns an isolated immutable
compilation boundary. Acquisition never moves into source binding.

The first foundations work is a local or programmatic resolved catalog, not a
remote installer. Before implementing it, review the declaration carrier,
canonical name grammar, alias replacement rules, allowed local source and exact
catalog handoff. Preserve the rejection of config import preambles, implicit
`/lib` or `package_folders` discovery and fallback namespace probing.

The roadmap retains the old plan's producer/consumer sequence and acceptance
obligations. They include duplicate rejection before resolution, direct-only
visibility, private project globals, exact numeric/ABI compatibility, immutable
publication and resource identity independent of aliases. The new work must
consume current delivered compiler contracts rather than restoring historical
numeric profiles or old source grammar.

## Later design

Revisit version solving, lockfiles, registry transport, local/Git overrides,
publishing, assessment tooling and threshold calibration at their own gates.
Persistent source and compiled artefact caches need distinct policies. Deferring
AOT caching does not make acquired source identical to a machine-code cache.

Builder-dependent managed packages will eventually need versioned API identities
and transitive capability requirements. Record exact requirements now through
existing compiler facts, but defer general provider negotiation. Foreign Wasm
components, native host acquisition and dynamic native extensions remain beyond
V1. Optional HTTP and cryptography helpers belong to later builder/Lappet
library design rather than this manager's package-eligibility mechanism.
