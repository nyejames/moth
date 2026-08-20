# Moth Test-Suite Honesty and Infrastructure Hardening Plan

## Current state

```text
WORK_ID: test-suite-honesty
WORK_SOURCE: docs/roadmap/plans/test-suite-honesty-and-infrastructure-hardening-plan.md
BASE_REVISION: f41f93a7a (post-TIR, post-benchmark-counters-timers)
STATUS: active — Phase 7 complete, ready for Phase 8; one exposed failure is open in the ledger
CURRENT_SCOPE: Phase 7 integration contract honesty (paused before Phase 8)
COMPLETED:
  Phase 0: baseline established (4314 unit tests, 0 ignored, 1699 integration cases correct,
    1851 backend executions); durable inventory at docs/roadmap/evidence/test_honesty_inventory.json;
    feature lane mapping documented (default pass, timers pass, detailed_timers pass,
    benchmark_counters pre-existing failure); stale CFG timer language fixed
  Phase 1: test_fs helpers with symlink_metadata; temp_dir→unused_temp_path rename;
    ~250 callers migrated to tempfile::tempdir(); hardcoded /tmp/ removed;
    Linux non-UTF-8 fixtures fixed; all negative exists() assertions migrated to
    assert_path_missing; fixture discovery fail-closed; created-workspace audit completed:
    all remaining created-workspace fixtures migrated to tempfile::tempdir() with owned
    cleanup; only 3 genuine uncreated-path contracts remain (validate_dangerous_project,
    output_defaults, output_overrides); durable hard finding temp_dir_uncreated_paths marked resolved
  Phase 2: infrastructure_errors_for_tests() iterator; test_diagnostics helpers;
    assert_exact_infrastructure_error tightened to require exactly one error diagnostic;
    assert_output_rejection helper with typed OutputRejectionReason metadata;
    OutputRejectionReason enum (29 variants) added to production writer as typed reason seam;
    file_error_with_rejection_reason function; all file_error_messages calls in writer.rs
    and manifest.rs updated with specific reasons; all 26 broad is_err() assertions in
    build_orchestration_tests.rs migrated to assert_output_rejection with exact reasons;
    benchmark seed tests use structured FrontendBenchmarkError identity:
    FrontendBenchmarkFailureKind enum (TimingSession, InvalidUtf8Path, PathValidation,
    Bootstrap, Compilation) plus stable diagnostic_codes; missing-file and invalid-syntax
    negative tests assert the typed kind and exact diagnostic code instead of rendered
    substrings; rendered message preserved for CLI/tool output; missing-file benchmark uses
    tempfile not fabricated /definitely/does/not/exist.moth path; busy raw-session benchmark
    test uses a valid owned tempfile entry and asserts FrontendBenchmarkFailureKind::TimingSession
    with empty diagnostic_codes instead of a fabricated /definitely/does/not/exist.moth path;
    typed error seams for integration fixture/expectation loading and runner harness failures:
    FixtureLoadError{kind,message} (Filesystem/Manifest/ExpectationParse/ExpectationContract/
    FixtureContract/PathBoundary) threaded through fixture.rs, expectations.rs, manifest.rs and
    golden discovery; TestRunnerError with Options/SuitePolicy/InventoryReport/TriageReport/
    Selection/ThreadPool/Fixture kinds across run_all_test_cases/run_loaded_suite/options.validate/
    report persistence/selection/thread-pool/env parsing; negative self-tests in runner.rs,
    manifest.rs, expectations.rs, fixture.rs assert error.kind alongside message prose;
    CLI reads error.message for display; all broad is_err() seeds in compile_project_frontend_tests.rs
    (incl. the named Stage 0 false-positive seed) converted to let-Err-unwrap with exact diagnostics
    (Stage 0 seed asserts MOTH-SYNTAX-0019 and error_count 1);
    config_and_write_time_containment_share_canonical_classification uses assert_output_rejection
    with "output-root-not-inside-project"
  Phase 3: WarningBuilder StringId uses active string_table; CurrentDirGuard redesigned with
    Option<PathBuf> take pattern; CurrentDirGuard restore-failure injection seam;
    CurrentDirGuard Drop reports restore failure to stderr during unwinding;
    finish() handles None previous (returns Ok instead of panicking);
    current_dir_guard_finish_returns_error_when_restore_fails calls finish() directly
    under an injected restore that actually restores CWD then returns the synthetic error,
    proving the exact returned error; obsolete test_restore seam removed; unwind test
    explicitly restores CWD and asserts exact panic payload; CaseExecutionResult
    constructors removed from production;
    all synthetic fixtures carry valid build_result or messages;
    ScopedEnvVar panic-safe env guard; surface_thread_panic replaces discarded joins
  Pre-Phase-4 review pass: unused_temp_path now proves its non-existence contract with
    symlink_metadata; assert_panics_with added so helper self-tests prove the panic reason
    instead of accepting any panic (test_fs, test_diagnostics, timers conflict tests);
    assert_output_rejection and assert_diagnostic_reason panic on a missing reason instead of
    comparing a "<none>" placeholder; xtask gained its own test_fs owner and 14 negative
    Path::exists assertions in bench_history/benchmark_workspace were migrated to
    assert_path_missing; added read_utf8 invalid-UTF-8 and read_bytes missing-file regressions
  Phase 4: BuiltArtifactIndex (integration assertions) built once in validate_success_result
    before any success assertion and consumed by the HTML and HTML-Wasm baselines, artifact
    assertions, absence checks, goldens and both rendered-output harnesses; construction rejects
    duplicate normalized paths, case-only portability aliases and non-UTF-8 relative paths, and an
    ambiguous artifact set fails the case as HarnessFailed; find_output_file and
    collect_built_artifact_paths deleted; artifact absence now normalizes the authored path;
    BuiltOutputs index added for build-system tests (at/exactly_one/exactly_one_path/
    none_matching plus html_text/js_text kind accessors) with its own self-tests;
    reachable-dependency test asserts the exact emitted path set plus the lowered dependency body
    and its call site; JS glue, runtime-module and canvas tests assert exactly-one glue/canvas
    artifacts, the exact runtime module path, and that the page imports the module actually
    emitted; frontend pipeline tests assert exact function-origin multisets and borrow-summary
    coverage instead of functions_analyzed >= N; borrow-checker pipeline test asserts
    statement/terminator fact sets equal the lowered statements/blocks with an exact snapshot
    count; source-package benchmark warning test asserts an exact warning multiset; HIR local
    lowering tests assert exact authored locals and exactly one assignment to the authored local;
    the diagnostics include_str! ban renamed to state it is a source-text tripwire, not behavior
    evidence (behavior owners named in its doc comment), pending the Phase 8 audit move
  Phase 4 closeout (review response): both artifact indexes now derive validity and collision
    identity from the canonical output_path_identity instead of a harness-local path taxonomy, so
    the harness rejects exactly the destinations the output writer rejects (parent segments,
    reserved device basenames, invalid components) and folds ASCII case only, matching production
    rather than Unicode-folding distinct destinations together; BuiltOutputs gained the same
    canonical collision check including case aliases, which it previously lacked entirely;
    ArtifactIndexError gained an InvalidOutputPath variant carrying the writer's reason; the
    index self-tests match typed variants instead of message substrings, with one separate test
    owning the rendered wording; new regressions cover an ASCII case alias, a non-ASCII pair that
    must stay distinct, a parent segment and a reserved device basename in both indexes; glue
    selectors anchored with a shared GLUE_MODULE_PREFIX and starts_with (matching the provider,
    canvas and runtime selectors) with a nested-path regression; BuiltOutputs and its self-tests
    moved from the shared build_system/tests/mod.rs to their only consumer,
    build_system/tests/build_dependency_tests.rs
  Phase 5: golden comparison decides artifact kind before reading content — Directory and
    NotBuilt are kind mismatches instead of empty bytes, text goldens require strict UTF-8
    (invalid UTF-8 is HarnessFailed), binary/wasm compare bytes only; regressions for
    directory-vs-empty-file, unbuilt path, invalid UTF-8, binary bytes and duplicate paths;
    HTML shell contract replaced with one ordered, bounded, exactly-once marker contract
    (html_shell_violation + typed HtmlShellViolation) owned in the integration assertion module
    and consumed by html_project's assert_has_basic_shell, deleting the duplicated fragment list;
    HTML-Wasm baseline now derives its required exports from the emitted page.js
    (instance.exports.<name> classified as Func or Memory) and checks each against the module's
    typed export kind, with an explicit runtime ABI floor that includes moth_start, which the old
    name-list baseline never required; collect_wasm_exports returns a typed name→kind map and
    rejects duplicate export names, collect_wasm_imports rejects duplicate module.name identities;
    the page script include must appear exactly once inside the body; Node harness moved to an
    owned tempfile workspace (tempfile promoted to a normal dependency because the runner ships in
    the binary) with reported cleanup failure and a documented bounded retry for the Windows
    removal race; run_node_script enforces a 30s deadline with kill-and-reap, drains stdout and
    stderr on their own threads under a 4 MiB bound, decodes protocol stdout strictly and describes
    stderr for reports without letting a decode failure replace the real boundary; typed
    RenderHarnessError{kind,message} with Artifact/Workspace/Spawn/Timeout/ExitStatus/
    OutputDecoding/OutputProtocol/ScriptShape/Cleanup; permissive script scanning replaced by a
    supported-shape parser (case-insensitive tags, quoted attribute values, data blocks skipped,
    external src / unknown type / nomodule / async / malformed or unterminated tags rejected);
    HTML-Wasm harness resolves artifacts through __dirname so no workspace path crosses a text
    boundary; lossy conversions removed at the integration entry path, MOTH_TEST_THREADS and the
    module-discovery test helpers; node_harness.rs added to the timers-erasure wall-clock allowlist
    beside the other subprocess-deadline owners
  Phase 5 closeout (review response): `type="module"` is rejected as an unsupported script shape
    instead of being executed as a classic script — the harness materializes no emitted glue,
    provider or runtime module and no import map, so it cannot run the module graph the HTML
    backend emits when bundle_import_preamble is present; an execution-level regression drives
    validate_success_result with a module page and asserts HarnessFailed naming the module shape,
    beside the extraction-level rejections; goldens own their expected artifact kind
    (GoldenFile.expected_kind, decided by the authored golden's extension) and a cross-kind match
    is rejected before content is read, with regressions for a JS golden against generic bytes, a
    wasm golden against generic bytes, an HTML golden produced as JavaScript outside the universal
    index.html path, and a binary golden that must still match generic bytes; both Node capture
    threads are joined before any result is inspected, so no failure path drops a live thread, and
    a stderr-capture failure never replaces the boundary that actually failed; terminate() always
    attempts the reap even when kill() fails (the child exiting between the final try_wait and the
    kill) and reports the kill and wait outcomes separately; focused self-tests added for the three
    remaining harness failure classes — Artifact (missing artifact and wrong artifact kind through
    a test-only artifact-requirement seam), Workspace (a write blocked by a directory of the same
    name) and Spawn (a nonexistent interpreter through the process owner's executable parameter,
    never a PATH mutation); the script parser fails closed on nameless or malformed attribute
    tokens instead of skipping them; the HTML shell contract is structural — script and style
    content is the opaque payload the shell inserts, so a JavaScript string spelling `</body>` is
    no longer counted as a second closing-body element, while a marker repeated in real markup is
    still rejected
  Phase 6: the collector drain-synchronization seam is a condition variable instead of three
    atomics plus a 1,000,000-iteration yield loop — pause_record_admission_for_test publishes one
    RecordAdmissionState (paused / admission_reached / session_deactivated) under a mutex, the
    targeted recorder parks on Condvar::wait instead of spinning, and the two test waits
    (wait_for_paused_record_admission_for_test, wait_for_session_deactivation_for_test) use a 30s
    wall-clock deadline purely as deadlock protection and return the observed state, so a timeout
    names what it gave up on; wait_for_timing_flag deleted; surface_thread_panic moved from the
    timing tests to compiler_tests::test_support beside a new await_worker_completion that
    receives a worker's completion signal and then joins it, so a worker panic is the reported
    cause instead of a receive timeout; the dev-server SSE and partial-request tests now own
    their server threads (both handles were previously dropped) and the SSE registration wait is
    a bounded deadline reporting the observed client count instead of a fixed 20x10ms poll;
    WatchSession::drop reports a panicked polling worker instead of discarding the join;
    write_project_outputs returns a typed OutputWriteSummary (Written / SkippedUnchanged /
    DirectoryCreated per authored relative path, looked up through the canonical
    output_path_identity) and both filesystem-timestamp tests assert that outcome with their 30ms
    sleeps removed; the build command reports emitted artifacts instead of planned ones, so a
    NotBuilt entry is no longer counted as a built file, with a focused CLI regression;
    the two renderer-boundary tests dropped their 5ms "simulate renderer work" sleeps, because the
    scripted duration handed to the renderer is the ordering evidence and the sleep changed
    nothing; frontend_benchmark_runs_for_simple_file no longer asserts total_ms > 0 (a
    clock-resolution claim) and instead requires a usable measurement plus exactly-once
    schema-named stage rows including the frontend spine; the observed busy-raw-session flake is
    fixed by replacing the process-global "outer snapshot has zero samples" proxy with two
    deterministic ordering tests (a missing entry path must fail as TimingSession rather than
    PathValidation; invalid source must fail as TimingSession rather than Compilation);
    xtask gained a stress mode with a `just stress [repeats]` recipe that runs the unit and
    integration suites at one thread, default parallelism and 16 threads, repeats each lane and
    reports every lane's outcome instead of stopping at the first failure;
    sse_tests.rs joined node_harness.rs on the timers-erasure wall-clock allowlist as a
    cross-thread test-deadline owner;
    create_project_modules_tests' private SOURCE_READ_COUNTER_TEST_LOCK now delegates to the one
    facade-owned instrumentation lock, because two of its holders also open a collection session
    and a private lock left them racing the collector's other owners — that removes the
    CollectorBusy failure of synthetic_traversal_prepares_retained_clauses_without_a_token_rescan
    from the timers+benchmark_counters lane, leaving one genuine pre-existing failure there
  Phase 7: every weak integration contract in the canonical suite is gone. All 12 acceptance-only
    backend blocks (which were exactly the 12 smoke-role cases) were promoted to
    rendered_output_exact runtime contracts, each with a manifest contract and a non-smoke role,
    and each fixture's comment rewritten to state what the case now proves — the stale
    "Tests requirement 7.x" claims on the three borrow_checker_* cases are gone, and
    complex_borrowing_scenarios no longer claims field access it never exercised; the promoted
    cases moved to the suite's house pattern (one render function returning one named top-level
    fragment) except where a top-level binding is the subject; all 17 diagnostic_match = "contains"
    cases were re-measured under exact matching and every one passes, so the shared recovery reason
    ("current pre-canonical directory frontend compiles the imported child module as both a root
    candidate and a reachable dependency") is stale — contains-mode and its reason were removed
    from all 19 backend blocks and the redundant explicit exact declarations dropped with them;
    all 4 warnings = "ignore" blocks were re-measured too — path_escape_project_root_error and
    path_missing_error emit no warning at all and became forbid, external_import_constant_alias_success
    and direct_selection_external_import_alias_success each emit exactly one MOTH-IMPORT-0003 and
    became exact with that code; the suite now declares zero acceptance-only, zero contains-mode
    and zero warning-ignore contracts; schema 7 of the audit inventory adds the weak-contract review
    seam that keeps them findable if they return — summary counters smoke_role_cases,
    warning_ignore_backend_blocks, diagnostic_contains_backend_blocks and
    weak_contract_review_backend_blocks, plus a per-backend weak_contract_reviews list
    (acceptance_only_success / warnings_ignored / diagnostic_match_contains) with its own self-test,
    all of it review-only so a valid smoke case stays legal and hard policy keeps its single owner;
    the structured reason assertion no longer compares a "<none>" placeholder — a diagnostic with no
    reason key can satisfy no authored reason whatever its text, proved by a self-test that authors
    the rendered absent-reason wording as its reason; the rendered_output_exact mismatch report now
    escapes both sides post-normalization and names the first differing byte, because the previous
    report printed a whitespace-only mismatch as two identical lines; manifest ownership rechecked
    after the changes (0 hard policy findings, advisory set unchanged at 84, no new primary-less
    contract family); AUD-0001-F05 decided — both test_diagnostics.rs helpers are retired, not
    adopted, because the integration suite's diagnostic_assertions[].reason owns reason-key
    contracts (192 cases) and assert_exact_diagnostic_codes plus the suite's diagnostic_codes
    multiset own exact cardinality; each suppression now names that decision for Phase 11 to delete
NEXT_ACTION: clear ledger entry EF-0001 in Phase 10, and run Phase 8 (feature, platform and CI
  visibility)
VALIDATION: Phase 7 — cargo fmt --all --check clean; clippy --workspace --all-targets
  --all-features -D warnings clean; cargo test --workspace 4391+17+658, 0 failed, 0 ignored;
  timers lane 4391+17+658; detailed_timers lane 4393+17+658; docs check clean; bench-ci preflight;
  timers-erasure-check clean; integration 1850/1851 with the single failure being ledger entry
  EF-0001, reproduced identically at MOTH_TEST_THREADS=1 and MOTH_TEST_THREADS=8. `just validate`
  was not run to completion because its integration step stops on EF-0001; every gate it chains was
  run individually and is reported above.
  Phase 6 — just validate (pass: clippy --workspace --all-targets --all-features
  -D warnings clean; cargo test --workspace 4388+17+658, 0 failed, 0 ignored; integration
  1851/1851; docs check clean; bench-ci 60/60 preflight; timers-erasure-check clean).
  Phase 6 stress — just stress 1 (six lanes: unit and integration at 1, default and 16 threads,
  all pass).
  Phase 6 feature lanes — detailed_timers pass (4390+17+658); benchmark_counters and
  timers+benchmark_counters each fail only
  const_required_construction_preparation_is_reused_by_folding (a genuine pre-existing
  counter-behaviour defect), stable across three consecutive timers+benchmark_counters runs after
  the source-read lock fix. Phase 0 recorded four failures in that lane; two were observed here
  before the fix and one after, so the race-dependent members of that set need re-measuring when
  Phase 8 makes the lane an executed gate.
  Phase 5 closeout — just validate (pass: clippy --workspace --all-targets
  --all-features -D warnings clean; cargo test --workspace 4386+17+646, 0 failed, 0 ignored;
  integration 1851/1851; docs check clean; bench-ci preflight; timers-erasure-check clean).
  Phase 5 — just validate (pass: clippy --workspace --all-targets --all-features
  -D warnings clean; cargo test --workspace 4373+17+646, 0 failed, 0 ignored; integration
  1851/1851; docs check clean; bench-ci preflight; timers-erasure-check clean).
  Phase 4 closeout — just validate (pass: clippy --workspace --all-targets --all-features
  -D warnings clean; cargo test --workspace 4337+17+646, 0 failed, 0 ignored; integration
  1851/1851; docs check clean; bench-ci 60/60 preflight; timers-erasure-check clean).
  Phase 4 checkpoint — cargo fmt; cargo clippy --workspace --all-targets -D warnings (clean);
  cargo test --workspace (4329+17+646); cargo run -- tests --terse (1851/1851).
  Earlier phases: cargo fmt --check; cargo clippy -D warnings; cargo test --workspace (4314+17+643);
  cargo run -- tests --terse (1851/1851); cargo test --features timers (pass);
  cargo test --features detailed_timers (pass, 4316+17+643); just validate (pass);
  pre-existing benchmark_counters failure unchanged; Linux lane passed via GitHub Actions
  validate-linux (4324 unit tests incl. Linux-only non-UTF-8 filesystem identity tests,
  1851/1851 integration cases)
AUDITS: Phase 7 inventory of every acceptance-only backend block, smoke role,
  diagnostic_match = "contains" reason and warnings = "ignore" block in the canonical suite, each
  re-measured against the compiler rather than accepted from its authored justification; Phase 6
  sweep of every thread::sleep, yield_now, spin_loop, Instant::now,
  SystemTime::now and filesystem-timestamp read across src and xtask, with a disposition for each
  survivor (recorded under NOTES); pre-Phase-4 review of the Phase 0-3 work (helper contracts,
  panic-reason assertions, xtask absence assertions); Phase 4 sweep of >=, non-empty, any and find_map survivors across
  src and xtask with a disposition for each; Phase 4 closeout review (artifact identity must reuse
  the canonical output-path policy, typed rejection self-tests, anchored path predicates, helper
  ownership, just validate as the final gate); Phase 5 sweep of to_string_lossy/from_utf8_lossy and
  path unwrap_or_default across src and xtask, with every assertion-boundary use removed and the
  remaining report-rendering uses dispositioned; AUD-0001 (Redundancy over tests.support) —
  F01, F02 and F03 corrected on this branch, F04 routed to Phase 11, and F05 decided in Phase 7
  (both helpers retired, deletion left to Phase 11)
BLOCKERS: none for Phase 8. One exposed failure is open —
  docs/roadmap/plans/test-suite-honesty-exposed-failures.md entry EF-0001 (assigned top-level
  templates are counted as page fragments). Phase 10 owns the correction; the suite stays red until
  it lands, which is Patch A's intended state.
NOTES: Phase 7 exposed one defect, ledgered as EF-0001: a template head that opens the
  right-hand side of a top-level assignment is classified as a top-level runtime fragment, because
  top_level_classifier.rs maps TokenKind::TemplateHead to HeaderFileItem::RuntimeTemplate with no
  at_statement_boundary guard, unlike every other statement-level classification in that match. The
  emitted page therefore mounts one slot per assigned top-level template and inserts an empty
  fragment into each, contradicting entry-runtime-and-fragments.mtf ("Assigned or returned
  templates are not page fragments by themselves"). borrow_checker_string_memory reports it because
  a top-level mutable template buffer is that case's own subject; the other promoted cases use the
  house render-function pattern, so one owner reports the defect instead of eleven.
  Phase 7 carry-over for Phase 8: testing.mtf should record schema 7 of the suite inventory (the
  weak_contract_reviews field and the four new summary counters), and that the canonical suite now
  declares no acceptance-only, contains-mode or warning-ignore contract — all three remain legal and
  keep their parser, policy and reporting self-tests, but no fixture authors one.
  Phase 7 carry-over for Phase 11: three boundary cases now share
  language.bindings.implicit_shared_borrowing_acceptance (borrow_checker_basic_variables,
  borrow_checker_string_memory, immutable_alias_while_borrowed) beside its primary owner
  implicit_borrowing, and choice_dependency_visibility_exported restates
  choice_cross_file_runtime's shape. Both groups are candidates for the duplicate-test pruning in
  Phase 11 item 2; Phase 7 promoted rather than deleted them because deletion is that phase's call.
  Phase 6 timing-and-clock sweep dispositions. Remaining sleeps: node_harness workspace
  removal retry and exit poll (subprocess deadline owners, documented in Phase 5),
  dev_server/watch.rs poll interval (production polling backend, not a test) and the SSE
  registration wait (bounded deadline as deadlock protection, reports the observed client
  count — the handler registers after writing headers, so no in-process signal exists without
  changing production ordering). Remaining spin: runtime::wait_for_records, the production drain
  barrier bounded by the admitted-recorder count. Remaining wall-clock reads: check.rs, cli.rs,
  build_loop.rs, timing guard.rs, benchmarking/frontend.rs, integration runner.rs,
  xtask process_runner.rs and profile/runner.rs all measure the thing they report;
  dev_server/error_page.rs, xtask bench_system.rs, bench_time.rs and profile/artifacts.rs render
  timestamps or non-security ids; test_support::unused_temp_path mixes the clock into a name but
  proves non-existence with symlink_metadata, so uniqueness never rests on the clock; the
  Instant::now() values in timing and erasure tests are inputs to the API under test and the
  assertions compare exact recorded durations or monotonic ordering, not elapsed magnitudes.
  Remaining filesystem-timestamp reads: dev_server/watch.rs fingerprints are production change
  detection, and watch_tests sets modification times explicitly with fs::FileTimes rather than
  waiting for the clock to advance. xtask benchmark_execution success tests keep
  total_duration_ms > 0.0 as a narrow restatement of validate_total_duration, whose own test owns
  the zero, negative, NaN and infinite rejections.
  Pre-existing benchmark_counters feature test failures are not caused by this work.
  Inventory finding mappings fixed: lossy_path_text_conversion → Phase 5 item 7,
  source_text_tests_false_confidence → Phase 4 items 3 and 7.
  Phase 6 carry-over for Phase 8: testing.mtf should record the concurrency-test policy this
  phase established (condition variables or channels for expected transitions, a bounded
  wall-clock deadline only as deadlock protection, a deadline failure that names the observed
  state, and every spawned worker joined so its panic is the reported cause), and validation.mtf
  should list `just stress [repeats]` beside the thread and repetition gates.
  Phase 5 carry-over for Phase 8: testing.mtf's "Runtime output assertions" section should record
  the supported <script> shapes (including that `type="module"` is deliberately unsupported until
  the harness can materialize and execute the emitted module graph), the harness execution
  deadline, the harness failure classes and the golden expected-artifact-kind contract.
  The intermittent failure of frontend_benchmark_rejects_a_busy_raw_session_before_compilation
  that Phase 5 observed reproduced on the first Phase 6 baseline run and is now fixed. The cause
  was the test's evidence, not the compiler: xtask depends on moth with features=["timers"], so
  cargo test --workspace runs with the collector live, and the shared instrumentation lock
  serializes session owners but not the compiler work other concurrent tests record into the
  active process-global session — so the outer session's "every aggregate has zero samples"
  assertion depended on what else the suite happened to be running. Phase 6 replaced that proxy
  with typed ordering evidence that needs no process-global quiet, and left the collector
  process-global because cross-thread recording is its production contract (see
  parallel_timing_records_sum_into_one_atomic_slot).
  The four inventory findings Phase 5 owned (golden_comparison_accepting_directories_as_empty_files,
  html_wasm_baseline_broad_fragments, node_runtime_harness_can_hang, lossy_path_text_conversion)
  are now marked resolved in docs/roadmap/evidence/test_honesty_inventory.json.
  The three inventory findings Phase 6 owned (timing_concurrency_tests_machine_speed,
  filesystem_timestamp_tests, global_timing_collector_observed_flake) are now marked resolved in
  the same file.
  AUD-0001 test-support redundancy corrections landed alongside Phase 5 without changing any test
  outcome (4373+17+646 unchanged): one canonical test_source_location replaces 23 zero-value
  SourceLocation wrappers and 5 duplicated line-based builders; the seven HIR node constructors
  both backends shared moved to compiler_frontend::tests::hir_fixture_support, with each backend's
  build_type_environment and build_module documented as deliberately divergent; setup_builder,
  register_local and runtime_template_expression moved from hir_expression_lowering_tests.rs to
  hir_builder_test_support.rs and the three forwarding aliases were deleted. AUD-0001-F04 (Phase
  11) and AUD-0001-F05 (Phase 7, then Phase 11) stay open because both depend on decisions this
  plan owns.
```

