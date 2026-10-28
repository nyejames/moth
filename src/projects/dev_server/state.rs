//! Shared mutable state for the dev server runtime.
//!
//! HTTP handlers, SSE broadcast logic, and the watcher/build loop coordinate through this state.

use crate::projects::routing::HtmlSiteConfig;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::SyncSender;

pub type ServedArtifactSnapshot = Arc<HashMap<PathBuf, Arc<Vec<u8>>>>;
pub type ServedHtmlEntrySnapshot = Arc<HashSet<String>>;

#[derive(Debug)]
pub struct SseClient {
    pub id: u64,
    /// Wakes the connection's writer to announce the current published generation.
    pub sender: SyncSender<()>,
}

#[derive(Debug, Clone)]
pub struct BuildState {
    pub last_build_ok: bool,
    pub last_error_html: Option<String>,
    pub last_build_version: u64,
    pub entry_page_rel: Option<PathBuf>,
    pub output_dir: PathBuf,
    /// Immutable artifact bytes published with this last-known-good build version.
    pub served_artifacts: ServedArtifactSnapshot,
    /// Validated HTML entry paths published with the matching artifact generation.
    pub served_html_entries: ServedHtmlEntrySnapshot,
    pub served_artifacts_version: u64,
    pub html_site_config: HtmlSiteConfig,
    pub last_build_messages_summary: String,
}
impl BuildState {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            last_build_ok: false,
            last_error_html: None,
            last_build_version: 0,
            entry_page_rel: None,
            output_dir,
            served_artifacts: Arc::new(HashMap::new()),
            served_html_entries: Arc::new(HashSet::new()),
            served_artifacts_version: 0,
            html_site_config: HtmlSiteConfig::default(),
            last_build_messages_summary: String::from("Initial build has not completed yet."),
        }
    }
}

/// Browser runtime failures stay invocation-local and never change compilation state.
#[derive(Debug, Default)]
pub struct RuntimeReports {
    pub build: Option<u64>,
    pub keys: HashSet<RuntimeReportKey>,
}

/// One accepted report per startup invocation, independent of the reported outcome category.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct RuntimeReportKey {
    pub build: u64,
    pub entry: String,
    pub invocation: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeReportCategory {
    EntryError,
    Assertion,
    StartupFault,
}

impl<'de> Deserialize<'de> for RuntimeReportCategory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CategoryVisitor;

        impl Visitor<'_> for CategoryVisitor {
            type Value = RuntimeReportCategory;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("entry_error, assertion or startup_fault as a JSON string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "entry_error" => Ok(RuntimeReportCategory::EntryError),
                    "assertion" => Ok(RuntimeReportCategory::Assertion),
                    "startup_fault" => Ok(RuntimeReportCategory::StartupFault),
                    _ => Err(E::unknown_variant(
                        value,
                        &["entry_error", "assertion", "startup_fault"],
                    )),
                }
            }
        }

        deserializer.deserialize_str(CategoryVisitor)
    }
}

/// Points where tests pause a reader that captures published state: an HTTP GET before and
/// inside the publication lock, and an SSE connection between registration and its first
/// generation read.
#[cfg(test)]
#[derive(Clone, Copy)]
pub enum OutputCapturePoint {
    BeforeLock,
    BeforeRead,
    AfterSseRegistration,
}

#[cfg(test)]
type OutputCaptureHook = Box<dyn Fn(OutputCapturePoint) + Send>;

pub struct DevServerState {
    /// Publication boundary: immutable artifact and metadata snapshots switch together.
    pub build_state: Mutex<BuildState>,
    pub runtime_reports: Mutex<RuntimeReports>,
    pub clients: Mutex<Vec<SseClient>>,
    // IDs are monotonic so client removal stays stable even when vector indices shift.
    pub next_client_id: AtomicU64,
    #[cfg(test)]
    pub capture_hook: Mutex<Option<OutputCaptureHook>>,
}

impl DevServerState {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            build_state: Mutex::new(BuildState::new(output_dir)),
            runtime_reports: Mutex::new(RuntimeReports::default()),
            clients: Mutex::new(Vec::new()),
            next_client_id: AtomicU64::new(1),
            #[cfg(test)]
            capture_hook: Mutex::new(None),
        }
    }
}
