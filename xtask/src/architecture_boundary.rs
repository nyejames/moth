//! The compiler/build dependency-direction rules the source audit enforces.
//!
//! WHAT: three source-shaped bans that Rust visibility cannot express — production code outside
//!       `src/compiler_frontend` naming a frontend semantic stage owner, `compiler_frontend` code
//!       naming the build system or project tool's config container and `boracle/oracle` code
//!       naming static-solver owners or reaching a static-solver module.
//! WHY: `style-guide.mtf > Production layering and stage ownership` is a dependency rule. Every
//!      owner named below is now `pub(in crate::compiler_frontend)` or narrower, so `rustc`
//!      already rejects a build-side caller. What `rustc` cannot reject is the edit that widens
//!      one of them back to `pub(crate)` in order to make such a caller compile — that edit is
//!      legal Rust and silent. This file makes it fail in the same commit that performs it.
//!
//! The second rule is different in kind: nothing in the module tree stops `compiler_frontend` from
//! importing `crate::build_system`, because both are crate-internal. That direction is only
//! checkable across files, so text is the only tool available for it.
//!
//! The third rule is also different in kind: both the operational oracle and static solver are
//! crate-internal, so `rustc` cannot express the direction that keeps `boracle/oracle` independent
//! of static solving. Source text is the only available tool, while the comparison-layer reducer
//! lives at the `boracle` level.
//!
//! These are reintroduction tripwires, not behaviour tests. The behaviour each protects — that one
//! module compilation, one config compilation and one template fold each run behind a named
//! compiler service — is owned by those services' own tests. Text matching cannot prove it: a
//! rename or an equivalent reimplementation would pass. Saying so is why the rules live here.

/// The frontend semantic stage owners production code outside the compiler must not name.
///
/// Each entry mirrors a declaration that is currently frontend-private. The list is the record of
/// which names that privacy is load-bearing for, so widening one is visible as a rule violation
/// rather than as an ordinary visibility tweak.
///
/// Stage 0 source preparation is deliberately absent. `tokenize`, `prepare_file_frontend_local` and
/// `prepare_header_syntax` are the documented build-system exception: deciding which source belongs
/// to a module and when to prepare it is scheduling policy, and the canonical architecture boundary
/// records that as an allowed direction. Everything after retained syntax is compiler-owned.
const FRONTEND_SEMANTIC_STAGE_OWNERS: &[&str] = &[
    // Interface binding and local declaration ordering, including the facade methods that wrap
    // them. A banned owner reachable through a one-line `pub(crate)` wrapper is not banned.
    "bind_module_headers",
    "resolve_module_dependencies",
    "sort_headers",
    // Raw AST construction. `AstBuildInput` is required by `Ast::new`, so naming it is the
    // construction, while `Ast` alone stays available to whoever legitimately holds one.
    "AstBuildInput",
    "AstBuildContext",
    "headers_to_ast",
    // HIR lowering. The module path is the marker rather than `lower_module`, which is also an
    // ordinary backend emitter method name.
    "hir_builder",
    "generate_hir",
    // Pre-AST public-interface projection and draft construction. The completed interface stays a
    // documented handoff; building one is not.
    "PublicInterfaceDraftBuilder",
    "build_direct_export_seed",
    "build_public_source_nominal_origin_index",
    "build_public_source_trait_origin_index",
    // Borrow execution.
    "check_borrows",
    // Generated semantic completion. `style-guide.mtf` names call-summary installation directly:
    // "No build-owned function installs call summaries, rewrites HIR or reruns a compiler
    // analysis." Storing, publishing and reusing a completed sidecar stays build-owned.
    "materialise_generated_request_roots",
    "run_generated_summary_convergence",
    "install_exact_concrete_call_summaries",
];

/// What production `compiler_frontend` code must not name.
///
/// The project tool's settings module is not banned wholesale: the compiler reads authored-name
/// constants such as the implicit start function from it today. The banned name is the mutable
/// configuration container itself, which is what the layering rule is about.
const BUILD_AND_PROJECT_CONTAINERS: &[&str] = &["crate::build_system", "settings::Config"];

