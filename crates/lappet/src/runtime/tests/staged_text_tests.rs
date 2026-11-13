//! Staged text through a guest, under both optimisation levels.

use super::{assemble, engine, under_both_optimisations};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::staging::StagingTable;
use crate::runtime::{BoundaryFault, InstanceLimits, RuntimeEngine, RuntimeError};
use wasmtime::{Val, ValType};

const STAGED: &[u8] = b"hello";

struct StageHost {
    recorded: Vec<i32>,
}

fn prepared(engine: &RuntimeEngine) -> crate::runtime::instance::PreparedModule<StageHost> {
    prepared_from(engine, include_str!("fixtures/staged_text.wat"))
}

fn prepared_from(
    engine: &RuntimeEngine,
    wat_source: &str,
) -> crate::runtime::instance::PreparedModule<StageHost> {
    let mut imports = HostImports::new(engine);
    imports
        .define(
            "host",
            "stage",
            [],
            [ValType::I64],
            |_host: &mut StageHost,
             staging: &mut StagingTable,
             limits,
             _resources,
             _arguments,
             results| {
                let ticket = staging
                    .stage(STAGED, limits)
                    .map_err(|_| "the staged text did not fit the limits")?;
                results[0] = Val::I64(i64::try_from(ticket).expect("a test ticket fits in i64"));
                Ok(())
            },
        )
        .expect("the stage import is defined once");
    imports
        .define(
            "host",
            "record",
            [ValType::I32],
            [],
            |host, _staging, _limits, _resources, arguments, _results| {
                host.recorded
                    .push(arguments[0].i32().expect("the import takes one i32"));
                Ok(())
            },
        )
        .expect("the record import is defined once");

    let module = engine
        .compile_module(&assemble(wat_source))
        .expect("the fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture imports are provided")
}

fn instantiate(optimisation: crate::runtime::NativeOptimisation) -> Instance<StageHost> {
    prepared(&engine(optimisation))
        .instantiate(
            StageHost {
                recorded: Vec::new(),
            },
            InstanceLimits::default(),
            Deadline::Unbounded,
        )
        .expect("the fixture has no start function")
}

fn assert_boundary(result: Result<impl Sized, RuntimeError>, expected: BoundaryFault) {
    let Err(RuntimeError::BoundaryFault(fault)) = result else {
        panic!("expected a boundary fault");
    };
    assert_eq!(fault, expected);
}

#[test]
fn a_start_function_that_leaves_a_ticket_live_fails_instantiation() {
    let leaving_a_ticket = r#"
        (module
          (import "host" "stage" (func $stage (result i64)))
          (func $start (drop (call $stage)))
          (start $start))
    "#;
    let taking_the_ticket = r#"
        (module
          (import "host" "stage" (func $stage (result i64)))
          (import "moth-abi-v1" "staged_text_take" (func $take (param i64 i32 i32)))
          (memory (export "memory") 1)
          (func $start (call $take (call $stage) (i32.const 0) (i32.const 5)))
          (start $start))
    "#;

    under_both_optimisations(|optimisation| {
        let runtime = engine(optimisation);
        let left_live = prepared_from(&runtime, leaving_a_ticket).instantiate(
            StageHost {
                recorded: Vec::new(),
            },
            InstanceLimits::default(),
            Deadline::Unbounded,
        );
        assert_boundary(left_live, BoundaryFault::TicketLiveAtReturn);

        let taken = prepared_from(&runtime, taking_the_ticket).instantiate(
            StageHost {
                recorded: Vec::new(),
            },
            InstanceLimits::default(),
            Deadline::Unbounded,
        );
        assert!(taken.is_ok(), "{:?}", taken.err());
    });
}

#[test]
fn staged_text_is_copied_and_the_ticket_is_consumed() {
    under_both_optimisations(|optimisation| {
        let mut instance = instantiate(optimisation);

        instance
            .call::<(i32, i32), ()>("take_staged", (0, STAGED.len() as i32), Deadline::Unbounded)
            .expect("the destination is inside the first page");
        assert!(!instance.has_live_staged_text());

        let mut copied = Vec::new();
        for address in 0..STAGED.len() as i32 {
            let byte: i32 = instance
                .call("load_byte", address, Deadline::Unbounded)
                .expect("the copied byte is in range");
            copied.push(byte as u8);
        }
        assert_eq!(copied, STAGED);
    });
}

#[test]
fn a_bad_take_quarantines_and_clears_the_table() {
    let cases = [
        (
            "out of bounds",
            (65_534, STAGED.len() as i32),
            BoundaryFault::DestinationOutOfBounds,
        ),
        ("wrong length", (0, 4), BoundaryFault::WrongLength),
    ];

    under_both_optimisations(|optimisation| {
        for (label, arguments, expected) in cases {
            let mut instance = instantiate(optimisation);
            let result =
                instance.call::<(i32, i32), ()>("take_staged", arguments, Deadline::Unbounded);
            assert_boundary(result, expected);
            assert!(!instance.has_live_staged_text(), "{label}");
            let refused = instance.call::<i32, i32>("load_byte", 0, Deadline::Unbounded);
            assert!(
                matches!(refused, Err(RuntimeError::InstanceFaulted)),
                "{label}"
            );
        }
    });
}

#[test]
fn taking_one_ticket_twice_is_a_boundary_fault() {
    under_both_optimisations(|optimisation| {
        let mut instance = instantiate(optimisation);
        let result = instance.call::<(), ()>("take_twice", (), Deadline::Unbounded);
        assert_boundary(result, BoundaryFault::UnknownTicket);
        assert!(!instance.has_live_staged_text());
    });
}

#[test]
fn a_live_ticket_at_another_import_is_a_boundary_fault() {
    under_both_optimisations(|optimisation| {
        let mut instance = instantiate(optimisation);
        let result = instance.call::<(), ()>("stage_then_record", (), Deadline::Unbounded);
        assert_boundary(result, BoundaryFault::TicketLiveAtImport);
        assert!(instance.host_state().recorded.is_empty());
        assert!(!instance.has_live_staged_text());
    });
}

#[test]
fn a_live_ticket_at_entry_return_is_a_boundary_fault() {
    under_both_optimisations(|optimisation| {
        let mut instance = instantiate(optimisation);
        let result = instance.call::<(), ()>("stage_and_return", (), Deadline::Unbounded);
        assert_boundary(result, BoundaryFault::TicketLiveAtReturn);
        assert!(!instance.has_live_staged_text());
        let refused = instance.call::<(), ()>("stage_and_return", (), Deadline::Unbounded);
        assert!(matches!(refused, Err(RuntimeError::InstanceFaulted)));
    });
}
