//! The resource table, under both optimisation levels where an instance is involved.
//!
//! Tokens come from one process-wide generator. The table checks a token before a Rust
//! reference exists, and a fault does not take the embedder's values away.

use super::{assemble, engine, under_both_optimisations};
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::resource::{
    HostStateBound, ResourceError, ResourceToken, Sendable, ThreadBound,
};
use crate::runtime::{InstanceLimits, RuntimeError};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

fn instance<Kind>(optimisation: crate::runtime::NativeOptimisation) -> Instance<(), Kind>
where
    (): HostStateBound<Kind>,
    Kind: crate::runtime::resource::InstanceKind,
{
    let engine = engine(optimisation);
    let imports = HostImports::<(), Kind>::new(&engine);
    let module = engine
        .compile_module(&assemble("(module (func (export \"trap\") unreachable))"))
        .expect("the trap fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture has no imports")
        .instantiate((), InstanceLimits::default(), Deadline::Unbounded)
        .expect("the fixture has no start function")
}

#[test]
fn tokens_from_two_instances_never_collide_and_a_foreign_token_is_not_live() {
    under_both_optimisations(|optimisation| {
        let mut first = instance::<Sendable>(optimisation);
        let mut second = instance::<Sendable>(optimisation);
        let here = first
            .insert_resource(1u32)
            .expect("the first table has room");
        let there = second
            .insert_resource(1u32)
            .expect("the second table has room");

        assert_ne!(here.raw(), there.raw());
        let foreign = ResourceToken::<u32>::from_raw(there.raw()).expect("the carrier is a token");
        assert_eq!(first.resource(foreign), Err(ResourceError::NotLive));
        assert_eq!(
            *second.resource(there).expect("the owner can resolve it"),
            1
        );
    });
}

#[test]
fn zero_is_not_a_token_and_a_closed_token_is_not_live() {
    under_both_optimisations(|optimisation| {
        assert!(ResourceToken::<u32>::from_raw(0).is_none());
        assert!(ResourceToken::<u32>::from_raw(-1).is_none());

        let mut instance = instance::<Sendable>(optimisation);
        let token = instance.insert_resource(7u32).expect("the table has room");
        assert_eq!(instance.close_resource(token), Ok(7));
        assert_eq!(instance.resource(token), Err(ResourceError::NotLive));
        assert_eq!(instance.close_resource(token), Err(ResourceError::NotLive));
    });
}

#[test]
fn a_wrong_type_is_rejected_and_the_entry_stays() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance::<Sendable>(optimisation);
        let token = instance
            .insert_resource(String::from("sprite"))
            .expect("the table has room");
        let wrong = ResourceToken::<u32>::from_raw(token.raw()).expect("the carrier is a token");

        assert_eq!(instance.resource(wrong), Err(ResourceError::WrongType));
        assert_eq!(
            instance.close_resource(wrong),
            Err(ResourceError::WrongType)
        );
        instance
            .resource_mut(token)
            .expect("the type was checked before the reference")
            .push('s');
        assert_eq!(
            instance.resource(token).expect("the entry stays").as_str(),
            "sprites"
        );
    });
}

#[test]
fn a_full_table_rejects_the_next_insert_and_keeps_the_live_entry() {
    under_both_optimisations(|optimisation| {
        let engine = engine(optimisation);
        let imports = HostImports::<()>::new(&engine);
        let module = engine
            .compile_module(&assemble("(module)"))
            .expect("an empty module is valid");
        let mut instance = imports
            .prepare(&module)
            .expect("the module has no imports")
            .instantiate(
                (),
                InstanceLimits {
                    max_live_resources: 1,
                    ..InstanceLimits::default()
                },
                Deadline::Unbounded,
            )
            .expect("the module has no start function");

        let token = instance.insert_resource(1u8).expect("the first entry fits");
        assert_eq!(instance.insert_resource(2u8), Err(ResourceError::Full));
        assert_eq!(*instance.resource(token).expect("the first entry stays"), 1);
    });
}

