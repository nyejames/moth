//! Shared mutable state for the dev server runtime.
//!
//! HTTP handlers, SSE broadcast logic, and the watcher/build loop coordinate through this state.

use crate::projects::routing::HtmlSiteConfig;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::SyncSender;

#[derive(Debug)]
pub struct SseClient {
    pub id: u64,
    pub sender: SyncSender<String>,
}

#[derive(Debug, Clone)]
pub struct BuildState {
    pub last_build_ok: bool,
    pub last_error_html: Option<String>,
    pub last_build_version: u64,
    pub entry_page_rel: Option<PathBuf>,
    pub output_dir: PathBuf,
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
            html_site_config: HtmlSiteConfig::default(),
            last_build_messages_summary: String::from("Initial build has not completed yet."),
        }
    }
}

pub struct DevServerState {
    pub build_state: Mutex<BuildState>,
    pub clients: Mutex<Vec<SseClient>>,
    // IDs are monotonic so client removal stays stable even when vector indices shift.
    pub next_client_id: AtomicU64,
}

impl DevServerState {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            build_state: Mutex::new(BuildState::new(output_dir)),
            clients: Mutex::new(Vec::new()),
            next_client_id: AtomicU64::new(1),
        }
    }
}