## Purpose

The test suite must report what the compiler and its tooling actually do. A green test is harmful when it passed because fixture setup failed early, a different error happened, an IO failure was treated as absence, a path was changed through lossy conversion, a broad substring happened to match or a feature-gated branch never ran.

This plan hardens the complete test boundary so tests become reliable evidence. It deliberately separates two kinds of work:

1. **Make tests honest.** Strengthen fixtures, assertions, harnesses, feature coverage and validation reporting. This patch may turn previously green tests red.
2. **Correct exposed defects.** Fix compiler, build-system or test-fixture behavior revealed by the honest tests. This follow-up patch must clear the failure ledger before the roadmap continues.

The first patch must not weaken an assertion merely to preserve a green suite. A newly failing test is useful evidence when the stronger contract is correct.

## Sequencing and patch policy

This plan runs immediately after the TIR corrections and simplification plan. It blocks frontend module compilation ownership cleanup and every later item in the queued roadmap chain.

### Patch A: honesty and infrastructure

Patch A changes test infrastructure and test contracts. It may include narrow production seams needed to expose structured outcomes, such as a typed output-write result or an iterator over infrastructure diagnostics. It does not fix unrelated compiler behavior discovered by stronger tests.

Patch A may leave known failures. Every known failure must be recorded in the exposed-failure ledger before Patch A is considered reviewable. Do not hide a failure with:

- `#[ignore]`
- a platform exclusion that does not reflect the real contract
- `diagnostic_match = "contains"` without a genuine recovery reason
- a weaker count or substring assertion
- an acceptance-only success contract when semantic behavior is observable
- discarded IO, cleanup, join or restore errors
- a broad `is_err()` assertion in place of the intended reason

Patch A should remain a distinct commit or stacked review unit even when repository policy requires Patch B to land in the same pull request. This preserves an inspectable boundary between exposing defects and correcting them.

### Exposed-failure ledger

Create `docs/roadmap/plans/test-suite-honesty-exposed-failures.md` only when Patch A exposes failures. Keep it until Patch B clears the final entry. Phase 7 created it with entry `EF-0001`. Each entry records:

- stable entry ID
- test or integration case ID
- exact command
- operating system, architecture, enabled features and thread count
- intended contract
- previously masked condition
- newly observed result
- classification: false-positive test, fixture defect, harness defect or compiler/build defect
- correction owner and affected subsystem
- status and validating commit

The ledger is evidence, not an expected-failure mechanism. Tests remain failing until corrected. Do not teach the runner to accept ledger entries.

### Patch B: exposed-defect corrections

Patch B fixes every failure exposed by Patch A. It may be split by subsystem, but all slices remain part of this roadmap item. No later roadmap plan starts while the ledger contains an open entry.