/// What production Boracle oracle code must not name.
///
/// WHY: the operational oracle reads runtime state while the static solver reads the problem's
///      static side. Keeping those consumers apart makes the comparison-layer reducer the only
///      cross-side consumer, and its `boracle`-level location makes that boundary visible.
///
/// The table mirrors every importable item of the static-solver modules, grouped by owning file,
/// plus the two differential comparators whose only legal caller is the comparison layer. It is
/// a backstop for bare spellings, not the boundary itself: the boundary is the module set in
/// [`BORACLE_STATIC_SOLVER_MODULE_PREFIXES`], so a name missing from this table is still caught
/// when it is imported through its module. The loans-private `EventGraph` is listed
/// deliberately, so widening it to be importable shows up as a boundary violation rather than
/// as an ordinary visibility tweak.
const BORACLE_ORACLE_STATIC_SOLVER_NAMES: &[&str] = &[
    // `boracle/origins.rs`
    "OriginFact",
    "OriginSolution",
    "OriginSolver",
    "OriginTrace",
    "OriginTraceRule",
    // `boracle/relations.rs`
    "CopyGraphId",
    "DisjointReason",
    "OriginDisjointEvidence",
    "OriginOverlapDecision",
    "OriginOverlapEvidence",
    "OriginRegistration",
    "OriginRelation",
    "OriginRelationEvidence",
    "OriginRelationKind",
    "OriginRelations",
    "OriginUnknownEvidence",
    "PrecisionLossReason",
    "query_overlap",
    // `boracle/loans.rs`
    "AccessDecision",
    "ConflictWitness",
    "EventGraph",
    "ExclusiveLoanLiveness",
    "LoanFact",
    "LoanSolution",
    "LoanSolver",
    // `borrow_checker/last_use`, the static last-use vocabulary the reference solver runs on
    "FutureUseStatus",
    "LastUseAnalysis",
    "LastUseLocation",
    "LastUseObservation",
    "LastUseResult",
    "LastUseSubject",
    "LastUseWitness",
    "event_for_use",
    // `boracle/report.rs`
    "BoracleReport",
    "BoracleSolver",
    "ReactiveObservation",
    // `boracle/service.rs`
    "BoracleDump",
    "BoracleExperiment",
    "BoracleExperimentMetadata",
    "BoracleFunctionReport",
    "BoracleModuleReport",
    "BoracleReferencePromotionStatus",
    "BoracleReferenceRuleSet",
    "BoracleRuleSelection",
    "BoracleServiceOptions",
    "format_experiment_names",
    "run_hir_module",
    "solve_hir_module",
    // `boracle/differential.rs` comparators: the comparison layer, at the `boracle` level
    "compare_problem_parts",
    "compare_reference_and_experiments",
];

/// Import prefixes that reach the static-solver modules through a module path.
///
/// WHY: the name table pins the items that are load-bearing today, but the modules are the
///      boundary, and a module-path import such as `use super::loans::{LoanSolver, LoanFact}` or
///      `use crate::...::borrow_checker::boracle::origins::OriginSolver` reaches items the name
///      table may not list. Matching each module segment together with its `::` catches the
///      single, braced and fully qualified forms in one rule, reusing the same scan as
///      [`BUILD_AND_PROJECT_IMPORT_PREFIXES`].
///
/// The segments can be banned inside the oracle without identifier-boundary checking: no oracle
/// sibling is named `origins`, `relations`, `loans`, `last_use`, `report` or `service`, and
/// legitimate oracle imports run through `problem`. A module-group import such as
/// `use super::{loans}` and an alias such as `use super::loans as solver` still escape this
/// single-line match, but both then require naming a banned item at every use site, which the
/// name table catches. That is the limit of what source text can decide, and it is why this
/// rule remains a reintroduction tripwire rather than a proof.
const BORACLE_STATIC_SOLVER_MODULE_PREFIXES: &[&str] = &[
    "origins::",
    "relations::",
    "loans::",
    "last_use::",
    "report::",
    "service::",
];

/// Import prefixes that reach a banned container through a braced list.
///
/// WHY: `use crate::projects::settings::{Config, ...}` spells `settings::Config` nowhere, and the
///      braced form is already idiomatic in this tree. Matching the prefix catches the list without
///      banning the settings module wholesale, which the compiler legitimately reads authored-name
///      constants from. `crate::build_system` needs no entry here: it is banned as a whole path, so
///      every form of it already matches.
///
/// This is a single-line match, so a `Config` on a continuation line of a wrapped braced import
/// still escapes. That is the limit of what source text can decide, and it is why this file is a
/// reintroduction tripwire rather than a proof.
const BUILD_AND_PROJECT_IMPORT_PREFIXES: &[&str] = &["settings::{Config"];

/// Which boundary rule one message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryRule {
    ExternalStageOrchestration,
    CompilerDependencyOnBuild,
    OracleStaticSolverIndependence,
}

