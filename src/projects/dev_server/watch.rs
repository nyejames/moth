//! Filesystem watch scope and change-notification helpers for the dev server.
//!
//! WHAT: derives the dev-server watch scope, starts either a `notify` watcher or a polling
//! fallback, and exposes a revision-based wait API to the build loop.
//! WHY: rebuild coordination should react to concrete source changes without re-scanning broad
//! unrelated trees on every loop iteration.

use crate::build_system::create_project_modules::resolve_project_entry_root;
use crate::projects::settings::{CONFIG_FILE_NAME, Config};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use saying::say;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

const WATCH_DEBOUNCE_WINDOW: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchScope {
    pub output_dir: PathBuf,
    pub targets: Vec<WatchTarget>,
}

impl WatchScope {
    pub fn derive(entry_target: &Path, config: Option<&Config>, output_dir: &Path) -> Self {
        let mut scope = Self {
            output_dir: canonical_if_exists(output_dir),
            targets: Vec::new(),
        };

        if entry_target.is_dir() {
            let Some(config) = config else {
                scope.watch_directory_or_parent(entry_target);
                return scope;
            };
            let project_root = canonical_if_exists(&config.entry_dir);
            scope.watch_exact_file_or_parent(&project_root.join(CONFIG_FILE_NAME));
            scope.watch_directory_or_parent(&resolve_project_entry_root(config));
            return scope;
        }

        let parent = entry_target
            .parent()
            .map(canonical_if_exists)
            .unwrap_or_else(|| PathBuf::from("."));
        scope.push_target(WatchTarget {
            watch_path: parent,
            interest_path: None,
            recursive: true,
        });
        scope
    }

    pub fn watches_path(&self, path: &Path) -> bool {
        if should_ignore_path(path, &self.output_dir) {
            return false;
        }

        self.targets.iter().any(|target| target.matches(path))
    }

    fn watch_exact_file_or_parent(&mut self, target_path: &Path) {
        if target_path.is_file() {
            let canonical = canonical_if_exists(target_path);
            self.push_target(WatchTarget {
                watch_path: canonical.clone(),
                interest_path: Some(canonical),
                recursive: false,
            });
            return;
        }

        let watch_path = target_path
            .parent()
            .map(canonical_if_exists)
            .unwrap_or_else(|| PathBuf::from("."));
        self.push_target(WatchTarget {
            watch_path,
            interest_path: Some(target_path.to_path_buf()),
            recursive: false,
        });
    }

    fn watch_directory_or_parent(&mut self, target_path: &Path) {
        if target_path.is_dir() {
            self.push_target(WatchTarget {
                watch_path: canonical_if_exists(target_path),
                interest_path: None,
                recursive: true,
            });
            return;
        }

        let watch_path = target_path
            .parent()
            .map(canonical_if_exists)
            .unwrap_or_else(|| PathBuf::from("."));
        self.push_target(WatchTarget {
            watch_path,
            interest_path: Some(target_path.to_path_buf()),
            recursive: false,
        });
    }