Patch B completion requires:

- an empty exposed-failure ledger
- no new ignore or expectation weakening
- all hardened assertions retained
- the complete validation and stress matrix in this plan passing

## Definition of an honest test

An honest test has all of these properties where they apply:

1. **The name matches the contract.** A smoke test is named as a smoke test. A semantic test proves the named semantic result.
2. **Fixture setup is observable.** Required directories, files, links, permissions and process dependencies are created successfully before the subject runs.
3. **Failure identity is exact.** A multi-lane operation proves the expected diagnostic kind, reason, infrastructure type or typed error variant. Any `Err` is not enough.
4. **Success identity is exact.** A positive test proves the target function, artifact, path, output or side-table fact, not merely that some output or fact exists.
5. **Absence is an IO result.** Only `NotFound` means absent. Permission errors, dangling links and metadata failures are not absence.
6. **Path handling is lossless.** Tests keep `Path` and `OsStr` values until a boundary that explicitly requires UTF-8. Unsupported non-UTF-8 input fails clearly.
7. **Cardinality is explicit.** Tests that expect one artifact, diagnostic, warning, import, export or function assert exactly one.
8. **Ordering is explicit.** Tests either prove order or deliberately compare unordered multisets. Incidental container order is never accepted silently.
9. **Time is not synchronization.** Tests use barriers, channels or observable write outcomes instead of arbitrary sleeps, spin counts or timestamp granularity.
10. **Global state is restored.** Current directory, environment variables, collectors, counters and process-global registries have one serialized owner and surface restore failures.
11. **Harness failures stay separate.** Fixture, Node, filesystem, report-writing and process failures cannot satisfy compiler success or failure expectations.
12. **Feature and platform branches execute.** Every maintained `cfg(feature)`, Unix and Windows test branch has an owned validation lane.
13. **Test doubles preserve production invariants.** Synthetic results and diagnostics use valid identities and states unless the test explicitly targets malformed internal data.
14. **Cleanup is checked when it is part of isolation.** A leaked file or directory cannot silently affect a later test.
15. **Ignored coverage is governed.** Every ignored test has an owner, reason and removal condition. New failures from this plan are never ignored.

## Scope

This plan audits and hardens:

- Rust unit and subsystem tests under `src/**/tests`, `src/compiler_tests` and `xtask/src/**/tests`
- canonical integration cases under `tests/cases`
- integration fixture loading, assertion evaluation, runtime execution and reporting
- build output, manifest and cleanup tests
- benchmark and timing tests
- test-only helpers and synthetic builders
- process-global test state
- platform and feature matrices
- `justfile` validation recipes and GitHub Actions reporting
- testing and validation style-guide policy

It does not redesign language semantics, TIR, the compiler diagnostic taxonomy or build output policy except where a narrow typed seam is needed to inspect an existing result honestly.

## Confirmed fragility findings

The following findings are confirmed seed work at the base revision. Phase 0 must recheck them after TIR completes because files and test names may move.

### 1. The shared `temp_dir` helper returns something that is not a directory

`src/compiler_tests/test_support.rs::temp_dir` returns an unmanaged, uncreated path. Its name suggests a ready directory, while callers must remember to create it and clean it manually. The helper uses wall-clock nanoseconds, process ID and an atomic sequence, then callers commonly use `create_dir_all`. A stale path from an interrupted run can be reused without setup noticing because `create_dir_all` accepts an existing directory.

This is the original class of failure that motivated this plan. The two broken non-UTF-8 tests attempted to create files below the returned path without first creating the parent. The compiler then failed for fixture IO rather than the intended condition.

Required direction:

- migrate tests that need a directory to `tempfile::tempdir()` or `tempfile::Builder`
- use `TempDir::close()` where cleanup success is part of isolation
- retain an uncreated path helper only where nonexistence is the contract
- name that helper `unused_temp_path`
- delete `temp_dir` after migration

### 2. Manual temporary paths can inherit stale state

Confirmed examples include:

- `src/build_system/output/tests/writer_tests.rs::hard_link_inspection_fails_closed_when_a_file_cannot_be_opened` uses a path based only on process ID, calls `create_dir_all` and assumes `missing-output` is absent
- `src/build_system/tests/build_cleanup_tests.rs::stale_cleanup_preserves_non_regular_nodes` constructs a path directly under `/tmp` from process ID and wall-clock time
- the integration rendered-output harness creates files and directories under `std::env::temp_dir()` from process ID, wall-clock time and a counter

These tests own cleanup manually. Interrupted runs, PID reuse and cleanup failure can change later test behavior.

### 3. `exists`, `is_file` and `is_dir` can turn IO failure into absence

`Path::exists`, `Path::is_file` and `Path::is_dir` return false when metadata cannot be read. `exists` also follows links, so a dangling symlink looks absent. This can let a test claim that no output was written when an unexpected link or inaccessible node still exists.

Confirmed high-risk areas include:

- output preflight and no-mutation assertions in `src/build_system/tests/build_orchestration_tests.rs`
- manifest cleanup and retained-file assertions in `src/build_system/tests/build_cleanup_tests.rs`
- fixture discovery in `src/compiler_tests/integration_test_runner/fixture.rs`
- output and manifest absence checks after expected writer failures

Required direction:

```rust
#[track_caller]
fn assert_path_missing(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(metadata) => panic!("expected no filesystem node at {path:?}, found {metadata:?}"),
        Err(error) => panic!("failed to inspect {path:?}: {error}"),
    }
}
```

Add similarly explicit helpers for regular files, directories and symlinks. The helper name must state whether it follows links.

### 4. Broad `is_err()` assertions accept the wrong failure

A bare `is_err()` is valid only when the called API has one meaningful error state. Compiler, build, fixture and filesystem boundaries have several.

Confirmed fragile examples include:

- `single_file_rejects_missing_file` checks only `result.is_err()` and `error_count() > 0`
- `failed_directory_preparation_keeps_unfinished_module_metadata_out_of_completion` accepts any preparation error before inspecting timing metadata
- output writer tests for invalid paths, symlink aliases, canonical aliases, hard links and file/directory collisions often accept any `CompilerMessages` error
- frontend benchmark tests for a missing file and invalid syntax accept any error
- several integration-runner option and policy tests check only `is_err()` and that a callback was not called
- `current_dir_guard_recovers_after_mutex_poisoning` accepts any panic from the poison setup closure

Required direction:

- classify each `is_err`, `is_ok`, `expect_err`, `catch_unwind` and panic assertion by API error width
- assert a stable typed reason for multi-lane boundaries
- keep side-effect assertions, but do not use them as a substitute for failure identity
- do not mechanically ban `is_err()` for narrow enums and single-error invariants

### 5. Infrastructure error inspection can miss later infrastructure errors

`CompilerMessages::first_infrastructure_error_for_tests()` first selects the first error diagnostic, then returns `None` when that first error is not infrastructure. A later infrastructure error is therefore invisible to the helper.

Add an iterator that scans all error diagnostics:

```rust
#[cfg(test)]
pub(crate) fn infrastructure_errors_for_tests(
    &self,
) -> impl Iterator<Item = (&ErrorType, &str, &SourceLocation)> {
    self.error_diagnostics().filter_map(|diagnostic| {
        let DiagnosticPayload::InfrastructureError {
            msg,
            error_type,
            ..
        } = &diagnostic.payload
        else {
            return None;
        };

        Some((error_type, msg.as_str(), &diagnostic.primary_location))
    })
}
```

Make the existing first helper delegate to this iterator. Add assertion helpers for no infrastructure error, exactly one infrastructure error and exact authored diagnostic multisets.

### 6. Synthetic test data can violate production identity rules

Confirmed examples include:

- `WarningBuilder` in `src/build_system/tests/mod.rs` interns the warning name in a fresh `StringTable`, then returns it beside the caller's different table
- integration runner tests construct a `CaseExecutionResult` with `passed: true` and no `build_result`
- another synthetic failed result has no `failure_kind`

These shapes can make an orchestration test green while the synthetic value could not have been produced by the real pipeline.

Required direction:

- add test constructors that enforce valid success and failure states
- create synthetic diagnostics against the active `StringTable`
- audit raw literals for production-invariant result and identity types
- reserve malformed literals for tests whose name and assertion explicitly target malformed state rejection

### 7. Current-directory restoration failures are discarded

`CurrentDirGuard::Drop` uses `let _ = std::env::set_current_dir(&self.previous)`. A restore failure can leave the whole process in the wrong directory and make unrelated later tests fail or pass against the wrong files.

Required direction:

- give current-directory mutation one scoped owner
- expose an explicit `finish` or closure API that reports restoration failure on the normal path
- during unwinding, preserve the original panic and record restoration failure without replacing it
- add a focused restore-failure test using an intentional seam rather than damaging the real working directory

Apply the same audit to environment variables, global timing sessions, counters and registries.

### 8. Lossy path and text conversion hides unsupported input

Confirmed examples include:

- several module-discovery test helpers use `file_name().and_then(OsStr::to_str).unwrap_or_default()`, converting non-UTF-8 names to an empty string
- integration execution converts the entry path with `to_string_lossy()` before calling `build_project`
- the rendered-output harness converts paths and process output lossily
- golden comparison uses `String::from_utf8_lossy` for expected text files
- `MOTH_TEST_THREADS` parses an `OsString` through `to_string_lossy`

Required direction:

- keep path values as `Path` and `OsStr` until a real UTF-8 boundary
- reject non-UTF-8 values explicitly when the target protocol requires UTF-8
- never use `unwrap_or_default` for an asserted path component
- treat invalid UTF-8 text goldens and harness output as harness failures
- add platform-owned non-UTF-8 tests where the host API supports constructing them

### 9. Several positive tests prove only that something exists

Confirmed examples include:

- `build_single_file_project_includes_reachable_dependency_files` asserts only that the output list is non-empty
- frontend pipeline smoke tests use `functions_analyzed >= 1` or `>= 2` and require only some statement or value facts
- JS glue tests select the first HTML output and the first path containing `_moth/js/glue/`
- runtime-module tests use `any` over output paths without exact count
- a source-package benchmark warning test accepts at least one warning and any occurrence of one code

Required direction:

- look up exact normalized artifact paths and assert uniqueness
- assert exact function origins and expected analysed count when count is contractual
- assert the target side-table relationship rather than any non-empty table
- compare exact warning multisets
- split independent stage contracts into separate tests so an early failure does not suppress later coverage

### 10. Artifact lookup silently accepts duplicate paths

`find_output_file` in the integration assertion owner returns the first matching artifact. `collect_built_artifact_paths` sorts paths but does not reject duplicates. A backend could emit two outputs with the same normalized path and every later assertion would inspect only one.

Required direction:

Build one typed artifact index before any success assertion:

```rust
struct BuiltArtifactIndex<'a> {
    by_path: BTreeMap<String, &'a OutputFile>,
}
```

Construction fails on duplicate normalized paths, case/spelling aliases where policy requires portability and impossible built kinds. Every artifact, golden, baseline and runtime assertion consumes this index.

### 11. Golden comparison can accept a directory as an empty file

`validate_golden_outputs` currently converts `FileKind::Directory` and `FileKind::NotBuilt` to an empty byte vector. If the expected golden is an empty file and the produced path is a directory, the byte comparison can pass.

The same owner reads expected text with lossy UTF-8 conversion.

Required direction:

- a golden file requires a file artifact of the expected kind
- directories and `NotBuilt` are always a kind mismatch
- text goldens require valid UTF-8
- binary and Wasm goldens compare bytes only
- add explicit regression tests for empty file versus directory, `NotBuilt`, invalid UTF-8 and duplicate output paths

### 12. HTML and Wasm baselines rely on broad fragments

The HTML baseline checks that required fragments appear somewhere. It does not prove ordering, uniqueness or that fragments are structural rather than text in a comment or script.

The HTML-Wasm baseline checks JavaScript source fragments such as `instance.exports.moth_start()` but does not require the Wasm module to export `moth_start`. Import and export checks compare names only, not kinds or multiplicity.

Required direction:

- either strengthen the HTML baseline to an ordered, bounded structural contract or rename it so it does not overclaim document validity
- require every runtime-called Wasm export, including `moth_start`, with the expected export kind
- reject duplicate exports and imports when uniqueness is contractual
- prefer executing runtime behavior when runtime behavior is the real owner
- keep source-fragment assertions only for narrow code-shape contracts

### 13. The Node runtime harness can hang the whole suite

The rendered-output harness launches Node with `Command::output()` and no per-process timeout. Generated code containing an infinite loop can block until the outer CI job times out.

It also:

- creates manual temporary files and directories
- discards several cleanup errors
- decodes stdout and stderr lossily
- converts paths lossily
- extracts `<script>` blocks with a case-sensitive substring scanner
- executes every non-empty inline script regardless of `type`
- ignores external `src` script semantics
- stops quietly on malformed closing tags

Required direction:

- use an owned temporary directory
- launch Node with a bounded timeout, then kill and wait for the process
- classify timeout, spawn, exit, output decoding and cleanup as distinct harness failures
- parse only the documented supported script shapes and reject unsupported shapes
- add self-tests for timeout, malformed tags, script types, external scripts, invalid UTF-8, stdout noise and cleanup failure

### 14. Fixture discovery can fail open

The integration fixture loader uses `is_file`, `is_dir` and similar predicates for required fixture state. Metadata and permission failures can be reported as ordinary missing files or can cause entries to be skipped.

Directory entries with non-UTF-8 names are currently skipped in canonical fixture discovery. Fallback names such as `unknown_case` and `unnamed_case` can hide the identity that failed.

Many fixture self-tests assert string fragments from `String` errors rather than a typed failure.

Required direction:

- add a typed `FixtureLoadError` and stable reason enum
- distinguish not found, wrong kind, symlink, escape, invalid UTF-8, unreadable directory and parse failure
- make every discovered non-UTF-8 or unsupported entry a hard finding
- remove identity fallbacks for canonical fixture paths
- assert exact variants and paths in self-tests, then test rendering separately

### 15. Timing concurrency tests depend on machine speed

`wait_for_timing_flag` loops up to 1,000,000 times and calls `yield_now`. This is a CPU-speed-dependent timeout, not deterministic synchronization. Failure cleanup also discards some thread join results.

Required direction:

- use barriers, channels or condition variables for expected transitions
- use a bounded wall-clock deadline only as deadlock protection
- include observed state in deadline failures
- always join every spawned thread and surface its panic
- remove arbitrary spin counts and sleeps from deterministic concurrency tests

### 16. Feature-gated tests are compiled by Clippy but not executed by validation

The default workspace test command enables no Cargo features. Timing tests behind `feature = "timers"`, combined timing/counter behavior and other feature-only branches are therefore not executed by `just validate`. `--all-features` Clippy type-checks them but does not prove runtime behavior.

