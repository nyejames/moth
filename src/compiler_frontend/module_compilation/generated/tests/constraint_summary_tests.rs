//! End-to-end summary-bit convergence over real Moth programs.
//!
//! WHAT: compiles small module fixtures through the canonical service and asserts the final
//!       `uses_fast_semantics`, `may_trap` and `may_fail` bits on base and generated summaries.
//! WHY: phase B is the only post-lane summary writer. These tests exercise the real lane
//!       installation, convergence, proof policy and write-back path that unit tests deliberately
//!       bypass, including the `escapes_builtin_failure`/`may_fail` distinction.

use crate::compiler_frontend::compiler_messages::render::terse;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;

use super::*;

fn compile_success(source: &str) -> ProjectFrontendCompilation {
    let (outcome, _) = compile_owned_service_fixture(source, true, &[]);
    match outcome {
        CompilationOutcome::Success { value, .. } => value,
        CompilationOutcome::Diagnosed(reports) => panic!(
            "fixture must compile successfully: {}",
            terse::format_terse_diagnostic_reports(&reports).join("\n")
        ),
    }
}

fn base_module(frontend: &ProjectFrontendCompilation) -> &Module {
    frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artifact| artifact.module.metadata.entry.is_some())
        .map(|artifact| &artifact.module)
        .expect("fixture should compile one entry module")
}

fn function_named(module: &Module, name: &str) -> FunctionId {
    let hir = &module.executable.hir;
    hir.function_ids_by_private_origin
        .iter()
        .find(|(identity, _)| identity.defining_name() == name)
        .map(|(_, function)| *function)
        .or_else(|| {
            hir.function_ids_by_origin
                .iter()
                .find(|(identity, _)| identity.defining_name() == name)
                .map(|(_, function)| *function)
        })
        .unwrap_or_else(|| panic!("base module should retain a function named {name}"))
}

fn summary_named<'a>(module: &'a Module, name: &str) -> &'a PublicCallSummary {
    let function = function_named(module, name);
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .get(&function)
        .unwrap_or_else(|| panic!("base report should retain {name}'s summary"))
}

fn entry_summary(module: &Module) -> &PublicCallSummary {
    let hir = &module.executable.hir;
    let function = hir
        .start_function
        .expect("fixture should retain an entry start function");
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .get(&function)
        .expect("entry should retain a call summary")
}

#[test]
fn direct_and_folded_fast_arithmetic_mark_the_summary_without_trapping() {
    let frontend = compile_success(
        "$fast_math\n\
         direct |value I32| -> I32:\n\
             return value + 1\n\
         ;\n\
         folded || -> I32:\n\
             $fast_math\n\
             return 1 + 2\n\
         ;\n\
         left = direct(1)\n\
         right = folded()\n\
         ~[: [left][right]]\n",
    );
    let module = base_module(&frontend);

    let direct = summary_named(module, "direct");
    assert!(direct.uses_fast_semantics);
    assert!(!direct.may_trap, "fast integer addition wraps");

    let folded = summary_named(module, "folded");
    assert!(
        folded.uses_fast_semantics,
        "a fast operation that folded away still counts as selected evidence"
    );
    assert!(!folded.may_trap, "the folded operation executes no trap");
}