#[test]
fn a_faulted_instance_still_closes_and_drains() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance::<Sendable>(optimisation);
        let closed = instance.insert_resource(1u32).expect("the table has room");
        let drained = instance.insert_resource(2u32).expect("the table has room");
        let fault = instance.call::<(), ()>("trap", (), Deadline::Unbounded);
        assert!(matches!(fault, Err(RuntimeError::GuestTrap { .. })));
        let refused = instance.call::<(), ()>("trap", (), Deadline::Unbounded);
        assert!(matches!(refused, Err(RuntimeError::InstanceFaulted)));

        assert_eq!(instance.close_resource(closed), Ok(1));
        let mut recovered = instance.drain_resources().into_iter();
        let value = recovered
            .next()
            .expect("the second value is still owned by the embedder")
            .downcast::<u32>()
            .expect("the drained value is the inserted u32");
        assert!(recovered.next().is_none());
        assert_eq!(*value, 2);
        assert_eq!(instance.resource(drained), Err(ResourceError::NotLive));
    });
}

#[test]
fn teardown_drops_each_value_once_in_creation_order() {
    under_both_optimisations(|optimisation| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut instance = instance::<Sendable>(optimisation);
        for id in [1, 2, 3] {
            instance
                .insert_resource(DropProbe {
                    id,
                    log: Arc::clone(&log),
                })
                .expect("the table has room");
        }
        drop(instance);

        let dropped = log.lock().expect("the drop log is not poisoned");
        assert_eq!(dropped.as_slice(), [1, 2, 3]);
    });
}

#[test]
fn a_thread_bound_instance_stores_a_value_that_is_not_send() {
    under_both_optimisations(|optimisation| {
        let mut instance = instance::<ThreadBound>(optimisation);
        let kept = Rc::new(4u8);
        let token = instance
            .insert_resource(Rc::clone(&kept))
            .expect("a thread-bound table accepts a value that is not Send");
        let returned = instance
            .close_resource(token)
            .expect("close returns the value");
        assert!(Rc::ptr_eq(&kept, &returned));
    });
}

#[test]
fn a_thread_bound_instance_cannot_move_to_another_thread() {
    // Compiles only while `Instance<(), ThreadBound>` is not `Send`. Making the instance `Send`
    // makes `AmbiguousIfImpl<_>` ambiguous and this test stops compiling.
    trait AmbiguousIfImpl<Marker> {
        fn some_item() {}
    }
    impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
    struct WouldBeSend;
    impl<T: ?Sized + Send> AmbiguousIfImpl<WouldBeSend> for T {}
    let _ = <Instance<(), ThreadBound> as AmbiguousIfImpl<_>>::some_item;
}

#[test]
fn a_sendable_instance_rejects_host_state_that_is_not_send() {
    // Compiles only while `Rc<u8>` cannot create a sendable instance. Removing the `Send` bound
    // makes this assertion ambiguous.
    trait AmbiguousIfImpl<Marker> {
        fn some_item() {}
    }
    impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
    struct WouldBeSendableHost;
    impl<T: ?Sized + HostStateBound<Sendable>> AmbiguousIfImpl<WouldBeSendableHost> for T {}
    let _ = <Rc<u8> as AmbiguousIfImpl<_>>::some_item;

    fn assert_send<T: Send>() {}
    assert_send::<Instance<(), Sendable>>();
}

// Only the test below sets this cfg. The workspace lint table names it so the normal build accepts the attribute.
#[cfg(lappet_reject_non_send_insert)]
fn _sendable_insert_of_rc_must_not_compile(instance: &mut Instance<(), Sendable>) {
    let _ = instance.insert_resource(std::rc::Rc::new(1u8));
}

#[test]
fn a_sendable_instance_rejects_a_resource_that_is_not_send() {
    let output = std::process::Command::new("cargo")
        .args([
            "check",
            "-p",
            "lappet",
            "--features",
            "runtime",
            "--offline",
            "--tests",
        ])
        .env("RUSTFLAGS", "--cfg lappet_reject_non_send_insert")
        .env("CARGO_TARGET_DIR", "tmp/resource-compile-fail")
        .output()
        .expect("cargo check runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "inserting Rc into a sendable instance compiled:\n{stderr}"
    );
    assert!(
        stderr.contains("insert_resource") && stderr.contains("Rc<u8>") && stderr.contains("Send"),
        "{stderr}"
    );
}

struct DropProbe {
    id: u32,
    log: Arc<Mutex<Vec<u32>>>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        if let Ok(mut log) = self.log.lock() {
            log.push(self.id);
        }
    }
}