Required direction:

Add a curated execution matrix that covers at least:

- default features
- `timers`
- `benchmark_counters` without timers
- `timers,benchmark_counters`
- `detailed_timers`
- all features for broad interaction coverage

Avoid repeating the full slow suite where a package- or module-owned command gives the same branch coverage. The audit report must map every feature-gated test module to an executed lane.

### 17. Filesystem timestamp tests can pass after an unwanted rewrite

`skip_unchanged_mode_preserves_existing_output_mtime` sleeps for 30 ms and compares modification times. Filesystems with coarser timestamp resolution can report the same timestamp even when the file was rewritten. The related stale-cleanup test uses the same pattern.

Required direction:

- make the writer expose a typed `Written`, `SkippedUnchanged` or equivalent outcome
- assert that outcome directly
- keep metadata checks only as secondary evidence
- remove the sleep
- audit all uses of `thread::sleep`, `yield_now`, wall-clock uniqueness and modification times

### 18. Monolithic CI steps hide later failures

Each operating-system job currently runs one sequential `just validate`. Clippy failure prevents unit tests, integration tests, documentation checks, benchmark sanity and timer erasure from running in that job. Windows is marked non-blocking and deployment depends only on Linux and macOS.

The validation style guide also describes an older cross-target Clippy structure that no longer matches the current `justfile` and per-OS workflow.

Required direction:

- keep local `just validate` fail-fast
- make CI report validation families independently with separate steps or jobs
- use `if: ${{ !cancelled() }}` where one failed gate should not suppress independent evidence
- add the feature execution matrix as an explicit lane
- document the Windows non-blocking policy and its exit condition
- update `validation.mtf` to match executable reality

### 19. Acceptance-only smoke cases can overclaim through names and comments

Confirmed examples include:

- `borrow_checker_basic_variables` is an acceptance-only smoke, while the source comments claim multiple borrow-checker requirements
- `borrow_checker_function_calls` similarly claims a numbered borrow-checker requirement while serving as a smoke case
- `choice_basic_declaration_and_use` exercises a named semantic behavior but has only an acceptance-only contract

Smoke coverage is valid. The problem is a semantic-looking name or comment that implies stronger evidence than the expectation provides.

Required direction:

Audit every acceptance-only backend block and smoke role:

- keep it as smoke and rename or rewrite comments to match that purpose
- or give it a contract and the strongest observable semantic assertion
- or move the semantic claim to a focused unit or integration owner and leave a clearly named smoke case

Do not ban acceptance-only cases. Make their claimed evidence honest.

### 20. Ignored tests need owned governance

The active TIR plan currently records ignored regressions. Phase 0 must rebaseline all `#[ignore]` uses after TIR completes.

Every remaining ignored test must have:

- owning plan or issue
- exact reason
- command to reproduce
- removal condition
- no broader ignored scope than necessary

The honesty and correction patches may not add ignores for newly exposed failures. The audit command should reject unowned ignores.

### 21. Source-text tests can create false confidence

Confirmed source-inspection examples include:

- an integration assertion self-test that `include_str!`s `diagnostics.rs` and checks that a removed conversion name is absent
- timing erasure tests that search source text for a `cfg!` spelling

Source checks can miss aliases, formatting changes and equivalent code. They can also fail on comments.

Required direction:

- behavior claims use behavior tests
- architecture bans with broad source scope move to one owned `xtask` audit with structured findings
- no source-text test is the sole evidence for runtime or semantic behavior

### 22. Machine-readable reports can be stale or partial

Integration reports are written directly to their final path after execution. A killed process can leave a prior successful report in place or a partial new file. Repository commit discovery silently returns `None` on any Git failure.

Required direction:

- remove or mark the previous report incomplete at run start
- write reports to a sibling temporary file, flush, then rename atomically
- include run ID, command, OS, architecture, features, thread count and completion state
- distinguish an unknown repository revision from a cleanly discovered `None`
- add report interruption and write-failure self-tests

## Seed fragility inventory

Phase 0 must turn this table into a machine-readable inventory and recheck every row against the post-TIR tree.

| Area | Confirmed example | Current weakness | Required proof |
| --- | --- | --- | --- |
| Temp support | `compiler_tests::test_support::temp_dir` | Name implies a created directory but returns an unmanaged unused path | Created directory type owns cleanup, unused path has an explicit name |
| Windows writer test | `hard_link_inspection_fails_closed_when_a_file_cannot_be_opened` | PID-only stale directory and any error | Fresh owned directory and exact hard-link inspection error |
| Unix cleanup test | `stale_cleanup_preserves_non_regular_nodes` | Hardcoded `/tmp` and manual cleanup | Host temp root, owned cleanup and exact special-node preservation |
| Frontend missing file | `single_file_rejects_missing_file` | Any error and nonzero count | Exact File infrastructure error and intended path |
| Stage 0 timing | `failed_directory_preparation_keeps_unfinished_module_metadata_out_of_completion` | Any earlier error can satisfy setup | Exact malformed Stage 0 diagnostic plus timing invariant |
| Writer invalid paths | `write_project_outputs_rejects_invalid_paths` | One loop accepts any error for several reasons | Case table with exact reason per path |
| Writer aliases | canonical case and symlink alias rejection tests | Any File error plus partial no-write check | Exact alias/collision reason and complete no-mutation snapshot |
| Writer hard links | `hard_linked_outputs_are_rejected_before_emission` | Any error for four distinct cases | Exact hard-link relation reason per case |
| Writer file/directory collision | `file_output_to_existing_directory_is_rejected_before_emission` | Any error | Exact destination-kind conflict and no writes |
| Path absence | many `!path.exists()` assertions | Permission errors and dangling links look absent | `symlink_metadata` accepts only `NotFound` |
| Manifest cleanup | stale output removal tests | `exists` and `is_dir` hide metadata errors | Exact node kind or exact absence helper |
| CWD guard | `CurrentDirGuard::drop` | Restore error discarded | Explicit restore result and isolated unwind behavior |
| CWD poison test | `current_dir_guard_recovers_after_mutex_poisoning` | Any panic poisons lock | Exact intentional panic payload and post-poison recovery |
| Synthetic warning | `WarningBuilder` | Name ID belongs to a different string table | Diagnostic built from caller table and rendered assertion |
| Synthetic runner success | `successful_execution_result` | Impossible `passed` success with no build result | Typed constructor enforces valid success state |
| Benchmark missing file | `frontend_benchmark_fails_for_missing_file` | Hardcoded absolute path and any error | Fresh known-missing path and exact benchmark error source |
| Benchmark syntax | `frontend_benchmark_fails_for_invalid_syntax` | Any error | Exact compiler diagnostic set through benchmark boundary |
| Benchmark warnings | source-package warning test | `>= 1` and `any` code | Exact count and multiset |
| Frontend pipeline | single and multi-file borrow smoke tests | Minimum counts and non-empty tables | Exact function origins and target facts |
| Dependency build | reachable dependency test | Only non-empty output | Exact expected artifact or linked function behavior |
| JS glue | generated glue tests | First matching output and broad fragments | Exact one glue path, exact imports and expected body contract |
| Runtime assets | JS runtime module tests | `any` path match | Exact path count and presence/absence set |
| Artifact lookup | `find_output_file` | First duplicate wins | Typed unique artifact index |
| HTML baseline | required fragment loop | Comments or wrong order can satisfy | Ordered structural contract or narrower naming |
| Wasm baseline | source fragments and export names | Called `moth_start` not required, kinds unchecked | Exact required export names, kinds and multiplicity |
| Golden comparison | directory and `NotBuilt` map to empty bytes | Empty golden can match wrong kind | Exact artifact kind before bytes |
| Golden text | `String::from_utf8_lossy` | Invalid UTF-8 is silently changed | Strict UTF-8 harness failure |
| Fixture discovery | `is_file` and `is_dir` | IO errors look missing | Typed metadata error reasons |
| Fixture names | non-UTF-8 entries skipped | Cases can disappear from inventory | Hard finding with original path bytes where supported |
| Fixture self-tests | `error.contains(...)` | Wrong failure with similar prose can pass | Exact typed variant and path |
| Entry execution | `entry_path.to_string_lossy()` | Compiler receives a changed path | Path-preserving API or explicit UTF-8 rejection |
| Node temp state | rendered-output temp files | Manual cleanup and dropped errors | Owned temp root and checked close |
| Node process | `Command::output()` | Infinite loop hangs suite | Per-process timeout, kill and wait |
| Script extraction | substring scanner | Diverges from supported browser script behavior | Supported-shape parser with rejection tests |
| Timing synchronization | `wait_for_timing_flag` | Yield-count timeout depends on machine speed | Channel/barrier protocol with bounded deadlock deadline |
| Timing thread cleanup | failure paths discard joins | Worker panic is hidden | Every thread joined and panic surfaced |
| Skip unchanged | mtime tests with 30 ms sleep | Coarse clocks can false-pass | Typed write outcome, no sleep |
| Feature tests | default `cargo test` only | Feature-gated tests never execute | Curated feature execution matrix |
| CI | one `just validate` step per OS | First failure masks later gates | Independent gate reporting |
| Windows policy | `continue-on-error: true` | Windows-only regressions do not block | Explicit temporary policy and exit condition |
| Smoke cases | acceptance-only semantic names/comments | Case claims more than it proves | Rename as smoke or add semantic contract |
| Ignored tests | post-TIR ignored inventory | Hidden failures can accumulate | Owned ignore metadata and audit hard finding |
| Source checks | `include_str!` plus `contains` | Text spelling is weak evidence | Behavior test or one structured architecture audit |
| Reports | direct final-path write | Stale or partial JSON can look current | Atomic write with completion metadata |

