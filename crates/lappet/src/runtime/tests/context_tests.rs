//! Host context and export-argument checks, under both optimisation levels.
//!
//! Context moves into the instance for one call and back to the holder on every outcome. A
//! missing or wrong-type value is a host fault and the host function does not run. A token
//! argument is checked before guest entry, so a bad token leaves the instance usable.

use super::{assemble, engine, under_both_optimisations};
use crate::runtime::context::HostContext;
use crate::runtime::imports::HostImports;
use crate::runtime::instance::{Deadline, Instance};
use crate::runtime::{ContextFault, HostFault, InstanceLimits, RuntimeError};
use std::any::TypeId;
use wasmtime::ValType;

struct Frame {
    seen: i32,
}

struct ContextHost {
    ran: bool,
}

fn context_instance(optimisation: crate::runtime::NativeOptimisation) -> Instance<ContextHost> {
    let engine = engine(optimisation);
    let mut imports = HostImports::new(&engine);
    imports
        .define(
            "host",
            "touch",
            [],
            [],
            |host: &mut ContextHost, _, _, _, context, _, _| {
                let frame = context
                    .require::<Frame>()
                    .map_err(|fault| -> crate::runtime::error::ErrorSource { Box::new(fault) })?;
                frame.seen += 1;
                host.ran = true;
                Ok(())
            },
        )
        .expect("the touch import is defined once");
    imports
        .define(
            "host",
            "panic_after_touch",
            [],
            [],
            |host: &mut ContextHost, _, _, _, context, _, _| {
                let frame = context
                    .require::<Frame>()
                    .map_err(|fault| -> crate::runtime::error::ErrorSource { Box::new(fault) })?;
                frame.seen += 1;
                host.ran = true;
                panic!("host import bug");
            },
        )
        .expect("the panic import is defined once");
    let module = engine
        .compile_module(&assemble(include_str!("fixtures/context.wat")))
        .expect("the fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture imports are provided")
        .instantiate(
            ContextHost { ran: false },
            InstanceLimits::default(),
            Deadline::Unbounded,
        )
        .expect("the fixture has no start function")
}

#[test]
fn one_instance_serves_two_calls_with_different_context() {
    under_both_optimisations(|optimisation| {
        let mut instance = context_instance(optimisation);
        let mut first = HostContext::new(Frame { seen: 1 });
        instance
            .call_with_context::<(), ()>("entry", (), Deadline::Unbounded, &mut first)
            .expect("the first call has the context it names");
        assert!(instance.host_state().ran);
        assert!(!instance.has_context());
        assert_eq!(
            first
                .get_mut::<Frame>()
                .expect("the first value came back")
                .seen,
            2
        );

        instance.host_state_mut().ran = false;
        let mut second = HostContext::new(Frame { seen: 7 });
        instance
            .call_with_context::<(), ()>("entry", (), Deadline::Unbounded, &mut second)
            .expect("the second call has its own context");
        assert!(instance.host_state().ran);
        assert!(!instance.has_context());
        assert_eq!(
            second
                .get_mut::<Frame>()
                .expect("the second value came back")
                .seen,
            8
        );
        assert_eq!(
            first
                .get_mut::<Frame>()
                .expect("the first holder is untouched")
                .seen,
            2
        );
    });
}

#[test]
fn context_comes_back_after_a_caught_host_panic() {
    under_both_optimisations(|optimisation| {
        let mut instance = context_instance(optimisation);
        let mut context = HostContext::new(Frame { seen: 3 });
        let panicked = instance.call_with_context::<(), ()>(
            "panic_entry",
            (),
            Deadline::Unbounded,
            &mut context,
        );
        assert!(matches!(
            panicked,
            Err(RuntimeError::HostFault(HostFault::Panicked { .. }))
        ));
        assert!(!instance.has_context());
        assert_eq!(
            context
                .get_mut::<Frame>()
                .expect("the value came back")
                .seen,
            4
        );
    });
}

#[test]
fn a_missing_or_wrong_context_does_not_call_the_host_function() {
    under_both_optimisations(|optimisation| {
        let mut missing_instance = context_instance(optimisation);
        let missing = missing_instance.call_with_context::<(), ()>(
            "entry",
            (),
            Deadline::Unbounded,
            &mut HostContext::empty(),
        );
        assert!(matches!(
            missing,
            Err(RuntimeError::HostFault(HostFault::Context(
                ContextFault::Missing
            )))
        ));
        assert!(!missing_instance.host_state().ran);

        let mut wrong_instance = context_instance(optimisation);
        let wrong = wrong_instance.call_with_context::<(), ()>(
            "entry",
            (),
            Deadline::Unbounded,
            &mut HostContext::new(String::from("not a frame")),
        );
        assert!(matches!(
            wrong,
            Err(RuntimeError::HostFault(HostFault::Context(
                ContextFault::WrongType
            )))
        ));
        assert!(!wrong_instance.host_state().ran);
    });
}

struct TokenHost {
    ran: bool,
}

