//! Tests for SSE generation announcements and disconnected-client pruning.

use super::{broadcast_generation, format_generation_event, handle_sse_connection_with_timeouts};
use crate::compiler_tests::test_support::{
    WORKER_COMPLETION_DEADLINE, await_worker_completion, surface_thread_panic,
};
use crate::projects::dev_server::state::{DevServerState, OutputCapturePoint, SseClient};
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
fn generation_event_uses_expected_sse_format() {
    let formatted = format_generation_event(42);
    assert_eq!(formatted, "event: generation\ndata: 42\n\n");
}

#[test]
fn broadcast_prunes_disconnected_clients() {
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));

    let (sender_ok, receiver_ok) = mpsc::sync_channel::<()>(1);
    let client_id_ok = state.next_client_id.fetch_add(1, Ordering::Relaxed);
    state
        .clients
        .lock()
        .expect("clients mutex should not be poisoned")
        .push(SseClient {
            id: client_id_ok,
            sender: sender_ok,
        });

    let (sender_dead, receiver_dead) = mpsc::sync_channel::<()>(1);
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

    let notified = broadcast_generation(&state);
    assert_eq!(notified, 1);

    let remaining = state
        .clients
        .lock()
        .expect("clients mutex should not be poisoned");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, client_id_ok);
    receiver_ok
        .recv()
        .expect("connected client should receive a wake");
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

    let notified = broadcast_generation(&state);
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

/// Read SSE bytes until `count` generation events arrive, returning their data values.
pub(in crate::projects::dev_server) fn read_generations(
    client: &mut TcpStream,
    count: usize,
) -> Vec<String> {
    client
        .set_read_timeout(Some(WORKER_COMPLETION_DEADLINE))
        .expect("bound SSE read");
    let mut received = String::new();
    loop {
        let generations = received
            .split("event: generation\ndata: ")
            .skip(1)
            .filter_map(|event| event.split_once("\n\n").map(|(data, _)| data.to_owned()))
            .collect::<Vec<_>>();
        if generations.len() >= count {
            return generations;
        }
        let mut buffer = [0_u8; 256];
        let bytes_read = client.read(&mut buffer).unwrap_or_else(|error| {
            panic!("expected {count} generation events; received {received:?}: {error}")
        });
        assert!(bytes_read > 0, "SSE stream closed after {received:?}");
        received.push_str(std::str::from_utf8(&buffer[..bytes_read]).expect("SSE text"));
    }
}

/// A served SSE connection: the client socket, the handler thread and its exit signal.
pub(in crate::projects::dev_server) struct SseConnection {
    pub(in crate::projects::dev_server) client: TcpStream,
    server_thread: thread::JoinHandle<()>,
    done: mpsc::Receiver<()>,
}

/// Serve one SSE connection with short timers, or `None` when loopback binding is denied.
pub(in crate::projects::dev_server) fn connect_sse(
    state: &Arc<DevServerState>,
) -> Option<SseConnection> {
    let listener = bind_loopback_listener()?;
    let address = listener
        .local_addr()
        .expect("listener should report bound address");
    let server_state = Arc::clone(state);
    let (done_sender, done) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("should accept client");
        handle_sse_connection_with_timeouts(
            stream,
            server_state,
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .expect("sse handler should exit cleanly");
        done_sender.send(()).expect("signal handler exit");
    });
    let client = TcpStream::connect(address).expect("client should connect");
    Some(SseConnection {
        client,
        server_thread,
        done,
    })
}

pub(in crate::projects::dev_server) fn close_sse(
    state: &Arc<DevServerState>,
    connection: SseConnection,
) {
    connection
        .client
        .shutdown(Shutdown::Both)
        .expect("client should close the SSE connection");
    drop(connection.client);
    // A wake forces a write, so the handler observes the closed peer without waiting on timers.
    broadcast_generation(state);
    await_worker_completion(
        "sse server",
        &connection.done,
        connection.server_thread,
        SSE_PRUNE_DEADLINE,
    );
}

fn publish_generation(state: &Arc<DevServerState>, version: u64) {
    state
        .build_state
        .lock()
        .expect("build state")
        .last_build_version = version;
    broadcast_generation(state);
}

#[test]
fn reconnecting_clients_receive_the_current_generation_without_another_publication() {
    // WHAT: connect at generation 4, disconnect, publish 5 with no client registered, reconnect,
    //       then reconnect again without publishing.
    // WHY: a page that missed the broadcast must catch up on reconnection, while reconnecting
    //      to the same generation must announce that unchanged value rather than a new reload.
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));
    state
        .build_state
        .lock()
        .expect("build state")
        .last_build_version = 4;

    let Some(mut connection) = connect_sse(&state) else {
        return;
    };
    assert_eq!(read_generations(&mut connection.client, 1), ["4"]);
    close_sse(&state, connection);
    assert!(state.clients.lock().expect("clients").is_empty());

    publish_generation(&state, 5);
    for _ in 0..2 {
        let mut connection = connect_sse(&state).expect("loopback stays available");
        assert_eq!(read_generations(&mut connection.client, 1), ["5"]);
        close_sse(&state, connection);
    }
}

#[test]
fn publication_between_registration_and_generation_read_cannot_be_missed() {
    // WHAT: pause the handler after it registers but before it reads the published generation,
    //       publish and broadcast generation 2, then release it.
    // WHY: registration precedes the read, so the publication is either the generation read or
    //      a wake for the registered client. Either way the client observes generation 2.
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));
    state
        .build_state
        .lock()
        .expect("build state")
        .last_build_version = 1;
    let (registered_sender, registered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel::<()>();
    *state.capture_hook.lock().expect("capture hook") = Some(Box::new(move |point| {
        if matches!(point, OutputCapturePoint::AfterSseRegistration) {
            registered_sender.send(()).expect("signal registration");
            release_receiver
                .recv_timeout(WORKER_COMPLETION_DEADLINE)
                .expect("release generation read");
        }
    }));

    let Some(mut connection) = connect_sse(&state) else {
        return;
    };
    registered_receiver
        .recv_timeout(WORKER_COMPLETION_DEADLINE)
        .expect("handler registers before reading the generation");
    publish_generation(&state, 2);
    release_sender.send(()).expect("release handler");

    let generations = read_generations(&mut connection.client, 2);
    assert_eq!(generations, ["2", "2"]);
    *state.capture_hook.lock().expect("capture hook") = None;
    close_sse(&state, connection);
}