## Target test infrastructure

### Owned temporary workspaces

Use `tempfile::TempDir` as the default filesystem fixture. Add local fixture builders only where they encode a real repeated shape, such as a canonical integration case or output root. Builders must expose the root path and keep the `TempDir` owner alive.

Do not create one broad test DSL that hides source shape. The testing style guide still applies: important Moth inputs remain visible in the owning test.

### Explicit filesystem assertions

Add a small shared `test_fs` owner because path state is checked across build, integration and benchmark suites. Keep the API narrow:

```rust
assert_path_missing(path)
assert_regular_file(path)
assert_directory(path)
assert_symlink(path)
read_bytes(path)
read_utf8(path)
```

Every helper uses `#[track_caller]` and includes the path and underlying IO error in failure output. Do not add generic assertion combinators.

### Typed expected-failure assertions

Add focused helpers near the diagnostic owner:

```rust
assert_exact_diagnostic_codes(messages, expected)
assert_no_infrastructure_errors(messages)
assert_exact_infrastructure_error(messages, expected_type)
assert_diagnostic_reason(messages, code, occurrence, reason)
```

Output, fixture, benchmark and harness owners should expose typed reason enums rather than forcing tests to match rendered prose. Rendering tests remain separate.

### Typed artifact inventory

Build one normalized artifact index for each build result. It owns:

- exact normalized path identity
- artifact kind
- uniqueness
- lookup
- ordered path reporting

All success baselines, goldens, absence checks, runtime assertions and Wasm checks use it.

### Deterministic process and concurrency support

Add one process runner for test harness subprocesses with:

- timeout
- kill and wait
- exact exit status
- bounded stdout and stderr capture
- strict output decoding where text is required
- temporary workspace ownership

Use channels, barriers and condition variables for in-process concurrency tests. Test-only clocks or typed outcomes are preferred over sleeping for filesystem changes.

### Test honesty audit command

Add an owned `xtask` command and `just test-honesty-audit` recipe. It produces:

```text
target/test-reports/test_honesty_inventory.json
```

The command classifies findings rather than blindly rejecting syntax.

Hard findings should include:

- use of the retired `temp_dir` helper
- hardcoded `/tmp` in tests
- discarded current-directory or environment restoration
- discarded thread joins
- discarded required cleanup in harness owners
- unowned `#[ignore]`
- feature-gated test modules with no execution lane
- integration fixture entries skipped for unsupported identity
- direct final-path report writes where atomic reporting is required

Review findings should include:

- broad `is_err`, `is_ok`, `catch_unwind` and `should_panic`
- `exists`, `is_file` and `is_dir`
- `to_string_lossy`, `from_utf8_lossy` and path `unwrap_or_default`
- `>=`, non-empty and `any` assertions
- first-match `find_map` artifact selection
- broad `contains` checks
- sleeps, spin loops, wall-clock uniqueness and mtime comparisons
- source-text assertions

Every review finding gets a disposition in the report:

- hardened
- narrow API with one valid outcome
- intentionally smoke-level
- rendered prose is the contract
- platform-specific by real API ownership
- moved to structured audit

The audit must not regex-ban all uses of `is_err`, `contains` or `any`.

## Required repository search pass

Phase 0 and Phase 8 must run these searches, then inspect each result in context:

```bash
rg '\.is_err\(\)|\.is_ok\(\)|expect_err\(|catch_unwind|should_panic' src xtask
rg 'error_count\(\).*?>|warning_count.*?>=|functions_analyzed.*?>=|is_empty\(\)' src xtask
rg '\.exists\(\)|\.is_file\(\)|\.is_dir\(\)' src xtask
rg 'to_string_lossy|from_utf8_lossy|unwrap_or_default\(\)' src xtask
rg 'std::env::temp_dir|SystemTime::now|process::id' src xtask
rg 'let _ = .*remove_|let _ = .*set_current_dir|let _ = .*join|\.ok\(\)\?' src xtask
rg 'set_current_dir|set_var|remove_var' src xtask
rg 'thread::sleep|yield_now|Instant::now|modified\(\)' src xtask
rg 'find_map|\.any\(' src xtask
rg 'contains\(' src/compiler_tests src/build_system xtask/src
rg '#\[ignore|#\[cfg\(.*(feature|unix|windows)' src xtask
rg 'read_dir|symlink_metadata|metadata\(' src/compiler_tests src/build_system xtask/src
rg '/tmp/|/definitely/does/not/exist|C:\\' src xtask
rg 'include_str!' src xtask
```

Also inspect:

- unordered collection iteration used in exact snapshots
- tests that combine independent contracts in one function
- test names containing `exact`, `all`, `only`, `never`, `preserves`, `rejects` or `before` without matching cardinality or stage assertions
- production `cfg` branches that have no corresponding executed test lane
- helpers used by only one or two tests that hide fixture setup
- broad mocks that bypass the real pipeline boundary

## Implementation phases

### Phase 0: Post-TIR baseline and complete inventory

1. Rebase this plan's seed paths and test names onto the completed TIR tree.
2. Record exact baseline commands, pass counts, ignored counts, feature sets and platform results.
3. Inventory every temporary path helper, filesystem predicate, expected-failure assertion, lossy conversion, sleep, global-state mutation, ignored test and feature-gated test module.
4. Run the canonical integration suite audit and classify every acceptance-only, contains-mode and warning-ignore contract.
5. Create the first `test_honesty_inventory.json` schema, even if generation is initially a one-off script.
6. Update the plan capsule with counts and confirmed owners before code changes start.

Exit criteria:

- every seed row is confirmed, moved or closed with evidence
- all ignored tests have an owner or are marked for removal
- every feature-gated test module is mapped to a proposed lane
- Patch A scope is fixed

### Phase 1: Test filesystem ownership

1. Add narrow explicit filesystem assertion helpers.
2. Migrate created directory fixtures to `tempfile`.
3. Rename any deliberate uncreated path helper to `unused_temp_path`.
4. Remove hardcoded `/tmp` and fabricated global missing paths.
5. Replace ambiguous `exists`, `is_file` and `is_dir` assertions at sensitive boundaries.
6. Make fixture discovery distinguish absence from IO failure.
7. Make cleanup and `TempDir::close` failures visible where isolation depends on removal.
8. Delete the old `temp_dir` helper.

Add regressions for:

- missing parent setup
- stale pre-existing temp state
- dangling symlink versus absence
- unreadable metadata where the host permits it
- wrong filesystem node kind
- cleanup failure reporting

### Phase 2: Structured failure identity

1. Add the complete infrastructure-error iterator.
2. Add exact diagnostic and infrastructure assertion helpers.
3. Introduce typed reason seams for output preparation, fixture loading, benchmark boundaries and harness failures where current `String` transport prevents exact tests.
4. Audit all multi-lane `is_err`, `expect_err`, panic and catch-unwind tests.
5. Preserve no-mutation and side-effect checks beside exact reasons.
6. Split tests that currently combine independent failure boundaries.
7. Add tests proving an unrelated infrastructure error cannot satisfy an authored diagnostic expectation.

### Phase 3: Synthetic state and global process state

1. Replace invalid synthetic `CaseExecutionResult` and diagnostic literals with checked constructors.
2. Make every synthetic `StringId` use the active table.
3. Audit test doubles for missing interface, origin, warning and artifact invariants.
4. Replace current-directory mutation with a checked scoped owner.
5. Audit environment-variable mutation and process-global collectors.
6. Ensure lock poisoning is intentional and exact.
7. Ensure every spawned test thread is joined on success and failure.

### Phase 4: Exact positive assertions and artifact inventory

1. Build the typed unique artifact index.
2. Replace first-match artifact discovery with exact path lookup.
3. Strengthen reachable dependency, JS glue, runtime asset and frontend pipeline tests.
4. Use exact counts and multisets for warnings, functions, imports, exports and output paths where contractual.
5. Split smoke coverage from semantic ownership.
6. Add duplicate artifact and alias regressions.
7. Review every `>=`, non-empty, `any` and `find_map` result from the inventory.

### Phase 5: Golden, HTML, Wasm and runtime harness hardening

