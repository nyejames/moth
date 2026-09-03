//! Dev server entry point and module map.
//!
//! WHAT: exposes the public `run_dev_server` entry point plus the small CLI option contract.
//! WHY: the real implementation is split by runtime concern so hot reload behavior stays
//! inspectable: `server` owns startup, `build_loop` owns rebuild scheduling, `watch` owns
//! filesystem change detection, `http` owns request routing, `sse` owns reload streams, and
//! `error_page` owns rendered failure pages.

mod build_loop;
mod dev_client;
mod error_page;
mod http;
mod server;
mod sse;
mod state;
mod static_files;
mod watch;

pub use server::run_dev_server;

use crate::compiler_frontend::build_config::BuildConfigInputSet;

// The validation/path helpers remain production-private; integration-style unit tests exercise
// them through this narrow re-export instead of making the runtime API wider.
#[cfg(test)]
pub(crate) use server::{resolve_dev_runtime_paths, validate_dev_entry_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevServerOptions {
    pub host: String,
    pub port: u16,
    pub poll_interval_ms: u64,
    /// The explicit typed build-config inputs this dev command started with.
    ///
    /// They ride the shared bootstrap state on every initial and watch-triggered rebuild so
    /// the server keeps one set of explicit inputs for its whole lifetime.
    pub(crate) inputs: BuildConfigInputSet,
}

impl Default for DevServerOptions {
    fn default() -> Self {
        Self {
            host: String::from("127.0.0.1"),
            port: 6342,
            poll_interval_ms: 300,
            inputs: BuildConfigInputSet::new(),
        }
    }
}

#[cfg(test)]
#[path = "tests/mod_tests.rs"]
mod tests;