#[test]
fn a_fast_range_guard_without_authored_arithmetic_is_fast_and_traps() {
    let frontend = compile_success(
        "$fast_math\n\
         sweep |step I64| -> I32:\n\
             loop 0 to 3 by step |item|:\n\
                 marker = item\n\
             ;\n\
             return 0\n\
         ;\n\
         result = sweep(1)\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "sweep");
    assert!(
        summary.uses_fast_semantics,
        "a fast range guard is fast use even with no authored arithmetic"
    );
    assert!(summary.may_trap, "a fast range guard is a fatal trap site");
    assert!(!summary.may_fail, "fast delivery never becomes recoverable");
}

#[test]
fn a_total_fast_store_conversion_is_fast_use_and_not_a_trap() {
    let frontend = compile_success(
        "$fast_math\n\
         wrap |target ~I8, increment I16|:\n\
             ~target += increment\n\
         ;\n\
         total ~I8 = 7\n\
         wrap(~total, 100)\n\
         ~[: [total]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "wrap");
    assert!(
        summary.uses_fast_semantics,
        "an integer fast write-back is selected fast semantics even when it wraps"
    );
    assert!(!summary.may_trap, "an integer fast write-back is total");
}

#[test]
fn dec_only_arithmetic_and_a_vacuous_annotation_are_not_fast() {
    let frontend = compile_success(
        "$fast_math\n\
         decimal |left Dec2, right Dec2| -> Dec2:\n\
             return left + right\n\
         ;\n\
         quiet || -> I32:\n\
             $fast_math\n\
             return 1\n\
         ;\n\
         value = decimal(1.0, 2.0)\n\
         number = quiet()\n\
         ~[: [number]]\n",
    );
    let module = base_module(&frontend);
    assert!(
        !summary_named(module, "decimal").uses_fast_semantics,
        "Dec never enters fast mode"
    );
    assert!(
        !summary_named(module, "quiet").uses_fast_semantics,
        "an annotation with no selected fast operation is vacuous"
    );
}

#[test]
fn calls_propagate_fast_use_transitively_and_through_recursion() {
    let frontend = compile_success(
        "$fast_math\n\
         leaf || -> I32:\n\
             return 1 + 2\n\
         ;\n\
         middle || -> I32:\n\
             return leaf()\n\
         ;\n\
         top || -> I32:\n\
             return middle()\n\
         ;\n\
         ping |value I32| -> I32:\n\
             if value > 0:\n\
                 return pong(value - 1)\n\
             ;\n\
             return value\n\
         ;\n\
         pong |value I32| -> I32:\n\
             if value > 0:\n\
                 return ping(value - 1)\n\
             ;\n\
             return value\n\
         ;\n\
         result = top()\n\
         other = ping(1)\n\
         ~[: [result][other]]\n",
    );
    let module = base_module(&frontend);
    for name in ["leaf", "middle", "top"] {
        assert!(
            summary_named(module, name).uses_fast_semantics,
            "{name} should inherit fast use transitively"
        );
    }
    for name in ["ping", "pong"] {
        assert!(
            !summary_named(module, name).uses_fast_semantics,
            "{name} is a mutual-recursion cycle with no fast evidence"
        );
    }
}

#[test]
fn mutual_recursion_with_fast_evidence_reaches_both_cycles() {
    let frontend = compile_success(
        "$fast_math\n\
         alpha |value I32| -> I32:\n\
             if value > 0:\n\
                 return beta(value - 1)\n\
             ;\n\
             return 1 + 2\n\
         ;\n\
         beta |value I32| -> I32:\n\
             if value > 0:\n\
                 return alpha(value - 1)\n\
             ;\n\
             return value\n\
         ;\n\
         result = alpha(2)\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    assert!(summary_named(module, "alpha").uses_fast_semantics);
    assert!(
        summary_named(module, "beta").uses_fast_semantics,
        "the reverse edge must propagate the cycle's fast evidence"
    );
}

#[test]
fn a_literal_division_proof_clears_trap_and_fail_while_escaping_stays_true() {
    let frontend = compile_success(
        "halve |value I32| -> I32:\n\
             return value // 2\n\
         ;\n\
         result = halve(10)\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "halve");
    assert!(
        summary.escapes_builtin_failure,
        "the AST-level escaping bit still routes the inferred lane"
    );
    assert!(
        !summary.may_trap,
        "a nonzero literal divisor without -1 discharges both // causes"
    );
    assert!(
        !summary.may_fail,
        "a discharged failure edge is dead after lanes"
    );
}

#[test]
fn an_unproved_divisor_keeps_a_recoverable_failure() {
    let frontend = compile_success(
        "halve |value I32, divisor I32| -> I32:\n\
             return value // divisor\n\
         ;\n\
         result = halve(10, 2)\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "halve");
    assert!(
        summary.may_fail,
        "the lane converts the unproved division into a recoverable escape"
    );
    assert!(
        !summary.may_trap,
        "post-lane recoverable delivery is not a trap"
    );
}

#[test]
fn a_reachable_assertion_marks_the_summary_as_trapping() {
    let frontend = compile_success(
        "check || -> I32:\n\
             assert(false, \"stop\")\n\
             return 1\n\
         ;\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "check");
    assert!(summary.may_trap);
    assert!(!summary.may_fail);
    assert!(!summary.uses_fast_semantics);
}

#[test]
fn a_caught_ordinary_error_with_a_total_handler_does_not_fail() {
    let frontend = compile_success(
        "safe |left I32, right I32| -> I32:\n\
             value = left // right catch then 0\n\
             return value\n\
         ;\n\
         result = safe(10, 2)\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "safe");
    assert!(
        !summary.may_fail,
        "a total catch handler contains the recoverable division failure"
    );
    assert!(!summary.may_trap);
}

#[test]
fn a_catch_handler_that_returns_the_error_marks_the_summary_as_failing() {
    let frontend = compile_success(
        "recover |left I32, right I32| -> I32, Error!:\n\
             value = left // right catch |error|:\n\
                 return! error\n\
             ;\n\
             return value\n\
         ;\n\
         result = recover(10, 2) catch then 0\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "recover");
    assert!(
        summary.may_fail,
        "the handler's reachable return! is a recoverable escape"
    );
}

#[test]
fn a_trusted_core_io_call_is_neither_fast_nor_trapping() {
    let frontend = compile_success("io.line(\"hi\")\n");
    let module = base_module(&frontend);
    let summary = entry_summary(module);
    assert!(
        !summary.uses_fast_semantics,
        "Core IO publishes a trusted ordinary-semantics contract"
    );
    assert!(
        !summary.may_trap,
        "Core IO publishes a trusted trap-free contract"
    );
    assert!(
        !summary.may_fail,
        "a plain external call has no error channel"
    );
}

#[test]
fn a_trusted_core_io_read_marks_the_summary_as_failing() {
    let frontend = compile_success(
        "open || -> I32, Error!:\n\
             handle = io.input.new()!\n\
             return 1\n\
         ;\n\
         result = open() catch then 0\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "open");
    assert!(
        !summary.uses_fast_semantics,
        "Core IO publishes a trusted ordinary-semantics contract"
    );
    assert!(
        !summary.may_trap,
        "Core IO publishes a trusted trap-free contract"
    );
    assert!(
        summary.may_fail,
        "`io.input.new` declares a recoverable Unsupported error that `return!` propagates"
    );
}

#[test]
fn an_exhaustive_choice_match_stays_trap_free() {
    let frontend = compile_success(
        "Status ::\n\
             Ready,\n\
             Waiting,\n\
         ;\n\
         pick |status Status| -> I32:\n\
             if status is:\n\
                 Ready => return 1\n\
                 Waiting => return 2\n\
             ;\n\
         ;\n\
         result = pick(Status::Ready)\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "pick");
    assert!(
        !summary.may_trap,
        "the exhaustive-match fallback is compiler machinery, not a reachable trap"
    );
}

#[test]
fn declaration_order_does_not_change_the_summary_bits() {
    let forward = compile_success(
        "$fast_math\n\
         first || -> I32:\n\
             return 1 + 2\n\
         ;\n\
         $fast_math\n\
         second || -> I32:\n\
             return first()\n\
         ;\n\
         result = second()\n\
         ~[: result=[result]]\n",
    );
    let reversed = compile_success(
        "$fast_math\n\
         second || -> I32:\n\
             return first()\n\
         ;\n\
         $fast_math\n\
         first || -> I32:\n\
             return 1 + 2\n\
         ;\n\
         result = second()\n\
         ~[: result=[result]]\n",
    );
    let forward_module = base_module(&forward);
    let reversed_module = base_module(&reversed);
    for name in ["first", "second"] {
        let forward = summary_named(forward_module, name);
        let reversed = summary_named(reversed_module, name);
        assert_eq!(
            (
                forward.uses_fast_semantics,
                forward.may_trap,
                forward.may_fail,
            ),
            (
                reversed.uses_fast_semantics,
                reversed.may_trap,
                reversed.may_fail,
            ),
            "{name}'s bits must not depend on declaration order"
        );
    }
}

#[test]
fn a_materialised_generic_sidecar_gets_its_own_final_bits() {
    let frontend = compile_success(
        "$fast_math\n\
         probe type T |marker T| -> I32:\n\
             divisor ~I32 = 2\n\
             return 100 // divisor\n\
         ;\n\
         result = probe(0)\n\
         ~[: result=[result]]\n",
    );
    let sidecar = frontend
        .project
        .successful_module_views()
        .find(|module| !module.executable.hir.function_ids_by_generated.is_empty())
        .expect("the fixture should publish one generated sidecar");
    let hir = &sidecar.executable.hir;
    let root = *hir
        .function_ids_by_generated
        .values()
        .next()
        .expect("the sidecar should retain its generated root");
    let summary = &sidecar
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries[&root];
    assert!(
        summary.uses_fast_semantics,
        "the materialised instance selects the template's fast division"
    );
    assert!(
        !summary.may_trap,
        "the literal divisor proof applies inside the instance"
    );
    assert!(!summary.may_fail);
}

#[test]
fn final_bits_survive_publication_without_a_later_borrow_rebuild() {
    // Any later `check_borrows` would re-seed the three bits false. A true bit on the published
    // report therefore proves phase B is the last summary writer for this module.
    let frontend = compile_success(
        "$fast_math\n\
         evidence || -> I32:\n\
             return 1 + 2\n\
         ;\n\
         result = evidence()\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    assert!(
        summary_named(module, "evidence").uses_fast_semantics,
        "published summary must retain phase B's final bit"
    );
    assert!(
        entry_summary(module).uses_fast_semantics,
        "the caller's propagated bit must also survive publication"
    );
}

#[test]
fn a_mutable_call_argument_invalidates_a_literal_divisor_proof() {
    let frontend = compile_success(
        "mutate |target ~I32|:\n\
             ~target = 0\n\
         ;\n\
         prove || -> I32:\n\
             divisor ~I32 = 2\n\
             mutate(~divisor)\n\
             return 10 // divisor\n\
         ;\n\
         result = prove()\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "prove");
    assert!(
        summary.may_fail,
        "the callee may overwrite the literal divisor, so the ordinary division still fails"
    );
    assert!(
        !summary.may_trap,
        "the ordinary division's failure is delivered through the private lane"
    );
}

#[test]
fn a_mutable_call_argument_invalidates_a_fast_literal_divisor_proof() {
    let frontend = compile_success(
        "mutate |target ~I32|:\n\
             ~target = 0\n\
         ;\n\
         $fast_math\n\
         prove || -> I32:\n\
             divisor ~I32 = 2\n\
             mutate(~divisor)\n\
             return 10 // divisor\n\
         ;\n\
         result = prove()\n\
         ~[: result=[result]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "prove");
    assert!(
        summary.may_trap,
        "a fast division after mutable overwrite is a fatal trap site"
    );
    assert!(!summary.may_fail, "fast delivery never becomes recoverable");
}

#[test]
fn a_fast_callee_called_only_from_a_pruned_handler_still_counts_as_fast_use() {
    // The protected `//` still has a construction-time failure edge, so this handler is reachable
    // while evidence is projected and is pruned only after convergence. The unrouted-root case is
    // covered by `a_fast_callee_in_an_unrouted_handler_root_block_still_counts_as_fast_use`.
    let frontend = compile_success(
        "$fast_math\n\
         witness || -> I32:\n\
             return 1 + 2\n\
         ;\n\
         probe |value I32| -> I32:\n\
             result = value // 2 catch |error|:\n\
                 return witness()\n\
             ;\n\
             return result\n\
         ;\n\
         answer = probe(10)\n\
         ~[: answer=[answer]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "probe");
    assert!(
        summary.uses_fast_semantics,
        "the selected call in the pruned handler is still fast-use evidence"
    );
    assert!(!summary.may_trap);
    assert!(!summary.may_fail);
}

#[test]
fn a_fast_callee_in_an_unrouted_handler_root_block_still_counts_as_fast_use() {
    // A safety-catch around a comparison builds no failure edge, so the handler is unrouted from
    // the start and convergence prunes it. The only fast-use witness is the projected call in its
    // root block; the comparison itself is ordinary code.
    let frontend = compile_success(
        "probe |value I32| -> Bool:\n\
             result = calm(value) < calm(1) catch |error|:\n\
                 return witness(value, 1)\n\
             ;\n\
             return result\n\
         ;\n\
         calm |value I32| -> I32:\n\
             return value\n\
         ;\n\
         $fast_math\n\
         witness |left I32, right I32| -> Bool:\n\
             marker = left + right\n\
             return marker > 0\n\
         ;\n\
         answer = probe(5)\n\
         ~[: answer=[answer]]\n",
    );
    let module = base_module(&frontend);
    let summary = summary_named(module, "probe");
    assert!(
        summary.uses_fast_semantics,
        "the projected root-block call in the pruned handler is fast-use evidence"
    );
    assert!(!summary.may_trap);
    assert!(!summary.may_fail);

    // Without pruning the final CFG walk would find the call itself and mask a projection gap,
    // so assert the handler really is gone from final HIR.
    let hir = &module.executable.hir;
    let witness = function_named(module, "witness");
    let target_is_witness =
        |target: &CallTarget| matches!(target, CallTarget::Local(function) if *function == witness);
    let retained_call = hir.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call { target, .. } if target_is_witness(target)
            )
        }) || match &block.terminator {
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { target, .. },
                ..
            }
            | HirTerminator::NeverCall { target, .. } => target_is_witness(target),
            _ => false,
        }
    });
    assert!(
        !retained_call,
        "the infallible protected call must prune its handler from final HIR"
    );
}
