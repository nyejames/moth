//! Tests for SSE payload formatting and disconnected-client pruning.

use super::{broadcast_reload, format_reload_event, handle_sse_connection_with_timeouts};
use crate::compiler_tests::test_support::{
    WORKER_COMPLETION_DEADLINE, await_worker_completion, surface_thread_panic,
};
use crate::projects::dev_server::state::{DevServerState, SseClient};
use std::io::Read;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a disconnected SSE client may remain registered after a broadcast.
const SSE_PRUNE_DEADLINE: Duration = Duration::from_secs(1);

fn bind_loopback_listener() -> Option<TcpListener> {
    match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => Some(listener),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => None,
        Err(error) => panic!("should bind test listener: {error}"),
    }
}

#[test]
fn reload_event_uses_expected_sse_format() {
    let formatted = format_reload_event(42);
    assert_eq!(formatted, "event: reload\ndata: 42\n\n");
}

#[test]
fn broadcast_prunes_disconnected_clients() {
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));

    let (sender_ok, receiver_ok) = mpsc::sync_channel::<String>(1);
    let client_id_ok = state.next_client_id.fetch_add(1, Ordering::Relaxed);
    state
        .clients
        .lock()
        .expect("clients mutex should not be poisoned")
        .push(SseClient {
            id: client_id_ok,
            sender: sender_ok,
        });

    let (sender_dead, receiver_dead) = mpsc::sync_channel::<String>(1);
    drop(receiver_dead);
    let client_id_dead = state.next_client_id.fetch_add(1, Ordering::Relaxed);
    state
        .clients
        .lock()
        .expect("clients mutex should not be poisoned")
        .push(SseClient {
            id: client_id_dead,
            sender: sender_dead,
        });

    let notified = broadcast_reload(&state, 7);
    assert_eq!(notified, 1);

    let remaining = state
        .clients
        .lock()
        .expect("clients mutex should not be poisoned");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, client_id_ok);
    assert_eq!(
        receiver_ok
            .recv()
            .expect("connected client should receive event"),
        "event: reload\ndata: 7\n\n"
    );
}

#[test]
fn loopback_disconnect_prunes_sse_client_promptly() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let address = listener
        .local_addr()
        .expect("listener should report bound address");
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));
    let (done_sender, done_receiver) = mpsc::channel();

    let server_state = Arc::clone(&state);
    let server_thread = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("should accept client");
        handle_sse_connection_with_timeouts(
            stream,
            server_state,
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .expect("sse handler should exit cleanly");
        done_sender
            .send(())
            .expect("server thread should signal completion");
    });

    let mut client = TcpStream::connect(address).expect("client should connect");
    let mut buffer = [0_u8; 256];
    let bytes_read = client
        .read(&mut buffer)
        .expect("client should read initial sse headers");
    assert!(bytes_read > 0);

    // The handler registers the client after writing the SSE headers, so reading them does not
    // prove registration and there is no in-process signal to wait on.
    if let Err(observed) = wait_for_registered_client_count(&state, 1) {
        surface_thread_panic("sse server", server_thread);
        panic!("the connected client should register exactly once; observed {observed} clients");
    }

    client
        .shutdown(Shutdown::Both)
        .expect("client should close the SSE connection");
    drop(client);

    let notified = broadcast_reload(&state, 3);
    assert_eq!(notified, 1);
    // "Promptly" is the contract this test owns: the handler's keep-alive interval is 50ms, so a
    // broadcast to a disconnected client must prune it inside this bound rather than waiting for
    // some later event.
    await_worker_completion(
        "sse server",
        &done_receiver,
        server_thread,
        SSE_PRUNE_DEADLINE,
    );
    assert!(
        state
            .clients
            .lock()
            .expect("clients mutex should not be poisoned")
            .is_empty()
    );
}

/// Wait for the SSE registry to hold `expected` clients, reporting the last count on failure.
///
/// WHAT: polls the registry until the count matches, bounded by the shared worker deadline.
/// WHY: registration happens inside the handler thread with no observable signal. The deadline
///      is deadlock protection only; a test that continued on a wrong count would exercise the
///      wrong precondition and still report a pruning failure.
fn wait_for_registered_client_count(state: &DevServerState, expected: usize) -> Result<(), usize> {
    let deadline = Instant::now() + WORKER_COMPLETION_DEADLINE;
    loop {
        let observed = state
            .clients
            .lock()
            .expect("clients mutex should not be poisoned")
            .len();
        if observed == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(observed);
        }
        thread::sleep(Duration::from_millis(1));
    }
}