fn token_instance(optimisation: crate::runtime::NativeOptimisation) -> Instance<TokenHost> {
    let engine = engine(optimisation);
    let mut imports = HostImports::new(&engine);
    imports
        .define(
            "host",
            "accept",
            [ValType::I64],
            [],
            |host: &mut TokenHost, _, _, _, _, _, _| {
                host.ran = true;
                Ok(())
            },
        )
        .expect("the accept import is defined once");
    let module = engine
        .compile_module(&assemble(include_str!("fixtures/token_argument.wat")))
        .expect("the fixture is valid core Wasm");
    imports
        .prepare(&module)
        .expect("the fixture imports are provided")
        .instantiate(
            TokenHost { ran: false },
            InstanceLimits::default(),
            Deadline::Unbounded,
        )
        .expect("the fixture has no start function")
}

#[test]
fn a_bad_token_argument_leaves_the_instance_usable() {
    under_both_optimisations(|optimisation| {
        let mut instance = token_instance(optimisation);
        let closed = instance.insert_resource(1u32).expect("the table has room");
        instance
            .close_resource(closed)
            .expect("the value is still owned");
        let mut context = HostContext::new(1u32);
        let rejected = instance.call_with_token::<()>(
            "pass",
            closed.raw(),
            TypeId::of::<u32>(),
            Deadline::Unbounded,
            &mut context,
        );
        assert!(matches!(
            rejected,
            Err(RuntimeError::ExportArgument(
                crate::runtime::BoundaryFault::TokenNotLive
            ))
        ));
        assert!(!instance.host_state().ran);
        assert_eq!(
            *context.get_mut::<u32>().expect("the holder was not taken"),
            1
        );

        let live = instance.insert_resource(2u32).expect("the table has room");
        instance
            .call_with_token::<()>(
                "pass",
                live.raw(),
                TypeId::of::<u32>(),
                Deadline::Unbounded,
                &mut HostContext::empty(),
            )
            .expect("a later live token still enters the guest");
        assert!(instance.host_state().ran);

        instance.host_state_mut().ran = false;
        let text = instance
            .insert_resource(String::from("sprite"))
            .expect("the table has room");
        let wrong_type = instance.call_with_token::<()>(
            "pass",
            text.raw(),
            TypeId::of::<u32>(),
            Deadline::Unbounded,
            &mut HostContext::empty(),
        );
        assert!(matches!(
            wrong_type,
            Err(RuntimeError::ExportArgument(
                crate::runtime::BoundaryFault::TokenTypeMismatch
            ))
        ));
        assert!(!instance.host_state().ran);
        instance
            .call_with_token::<()>(
                "pass",
                live.raw(),
                TypeId::of::<u32>(),
                Deadline::Unbounded,
                &mut HostContext::empty(),
            )
            .expect("the wrong-type rejection did not quarantine the instance");
    });
}

#[test]
fn a_thread_bound_instance_accepts_context_that_is_not_send() {
    use crate::runtime::resource::ThreadBound;
    use std::rc::Rc;

    under_both_optimisations(|optimisation| {
        let engine = engine(optimisation);
        let mut imports = HostImports::<(), ThreadBound>::new(&engine);
        imports
            .define(
                "host",
                "touch",
                [],
                [],
                |_: &mut (), _, _, _, context, _, _| {
                    let value = context.require::<Rc<u8>>().map_err(|fault| {
                        let source: crate::runtime::error::ErrorSource = Box::new(fault);
                        source
                    })?;
                    assert_eq!(**value, 7);
                    Ok(())
                },
            )
            .expect("the touch import is defined once");
        let module = engine
            .compile_module(&assemble(
                r#"(module (import "host" "touch" (func $touch)) (func (export "entry") (call $touch)))"#,
            ))
            .expect("the fixture is valid core Wasm");
        let mut instance = imports
            .prepare(&module)
            .expect("the fixture imports are provided")
            .instantiate((), InstanceLimits::default(), Deadline::Unbounded)
            .expect("the fixture has no start function");
        let mut context = HostContext::<ThreadBound>::new(Rc::new(7u8));
        instance
            .call_with_context::<(), ()>("entry", (), Deadline::Unbounded, &mut context)
            .expect("a thread-bound instance accepts context that is not Send");
        assert_eq!(
            **context.get_mut::<Rc<u8>>().expect("the value came back"),
            7
        );
    });
}

#[test]
fn a_sendable_context_rejects_a_value_that_is_not_send() {
    use crate::runtime::context::ContextValue;
    use crate::runtime::resource::Sendable;
    use std::rc::Rc;

    // Compiles only while `Rc<u8>` cannot be sendable context. Removing the `Send` bound makes
    // this assertion ambiguous.
    trait AmbiguousIfImpl<Marker> {
        fn some_item() {}
    }
    impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
    struct WouldBeSendableContext;
    impl<T: ?Sized + ContextValue<Sendable>> AmbiguousIfImpl<WouldBeSendableContext> for T {}
    let _ = <Rc<u8> as AmbiguousIfImpl<_>>::some_item;
}