/// Apply all three boundary rules to one file's text.
pub fn audit_architecture_boundary_fragment(
    relative: &str,
    content: &str,
) -> Vec<(BoundaryRule, String)> {
    // Tests may target stage-local APIs directly: the rule bans external orchestration of the
    // stages, not a test that drives one of them from the same crate.
    if is_test_source(relative) || !relative.starts_with("src/") {
        return Vec::new();
    }

    if relative.starts_with("src/compiler_frontend/") {
        let mut findings = banned_names_in_code(content, BUILD_AND_PROJECT_CONTAINERS)
            .into_iter()
            .chain(banned_prefixes_in_code(
                content,
                BUILD_AND_PROJECT_IMPORT_PREFIXES,
            ))
            .map(|name| {
                (
                    BoundaryRule::CompilerDependencyOnBuild,
                    format!(
                        "names '{name}'; the compiler receives compiler-owned option and input \
                         values instead of reading build or project state"
                    ),
                )
            })
            .collect::<Vec<_>>();

        if relative.starts_with("src/compiler_frontend/analysis/borrow_checker/boracle/oracle/") {
            findings.extend(
                banned_names_in_code(content, BORACLE_ORACLE_STATIC_SOLVER_NAMES)
                    .into_iter()
                    .map(|name| {
                        (
                            BoundaryRule::OracleStaticSolverIndependence,
                            format!(
                                "names static-solver dependency '{name}'; the operational oracle \
                                 must remain independent of static solving, while the \
                                 comparison-layer reducer lives at the `boracle` level"
                            ),
                        )
                    }),
            );
            findings.extend(
                banned_prefixes_in_code(content, BORACLE_STATIC_SOLVER_MODULE_PREFIXES)
                    .into_iter()
                    .map(|prefix| {
                        (
                            BoundaryRule::OracleStaticSolverIndependence,
                            format!(
                                "reaches the static-solver module behind '{prefix}'; the \
                                 operational oracle must remain independent of static solving, \
                                 while the comparison-layer reducer lives at the `boracle` level"
                            ),
                        )
                    }),
            );
        }

        return findings;
    }

    banned_names_in_code(content, FRONTEND_SEMANTIC_STAGE_OWNERS)
        .into_iter()
        .map(|name| {
            (
                BoundaryRule::ExternalStageOrchestration,
                format!(
                    "names the frontend stage owner '{name}'; a shorter compiler path is a named \
                     compiler service, not a stage sequence assembled outside the compiler"
                ),
            )
        })
        .collect()
}

/// Test sources, by the two layouts this repository uses for them.
///
/// `src/compiler_tests/` is deliberately not exempt as a directory: it also holds
/// `integration_test_runner`, which ships without `#[cfg(test)]` and is production code backing
/// `moth tests`. Its remaining `#[cfg(test)]` helpers name no stage owner, so treating them as
/// production costs nothing and keeps the directory rule from hiding the runner.
fn is_test_source(relative: &str) -> bool {
    relative.contains("/tests/") || relative.ends_with("_tests.rs")
}

/// Every banned name this file names outside a comment, in table order and without repeats.
///
/// Comment lines are skipped so a doc comment may still explain which owner consumes the data a
/// module produces. Naming an owner in prose is how a handoff is documented; calling or importing
/// it is the violation.
fn banned_names_in_code(content: &str, banned: &[&str]) -> Vec<String> {
    let code_lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect();

    banned
        .iter()
        .filter(|name| {
            code_lines
                .iter()
                .any(|line| contains_whole_word(line, name))
        })
        .map(|name| (*name).to_string())
        .collect()
}

/// Every banned import prefix this file spells outside a comment, in table order.
///
/// Prefixes end mid-token by design, so identifier-boundary matching does not apply to them.
fn banned_prefixes_in_code(content: &str, banned: &[&str]) -> Vec<String> {
    let code_lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect();

    banned
        .iter()
        .filter(|prefix| code_lines.iter().any(|line| line.contains(*prefix)))
        .map(|prefix| (*prefix).to_string())
        .collect()
}

/// Whether `line` contains `word` as a complete Rust identifier.
///
/// Substring matching would report `path_format_config` for `config` and miss the point; a Rust
/// identifier is bounded by anything that is not alphanumeric or `_`.
fn contains_whole_word(line: &str, word: &str) -> bool {
    let bytes = line.as_bytes();
    let word_bytes = word.as_bytes();

    line.match_indices(word).any(|(start, _)| {
        let before_is_identifier = start
            .checked_sub(1)
            .is_some_and(|index| is_identifier_byte(bytes[index]));
        let after = start + word_bytes.len();
        let after_is_identifier = bytes.get(after).copied().is_some_and(is_identifier_byte);

        !before_is_identifier && !after_is_identifier
    })
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests;
