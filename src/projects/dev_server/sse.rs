//! Server-Sent Events helpers for hot reload.
//!
//! Each connection announces the published build generation when it opens and again whenever a
//! publication wakes it. The browser compares that generation with its page's own and reloads
//! only on a mismatch, so a page that missed a broadcast catches up on its next (re)connection.
//! Failed clients are pruned from shared state during broadcast.

use crate::projects::dev_server::state::{DevServerState, SseClient};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(5);

pub fn format_generation_event(version: u64) -> String {
    format!("event: generation\ndata: {version}\n\n")
}

/// Wake every connected client to announce the newly published generation.
///
/// WHAT: the wake carries no version. Each writer reads the current generation when it sends.
/// WHY: a client with a wake already pending loses nothing when this one is coalesced, because
///      the pending wake also reads the latest generation rather than an older queued value.
pub fn broadcast_generation(state: &Arc<DevServerState>) -> usize {
    let mut clients = match state.clients.lock() {
        Ok(guard) => guard,
        Err(_) => return 0,
    };

    let mut notified_count = 0usize;
    // Broadcast and prune disconnected clients in one pass. Full queues already have a wake
    // pending, so they count as notified without stacking redundant work.
    clients.retain(|client| match client.sender.try_send(()) {
        Ok(()) | Err(TrySendError::Full(())) => {
            notified_count += 1;
            true
        }
        Err(TrySendError::Disconnected(())) => false,
    });

    notified_count
}

/// The generation the build loop last published, or `None` once the lock is poisoned.
fn published_generation(state: &DevServerState) -> Option<u64> {
    let build_state = state.build_state.lock().ok()?;
    Some(build_state.last_build_version)
}

fn register_client(state: &Arc<DevServerState>, sender: SyncSender<()>) -> Option<u64> {
    let client_id = state.next_client_id.fetch_add(1, Ordering::Relaxed);
    let mut clients = state.clients.lock().ok()?;
    clients.push(SseClient {
        id: client_id,
        sender,
    });
    Some(client_id)
}

pub fn remove_client(state: &Arc<DevServerState>, client_id: u64) {
    if let Ok(mut clients) = state.clients.lock() {
        clients.retain(|client| client.id != client_id);
    }
}

pub fn handle_sse_connection(stream: TcpStream, state: Arc<DevServerState>) -> io::Result<()> {
    handle_sse_connection_with_timeouts(stream, state, WRITE_TIMEOUT, KEEP_ALIVE_INTERVAL)
}

pub(super) fn handle_sse_connection_with_timeouts(
    mut stream: TcpStream,
    state: Arc<DevServerState>,
    write_timeout: Duration,
    keep_alive_interval: Duration,
) -> io::Result<()> {
    stream.set_write_timeout(Some(write_timeout))?;

    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "Access-Control-Allow-Origin: *\r\n\r\n"
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(b": connected\n\n")?;
    stream.flush()?;

    let (sender, receiver) = mpsc::sync_channel::<()>(1);
    let Some(client_id) = register_client(&state, sender) else {
        return Err(io::Error::other(
            "Failed to register SSE client due to state lock poisoning",
        ));
    };

    // WHAT: read the published generation only after registration.
    // WHY: a publication after this read wakes the registered client, and one before it is the
    //      generation read here, so no generation can fall between the snapshot and subscription.
    #[cfg(test)]
    if let Some(hook) = state.capture_hook.lock().expect("read hook lock").as_ref() {
        hook(crate::projects::dev_server::state::OutputCapturePoint::AfterSseRegistration);
    }
    let mut announce_generation = true;
    loop {
        if announce_generation {
            // A poisoned publication lock ends the stream; the browser reconnects after recovery.
            let Some(version) = published_generation(&state) else {
                break;
            };
            let event = format_generation_event(version);
            if stream.write_all(event.as_bytes()).is_err() || stream.flush().is_err() {
                break;
            }
        }

        announce_generation = match receiver.recv_timeout(keep_alive_interval) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout) => {
                // Keep-alive comments prevent proxies and browsers from considering the stream idle.
                if stream.write_all(b": keep-alive\n\n").is_err() || stream.flush().is_err() {
                    break;
                }
                false
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
    }

    remove_client(&state, client_id);
    Ok(())
}

#[cfg(test)]
#[path = "tests/sse_tests.rs"]
pub(super) mod tests;
