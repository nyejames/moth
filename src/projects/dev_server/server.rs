//! Dev server runtime implementation.
//!
//! WHAT: validates CLI input, runs the initial dev build, starts the watcher/build loop,
//! and serves HTTP/SSE traffic for hot reload.

use crate::build_system::BuildProfile;
use crate::build_system::build::{ProjectBuilder, bootstrap_project_build};
use crate::build_system::create_project_modules::resolve_project_entry_root;
use crate::build_system::output::OutputOwner;
use crate::build_system::path_validation::check_if_valid_path;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::dev_server::DevServerOptions;
use crate::projects::dev_server::build_loop::{ProjectBuildExecutor, dev_server_error_messages};
use crate::projects::dev_server::state::DevServerState;
use crate::projects::dev_server::watch;
use crate::projects::settings::LANGUAGE_SOURCE_EXTENSION;
use saying::say;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct DevRuntimePaths {
    pub(super) output_dir: PathBuf,
    pub(super) watch_scope: watch::WatchScope,
}

pub fn run_dev_server(
    builder: ProjectBuilder,
    entry_path: &str,
    flags: &[Flag],
    options: DevServerOptions,
) -> Result<(), CompilerMessages> {
    let entry_target = validate_dev_entry_path(entry_path)?;
    let resolved_paths =
        resolve_dev_runtime_paths(&builder, &entry_target, flags, &options.inputs)?;
    let mut watch_scope = resolved_paths.watch_scope;

    let state = Arc::new(DevServerState::new(resolved_paths.output_dir.clone()));
    let mut executor = ProjectBuildExecutor::new(builder, options.inputs);

    let initial_build_report = crate::projects::dev_server::build_loop::run_single_build_cycle(
        &state,
        &mut executor,
        &entry_target,
        flags,
    );
    if let Some(updated_watch_scope) = initial_build_report.watch_scope.clone() {
        watch_scope = updated_watch_scope;
    }
    if initial_build_report.build_ok {
        say!(
            Green "Initial dev build succeeded. Reload broadcast to ",
            Green initial_build_report.clients_notified,
            Green " clients."
        );
    } else {
        say!(
            Yellow "Initial dev build failed. Reload broadcast to ",
            Yellow initial_build_report.clients_notified,
            Yellow " clients."
        );
    }
    #[cfg(feature = "timers")]
    if let Some(snapshot) = &initial_build_report.timing_snapshot {
        crate::timing::render_command_timing_summary(
            snapshot,
            crate::timing::TimingCommandKind::Dev,
            initial_build_report.build_ok,
        );
    }

    let bind_addr = format!("{}:{}", options.host, options.port);
    let listener = TcpListener::bind(&bind_addr).map_err(|error| {
        dev_server_error_messages(
            &entry_target,
            format!("Failed to start dev server on {bind_addr}: {error}"),
        )
    })?;

    let host_display = if options.host == "127.0.0.1" {
        "localhost"
    } else {
        options.host.as_str()
    };
    say!(Bold "Dev server listening at:");
    say!(
        Green "http://",
        Green host_display,
        Green ":",
        Green options.port
    );

    let watch_state = Arc::clone(&state);
    let watch_executor =
        Box::new(executor) as Box<dyn crate::projects::dev_server::build_loop::DevBuildExecutor>;
    let watch_entry_file = entry_target.clone();
    let watch_flags = flags.to_vec();
    let poll_interval = Duration::from_millis(options.poll_interval_ms);

    // Watch/rebuild runs independently from request handling so SSE clients do not block rebuilds.
    thread::spawn(move || {
        crate::projects::dev_server::build_loop::run_watch_build_loop(
            watch_state,
            watch_executor,
            watch_entry_file,
            watch_flags,
            watch_scope,
            poll_interval,
        );
    });

    // The server keeps the accept loop simple: each connection is handled on a small worker thread.
    for stream_result in listener.incoming() {
        match stream_result {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(error) =
                        crate::projects::dev_server::http::handle_connection(stream, state)
                    {
                        say!(
                            Yellow "Dev server request handling warning: ",
                            Yellow error.to_string()
                        );
                    }
                });
            }
            Err(error) => {
                say!(
                    Yellow "Dev server connection accept warning: ",
                    Yellow error.to_string()
                );
            }
        }
    }

    Ok(())
}

pub(crate) fn resolve_dev_runtime_paths(
    builder: &ProjectBuilder,
    entry_target: &Path,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
) -> Result<DevRuntimePaths, CompilerMessages> {
    if !entry_target.is_dir() {
        let output_dir = entry_target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("dev");
        return Ok(DevRuntimePaths {
            watch_scope: watch::WatchScope::derive(entry_target, None, &output_dir),
            output_dir,
        });
    }

    let bootstrap =
        bootstrap_project_build(builder, entry_target.to_path_buf(), build_config_inputs)?;
    let Some(validated_output_settings) = bootstrap.validated_directory_output_settings else {
        return Err(dev_server_error_messages(
            entry_target,
            "Directory output settings were not available after bootstrap validation.",
        ));
    };
    let owner = OutputOwner {
        builder: builder.backend.builder_kind(),
        profile: BuildProfile::from_flags(flags),
    };
    let plan = validated_output_settings.select(
        bootstrap.config.entry_dir.clone(),
        resolve_project_entry_root(&bootstrap.config),
        owner,
    );
    let output_dir = plan.output_root.canonicalize().unwrap_or(plan.output_root);
    Ok(DevRuntimePaths {
        watch_scope: watch::WatchScope::derive(entry_target, Some(&bootstrap.config), &output_dir),
        output_dir,
    })
}

pub(crate) fn validate_dev_entry_path(entry_path: &str) -> Result<PathBuf, CompilerMessages> {
    let resolved_path = if entry_path.trim().is_empty() {
        std::env::current_dir().map_err(|error| {
            dev_server_error_messages(
                Path::new("."),
                format!("Failed to resolve current directory: {error}"),
            )
        })?
    } else {
        let mut string_table = StringTable::new();
        check_if_valid_path(entry_path, &mut string_table).map_err(|error| {
            CompilerMessages::from_error(error.with_error_type(ErrorType::DevServer), string_table)
        })?
    };

    if resolved_path.is_dir() {
        return match resolved_path.canonicalize() {
            Ok(canonical_path) => Ok(canonical_path),
            Err(error) => {
                let mut string_table = StringTable::new();
                let error = CompilerError::file_error(
                    &resolved_path,
                    format!("Failed to canonicalize dev entry path: {error}"),
                    &mut string_table,
                )
                .with_error_type(ErrorType::DevServer);

                Err(CompilerMessages::from_error(error, string_table))
            }
        };
    }

    if !resolved_path.is_file() {
        return Err(dev_server_error_messages(
            &resolved_path,
            "Dev server entry path must resolve to either a project directory or a .moth file.",
        ));
    }

    let is_moth_file = resolved_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == LANGUAGE_SOURCE_EXTENSION);
    if !is_moth_file {
        return Err(dev_server_error_messages(
            &resolved_path,
            "Dev server currently only supports .moth file entries.",
        ));
    }

    match resolved_path.canonicalize() {
        Ok(canonical_path) => Ok(canonical_path),
        Err(error) => {
            let mut string_table = StringTable::new();
            let error = CompilerError::file_error(
                &resolved_path,
                format!("Failed to canonicalize dev entry path: {error}"),
                &mut string_table,
            )
            .with_error_type(ErrorType::DevServer);

            Err(CompilerMessages::from_error(error, string_table))
        }
    }
}