    fn push_target(&mut self, target: WatchTarget) {
        if self.targets.contains(&target) {
            return;
        }
        self.targets.push(target);
        self.targets.sort_by(|left, right| {
            left.watch_path
                .cmp(&right.watch_path)
                .then_with(|| left.interest_path.cmp(&right.interest_path))
                .then_with(|| left.recursive.cmp(&right.recursive))
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchTarget {
    pub watch_path: PathBuf,
    pub interest_path: Option<PathBuf>,
    pub recursive: bool,
}

impl WatchTarget {
    fn matches(&self, path: &Path) -> bool {
        if let Some(interest_path) = &self.interest_path {
            return path == interest_path;
        }

        if self.recursive {
            return path.starts_with(&self.watch_path);
        }

        path == self.watch_path || path.parent() == Some(self.watch_path.as_path())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFingerprint {
    pub modified: SystemTime,
    pub len: u64,
}

pub struct WatchSession {
    scope: WatchScope,
    revision: Arc<AtomicU64>,
    receiver: Receiver<()>,
    stop_signal: Arc<AtomicBool>,
    backend: WatchBackend,
}

enum WatchBackend {
    Notify {
        _watcher: RecommendedWatcher,
    },
    Polling {
        join_handle: Option<JoinHandle<()>>,
    },
    #[cfg(test)]
    Manual,
}

impl WatchSession {
    pub fn start(scope: WatchScope, poll_interval: Duration) -> Self {
        let revision = Arc::new(AtomicU64::new(0));
        let (sender, receiver) = mpsc::sync_channel::<()>(1);
        let stop_signal = Arc::new(AtomicBool::new(false));

        let backend = match start_notify_backend(
            &scope,
            Arc::clone(&revision),
            sender.clone(),
            Arc::clone(&stop_signal),
        ) {
            Ok(watcher) => WatchBackend::Notify { _watcher: watcher },
            Err(error) => {
                say!(
                    Yellow "Dev server watch warning: notify backend unavailable, falling back to polling: ",
                    Yellow error.to_string()
                );
                let join_handle = start_polling_backend(
                    scope.clone(),
                    Arc::clone(&revision),
                    sender,
                    Arc::clone(&stop_signal),
                    poll_interval,
                );
                WatchBackend::Polling {
                    join_handle: Some(join_handle),
                }
            }
        };

        Self {
            scope,
            revision,
            receiver,
            stop_signal,
            backend,
        }
    }

    pub fn current_revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    pub fn scope(&self) -> &WatchScope {
        &self.scope
    }

    pub fn wait_for_stable_change(&self, last_seen_revision: u64) -> io::Result<u64> {
        if self.current_revision() <= last_seen_revision {
            loop {
                self.receiver.recv().map_err(|_| {
                    io::Error::other("Dev server watch session closed while waiting for changes")
                })?;
                if self.current_revision() > last_seen_revision {
                    break;
                }
            }
        }

        loop {
            match self.receiver.recv_timeout(WATCH_DEBOUNCE_WINDOW) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    return Ok(self.current_revision());
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn manual(scope: WatchScope) -> (Self, ManualWatchTrigger) {
        let revision = Arc::new(AtomicU64::new(0));
        let (sender, receiver) = mpsc::sync_channel::<()>(1);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let session = Self {
            scope,
            revision: Arc::clone(&revision),
            receiver,
            stop_signal,
            backend: WatchBackend::Manual,
        };
        let trigger = ManualWatchTrigger { revision, sender };
        (session, trigger)
    }
}

impl Drop for WatchSession {
    fn drop(&mut self) {
        self.stop_signal.store(true, Ordering::Relaxed);
        if let WatchBackend::Polling { join_handle } = &mut self.backend
            && let Some(join_handle) = join_handle.take()
            && join_handle.join().is_err()
        {
            // A polling worker that panicked stopped watching for changes. Discarding the join
            // result would leave the dev server reporting a live watch it no longer has.
            say!(Yellow "Dev server watch warning: the polling worker panicked and stopped watching.");
        }
    }
}

#[cfg(test)]
pub(crate) struct ManualWatchTrigger {
    revision: Arc<AtomicU64>,
    sender: SyncSender<()>,
}

#[cfg(test)]
impl ManualWatchTrigger {
    pub(crate) fn notify_change(&self) {
        signal_change(&self.revision, &self.sender);
    }
}

fn start_notify_backend(
    scope: &WatchScope,
    revision: Arc<AtomicU64>,
    sender: SyncSender<()>,
    stop_signal: Arc<AtomicBool>,
) -> notify::Result<RecommendedWatcher> {
    let callback_scope = scope.clone();
    let callback_revision = Arc::clone(&revision);
    let callback_sender = sender;
    let callback_stop_signal = stop_signal;

    let mut watcher = notify::recommended_watcher(move |event_result: notify::Result<Event>| {
        if callback_stop_signal.load(Ordering::Relaxed) {
            return;
        }

        let Ok(event) = event_result else {
            return;
        };
        if !is_actionable_event(&event) {
            return;
        }
        if event
            .paths
            .iter()
            .any(|path| callback_scope.watches_path(path))
        {
            signal_change(&callback_revision, &callback_sender);
        }
    })?;

    for target in &scope.targets {
        let recursive_mode = if target.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher.watch(&target.watch_path, recursive_mode)?;
    }

    Ok(watcher)
}

fn start_polling_backend(
    scope: WatchScope,
    revision: Arc<AtomicU64>,
    sender: SyncSender<()>,
    stop_signal: Arc<AtomicBool>,
    poll_interval: Duration,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut known_fingerprints = match collect_fingerprints(&scope) {
            Ok(fingerprints) => fingerprints,
            Err(error) => {
                say!(
                    Yellow "Dev server watch warning: failed to collect initial polling fingerprints: ",
                    Yellow error.to_string()
                );
                HashMap::new()
            }
        };

        while !stop_signal.load(Ordering::Relaxed) {
            thread::sleep(poll_interval);

            let current_fingerprints = match collect_fingerprints(&scope) {
                Ok(fingerprints) => fingerprints,
                Err(error) => {
                    say!(
                        Yellow "Dev server watch warning: polling scan failed: ",
                        Yellow error.to_string()
                    );
                    continue;
                }
            };

            if detect_changes(&known_fingerprints, &current_fingerprints) {
                known_fingerprints = current_fingerprints;
                signal_change(&revision, &sender);
            }
        }
    })
}

fn signal_change(revision: &Arc<AtomicU64>, sender: &SyncSender<()>) {
    revision.fetch_add(1, Ordering::Relaxed);
    let _ = sender.try_send(());
}

fn is_actionable_event(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Builds a [`FileFingerprint`] from a modification-time result and length.
///
/// WHAT: converts `metadata.modified()` into a fingerprint, propagating timestamp-read failures
/// instead of silently falling back to the Unix epoch.
/// WHY: an epoch sentinel could make a failed timestamp read look identical to a real file whose
/// modification time is genuinely the epoch, hiding same-length edits or read errors. Passing the
/// `modified` result in (rather than `&fs::Metadata`) lets tests inject a synthetic failure
/// without platform-specific tricks that coerce a real file's metadata into failing.
fn fingerprint_from_modified(
    modified: io::Result<SystemTime>,
    len: u64,
    path: &Path,
) -> io::Result<FileFingerprint> {
    Ok(FileFingerprint {
        modified: modified.map_err(|error| fingerprint_timestamp_error(path, error))?,
        len,
    })
}

/// Wraps a modification-time read failure with the affected path while preserving the underlying
/// [`io::ErrorKind`].
fn fingerprint_timestamp_error(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "dev-server watch: failed to read modification time for {}: {error}",
            path.display()
        ),
    )
}

pub fn collect_fingerprints(scope: &WatchScope) -> io::Result<HashMap<PathBuf, FileFingerprint>> {
    let mut fingerprints = HashMap::new();

    for target in &scope.targets {
        if let Some(interest_path) = &target.interest_path {
            collect_exact_path_fingerprint(interest_path, &scope.output_dir, &mut fingerprints)?;
            continue;
        }

        collect_directory_fingerprints(
            &target.watch_path,
            target.recursive,
            &scope.output_dir,
            &mut fingerprints,
        )?;
    }

    Ok(fingerprints)
}

fn collect_exact_path_fingerprint(
    target_path: &Path,
    output_dir: &Path,
    fingerprints: &mut HashMap<PathBuf, FileFingerprint>,
) -> io::Result<()> {
    if should_ignore_path(target_path, output_dir) {
        return Ok(());
    }

    let metadata = match fs::metadata(target_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    let fingerprint = fingerprint_from_modified(metadata.modified(), metadata.len(), target_path)?;
    fingerprints.insert(target_path.to_path_buf(), fingerprint);

    Ok(())
}

fn collect_directory_fingerprints(
    root: &Path,
    recursive: bool,
    output_dir: &Path,
    fingerprints: &mut HashMap<PathBuf, FileFingerprint>,
) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir_path) = stack.pop() {
        if should_ignore_path(&dir_path, output_dir) {
            continue;
        }

        let entries = fs::read_dir(&dir_path)?;
        for entry_result in entries {
            let entry = entry_result?;
            let path = entry.path();

            if should_ignore_path(&path, output_dir) {
                continue;
            }

            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                if recursive {
                    stack.push(path);
                }
                continue;
            }

            if metadata.is_file() {
                let fingerprint =
                    fingerprint_from_modified(metadata.modified(), metadata.len(), &path)?;
                fingerprints.insert(path, fingerprint);
            }
        }

        if !recursive {
            break;
        }
    }

    Ok(())
}

pub fn detect_changes(
    previous: &HashMap<PathBuf, FileFingerprint>,
    current: &HashMap<PathBuf, FileFingerprint>,
) -> bool {
    if previous.len() != current.len() {
        return true;
    }

    previous
        .iter()
        .any(|(path, previous_fingerprint)| match current.get(path) {
            Some(current_fingerprint) => current_fingerprint != previous_fingerprint,
            None => true,
        })
}

pub fn should_ignore_path(path: &Path, output_dir: &Path) -> bool {
    if path.starts_with(output_dir) {
        return true;
    }

    if path
        .components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
    {
        return true;
    }

    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(is_editor_temporary_name)
}

fn is_editor_temporary_name(name: &str) -> bool {
    name == ".DS_Store"
        || name.starts_with(".#")
        || (name.starts_with('#') && name.ends_with('#'))
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swo")
        || name.ends_with(".swx")
        || name.ends_with(".tmp")
        || name.ends_with(".temp")
}

fn canonical_if_exists(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "tests/watch_tests.rs"]
mod tests;
