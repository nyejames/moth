//! Call-scoped resource access, under both optimisation levels.
//!
//! The guest passes tokens as `i64`. The import checks the token, the type and the borrow
//! before it touches host state. Dropping the guard, including by a panic, releases the entry.

use super::{assemble, engine, under_both_optimisations};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::resource::AccessRequest;
use crate::runtime::{BoundaryFault, InstanceLimits, RuntimeError};
use wasmtime::{Val, ValType};

struct AccessHost {
    ran: bool,
}

fn instance(optimisation: crate::runtime::NativeOptimisation) -> Instance<AccessHost> {
    let engine = engine(optimisation);
    let mut imports = HostImports::new(&engine);
    define(&mut imports).expect("each access import is defined once");
    let module = engine
        .compile_module(&assemble(include_str!("fixtures/resource_access.wat")))
        .expect("the fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture imports are provided")
        .instantiate(
            AccessHost { ran: false },
            InstanceLimits::default(),
            Deadline::Unbounded,
        )
        .expect("the fixture has no start function")
}

fn define(imports: &mut HostImports<AccessHost>) -> Result<(), RuntimeError> {
    imports.define(
        "host",
        "shared",
        [ValType::I64],
        [],
        |host, _, _, resources, arguments, _| {
            resources.with_access(&[request(token(arguments), false)], |_| {
                host.ran = true;
            })?;
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "exclusive",
        [ValType::I64],
        [],
        |host, _, _, resources, arguments, _| {
            resources.with_access(&[request(token(arguments), true)], |access| {
                let _ = access.exclusive::<u32>(0);
                host.ran = true;
            })?;
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "shared_both",
        [ValType::I64, ValType::I64],
        [],
        |host, _, _, resources, arguments, _| {
            let second = arguments[1].i64().expect("the import takes two i64 tokens");
            resources.with_access(
                &[request(token(arguments), false), request(second, false)],
                |access| {
                    let _ = access.shared::<u32>(0);
                    let _ = access.shared::<u32>(1);
                    host.ran = true;
                },
            )?;
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "exclusive_then_shared",
        [ValType::I64],
        [],
        |host, _, _, resources, arguments, _| {
            let raw = token(arguments);
            resources.with_access(&[request(raw, true), request(raw, false)], |_| {
                host.ran = true;
            })?;
            Ok(())
        },
    )?;
    imports.define(
        "host",
        "panic_holding",
        [ValType::I64],
        [],
        |_host, _, _, resources, arguments, _| {
            resources.with_access(&[request(token(arguments), true)], |_| {
                panic!("host import bug while holding a resource");
            })?;
            Ok(())
        },
    )?;
    Ok(())
}

fn request(raw: i64, exclusive: bool) -> AccessRequest {
    AccessRequest {
        raw,
        exclusive,
        type_id: std::any::TypeId::of::<u32>(),
    }
}

fn token(arguments: &[Val]) -> i64 {
    arguments[0].i64().expect("the import takes an i64 token")
}

fn assert_boundary(result: Result<impl Sized, RuntimeError>, expected: BoundaryFault) {
    match result {
        Err(RuntimeError::BoundaryFault(fault)) => assert_eq!(fault, expected),
        Err(RuntimeError::HostFault(_)) => {
            panic!("a boundary fault was classified as a host fault")
        }
        Err(RuntimeError::GuestTrap { .. }) => panic!("expected {expected:?}, got a guest trap"),
        Err(RuntimeError::Link(_)) => panic!("expected {expected:?}, got a link error"),
        Err(RuntimeError::InstanceFaulted) => {
            panic!("expected {expected:?}, the instance was already faulted")
        }
        Ok(_) => panic!("expected {expected:?}, but the call succeeded"),
        Err(_) => panic!("expected {expected:?}, got another runtime error"),
    }
}

#[test]
fn a_token_that_is_not_live_is_rejected_before_the_host_body() {
    under_both_optimisations(|optimisation| {
        let mut missing_instance = instance(optimisation);
        let missing = missing_instance.call::<i64, ()>("shared", 0, Deadline::Unbounded);
        assert_boundary(missing, BoundaryFault::TokenNotLive);
        assert!(!missing_instance.host_state().ran);

        let mut closed_instance = instance(optimisation);
        let token = closed_instance
            .insert_resource(1u32)
            .expect("the table has room");
        assert_eq!(closed_instance.close_resource(token), Ok(1));
        let closed = closed_instance.call::<i64, ()>("shared", token.raw(), Deadline::Unbounded);
        assert_boundary(closed, BoundaryFault::TokenNotLive);
        assert!(!closed_instance.host_state().ran);
    });
}

#[test]
fn a_token_from_another_instance_is_not_live() {
    under_both_optimisations(|optimisation| {
        let mut here = instance(optimisation);
        let mut there = instance(optimisation);
        let foreign = there.insert_resource(1u32).expect("the table has room");
        let result = here.call::<i64, ()>("shared", foreign.raw(), Deadline::Unbounded);
        assert_boundary(result, BoundaryFault::TokenNotLive);
        assert!(!here.host_state().ran);
    });
}

#[test]
fn a_wrong_type_is_rejected_before_the_host_body() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance(optimisation);
        let token = instance
            .insert_resource(String::from("sprite"))
            .expect("the table has room");
        let result = instance.call::<i64, ()>("shared", token.raw(), Deadline::Unbounded);
        assert_boundary(result, BoundaryFault::TokenTypeMismatch);
        assert!(!instance.host_state().ran);
        assert_eq!(
            instance.resource(token).expect("the entry stays").as_str(),
            "sprite"
        );
    });
}

#[test]
fn two_shared_borrows_coexist_and_an_exclusive_borrow_conflicts() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance(optimisation);
        let token = instance.insert_resource(1u32).expect("the table has room");

        instance
            .call::<(i64, i64), ()>(
                "shared_both",
                (token.raw(), token.raw()),
                Deadline::Unbounded,
            )
            .expect("two shared borrows of one token are legal");
        assert!(instance.host_state().ran);

        instance.host_state_mut().ran = false;
        let conflict =
            instance.call::<i64, ()>("exclusive_then_shared", token.raw(), Deadline::Unbounded);
        assert_boundary(conflict, BoundaryFault::TokenConflict);
        assert!(!instance.host_state().ran);
        assert_eq!(
            *instance
                .resource(token)
                .expect("the rejected borrow was released"),
            1
        );
    });
}

#[test]
fn a_guard_is_released_after_the_call_and_after_a_panic() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance(optimisation);
        let token = instance.insert_resource(1u32).expect("the table has room");
        instance
            .call::<i64, ()>("exclusive", token.raw(), Deadline::Unbounded)
            .expect("the first exclusive borrow succeeds");
        instance
            .call::<i64, ()>("exclusive", token.raw(), Deadline::Unbounded)
            .expect("the guard was released, so the entry can be borrowed again");

        let panicked = instance.call::<i64, ()>("panic_holding", token.raw(), Deadline::Unbounded);
        assert!(matches!(panicked, Err(RuntimeError::HostFault(_))));
        assert_eq!(
            *instance
                .resource(token)
                .expect("the panic released the guard"),
            1
        );
    });
}