1. Reject golden artifact kind mismatches before comparing content.
2. Require strict UTF-8 for text goldens.
3. Strengthen or accurately rename HTML baseline checks.
4. Require the complete HTML-Wasm runtime export contract with kinds and cardinality.
5. Move the Node harness to owned temporary workspaces.
6. Add subprocess timeout, kill and wait.
7. Replace lossy process output and path conversion.
8. Replace permissive script extraction with a supported-shape parser.
9. Make harness cleanup failures visible.
10. Add focused self-tests for every harness failure class.

### Phase 6: Deterministic timing and concurrency tests

1. Replace yield-count synchronization with channels, barriers or condition variables.
2. Remove filesystem mtime sleeps by exposing typed write outcomes.
3. Audit wall-clock uniqueness and timing assertions.
4. Replace `total_ms > 0` tests with sample or outcome evidence that does not depend on clock resolution.
5. Add repeat and thread-count stress commands for timing, output and integration owners.
6. Surface all worker panic and timeout state in failures.

### Phase 7: Integration contract honesty

1. Inventory all smoke and acceptance-only backend blocks.
2. Compare case IDs, source comments, role and contract strength.
3. Rename or rewrite cases that overclaim.
4. Promote semantic smoke cases to exact runtime, artifact, diagnostic or internal assertions where the behavior is observable.
5. Audit `diagnostic_match = "contains"` reasons and warning-ignore usage.
6. Add audit fields for weak-contract review without making valid smoke cases illegal.
7. Recheck manifest ownership and primary contract coverage after changes.
8. Decide the adopt-or-retire question for `assert_diagnostic_reason` and `error_code_counts`
   in `src/compiler_tests/test_diagnostics.rs` (AUD-0001-F05). Both were added by Phase 2 and
   still have zero callers and no self-tests. Adopt them only where a case genuinely owns a
   reason-key or exact code-multiset contract. Never add a token caller to retire the lint —
   that is implementation-shaped coverage. If nothing adopts them here, they carry a justified
   suppression naming this decision until Phase 11 deletes them.

### Phase 8: Feature, platform and CI visibility

1. Add `just test-feature-matrix` with the curated feature lanes.
2. Map every feature-gated test module to one executed command.
3. Run Unix and Windows path, link and non-UTF-8 owners on the platforms that support them.
4. Split CI validation into independently reported gates while keeping the local fail-fast recipe.
5. Add the honesty audit and machine-readable report artifacts to CI.
6. Document Windows non-blocking status and define when it becomes blocking.
7. Update `testing.mtf` and `validation.mtf` to match the new commands and policies.

### Phase 9: Patch A honesty checkpoint

1. Run the complete validation, feature, platform, thread and repeat matrix.
2. Do not repair newly exposed compiler behavior in this checkpoint.
3. Record every newly failing contract in the exposed-failure ledger.
4. Review each failure to confirm the stronger assertion is valid.
5. Revert only an incorrect hardening assumption, never a correct assertion that reveals a defect.
6. Commit Patch A as a distinct review unit.

Patch A exit criteria:

- test infrastructure changes are complete
- all hard audit findings are gone
- every review finding has a disposition
- all failures are reproducible and ledgered
- no failure is ignored or accepted

### Phase 10: Patch B exposed-defect corrections

For each ledger entry:

1. reproduce with the recorded narrow command
2. identify whether the defect is compiler, build system, fixture or harness behavior
3. add any missing lower-level regression needed to localize ownership
4. fix the owning subsystem without weakening Patch A
5. run the narrow command on all relevant feature and platform lanes
6. mark the ledger entry closed with the validating commit

Delete the ledger file when its final entry closes, or retain an empty historical table only if roadmap audit policy requires it.

### Phase 11: Final audit, pruning and roadmap release

1. Rerun the repository search pass.
2. Remove superseded helpers, duplicate tests and temporary migration adapters. This includes
   the two `test_diagnostics.rs` helpers from AUD-0001-F05 if Phase 7 did not adopt them, and
   resolving the fixture-support name collision recorded below.
3. Resolve AUD-0001-F04: `build_ast` and `reference_expr` each exist twice under
   `src/compiler_frontend/tests/` with the same name and different semantics
   (`hir_fixture_support` forwards to production lowering; `type_id_fixture_support` registers
   types into a fresh `TypeEnvironment`, and its `reference_expr` fixes a different `ValueMode`
   and `DataType`). Ten or more modules import both supports, so an import can silently select
   the wrong fixture semantics and still compile. This is an honesty defect under
   "fixture setup is observable": the fixture a test builds must be the one its author named.
   Decide the TypeId-first migration question that this blocks, then either finish the
   migration and delete the superseded helpers, or give the two shapes self-describing names
   (for example `build_ast_from_production_lowering` and `build_ast_with_registered_types`).
   Delete the bare `hir_fixture_support::build_ast` forward either way. A rename here must be
   a pure rename: if a consumer turns out to have imported the other module's helper than its
   author intended, that is a separate exposed defect for the ledger, not a silent fix.
4. Confirm every remaining smoke case is named honestly.
5. Confirm every ignored test is owned and none was added by this plan.
6. Run the complete validation matrix below.
7. Audit changed production seams for minimality and non-test ownership.
8. Update the roadmap capsule and mark this plan complete.
9. Only then allow frontend module compilation ownership cleanup to become active.

## Validation matrix

### Required local gates

```bash
cargo fmt --all -- --check
just test-honesty-audit
cargo test --workspace --quiet -- --format terse
cargo test --workspace --quiet --features timers -- --format terse
cargo test --workspace --quiet --features benchmark_counters -- --format terse
cargo test --workspace --quiet --features timers,benchmark_counters -- --format terse
cargo test --workspace --quiet --features detailed_timers -- --format terse
cargo test --workspace --quiet --all-features -- --format terse
cargo run --quiet -- tests --audit
cargo run --quiet -- tests --terse
just validate
```

The implementation may consolidate feature commands into faster package-owned recipes after Phase 0 measures duplication. It must preserve executed branch coverage.

### Thread and repetition gates

Run the Rust and integration owners under:

- one thread
- default parallelism
- a higher bounded thread count used by CI
- repeated execution for stateful, filesystem, timing and harness tests

At minimum:

```bash
cargo test --workspace --quiet -- --test-threads=1
MOTH_TEST_THREADS=1 cargo run --quiet -- tests --terse
MOTH_TEST_THREADS=8 cargo run --quiet -- tests --terse
```

Add an owned repeat recipe rather than relying on a developer shell loop if repeat validation becomes part of CI.

### Platform gates

Linux, macOS and Windows each run:

- native Clippy
- default unit tests
- applicable feature lanes
- canonical integration tests
- honesty audit
- platform-owned filesystem tests

A platform-specific test may be absent only when the underlying API does not exist on that platform. Equivalent portable policy tests should still run everywhere.

### CI reporting gate

One failed validation family must not prevent independent families from reporting. The final workflow result remains failed if any blocking gate fails.

## Completion criteria

This plan is complete only when all of the following hold:

- `temp_dir` is deleted or no longer means an uncreated directory
- sensitive filesystem absence checks distinguish `NotFound` from every other IO result
- no multi-lane negative test accepts an unspecified error
- infrastructure errors can be enumerated independently of diagnostic order
- synthetic test values preserve production invariants
- current directory, environment and global instrumentation state restore failures are visible
- lossy path and text conversions are removed from test-critical boundaries or explicitly justified
- positive tests prove exact target facts rather than generic non-emptiness
- artifact paths are unique before any assertion consumes them
- golden comparison rejects wrong artifact kinds and invalid text encoding
- Node harness processes have owned temporary state and bounded execution
- fixture discovery fails closed on metadata, identity and encoding problems
- deterministic tests no longer depend on arbitrary sleeps or yield counts
- every maintained feature-gated test branch executes in validation
- CI reports independent validation families after earlier failures
- smoke and acceptance-only cases do not claim semantic ownership they do not prove
- every ignored test is owned and no exposed failure was ignored
- source-text tests are not sole semantic evidence
- no two fixture-support helpers share a name with different fixture semantics
- reports are atomic and identify completed runs
- `test_honesty_inventory.json` has no hard findings and every review finding has a disposition
- the exposed-failure ledger is empty after Patch B
- the full validation, feature, thread and platform matrix passes

## Non-goals and guardrails

- Do not create a general mocking framework.
- Do not replace visible Moth fixtures with an opaque builder DSL.
- Do not snapshot whole internal structs when a stable semantic field owns the contract.
- Do not make every output byte contractual.
- Do not ban `is_err`, `contains`, `any` or acceptance-only tests without context.
- Do not add retries around deterministic compiler assertions. Retries are allowed only for documented OS cleanup races and must still report final failure.
- Do not fix compiler defects inside the honesty checkpoint. Keep the evidence boundary reviewable.
- Do not continue the roadmap with an open failure ledger.

## Expected end state

After this plan, a green Moth test run means:

- fixtures were created as intended
- the intended compiler or tooling path ran
- expected failures happened for the stated reason
- expected successes produced the stated result
- filesystem and process failures could not impersonate language behavior
- all maintained feature and platform branches were exercised
- CI showed every independent failure it could find in one run

That is the evidence baseline required before the rest of the roadmap continues.