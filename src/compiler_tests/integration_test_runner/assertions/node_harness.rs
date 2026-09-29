//! Owned temporary workspaces and bounded Node execution for rendered-output assertions.
//!
//! WHAT: creates the harness workspace, runs one Node script under a wall-clock deadline with
//!       bounded output capture and strict UTF-8 decoding, then reports workspace cleanup failure.
//! WHY: generated page code can loop forever. `Command::output()` would block the whole suite
//!      until the outer CI job times out, and lossy decoding would silently repair invalid harness
//!      output into something an assertion could accept. Every failure here is a harness fact, so
//!      each class carries its own identity and can never be mistaken for a semantic mismatch.

use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// The interpreter the production harness runs.
const NODE_EXECUTABLE: &str = "node";

/// The major version whose JavaScript builtins include F16 (`Math.f16round`).
///
/// Emitted F16 values call the native builtin directly; supporting the same
/// implementation the compiler emits means the runtime must be at least Node 24.
const NODE_REQUIRED_MAJOR: u32 = 24;

/// The wall-clock budget for the one-shot runtime capability probe.
const NODE_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Wall-clock budget for one Node harness process.
///
/// This is deadlock protection, not synchronization: a well-behaved page finishes in
/// milliseconds, and anything that does not is reported as a timeout rather than hanging the run.
const NODE_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the runner re-checks whether Node exited while waiting for the deadline.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Upper bound on captured bytes per output stream.
const MAX_CAPTURED_STREAM_BYTES: usize = 4 * 1024 * 1024;

/// Bounded retries for removing the workspace after the run.
///
/// Windows can briefly keep a just-exited child's handles open, which is a documented OS cleanup
/// race rather than a harness defect. The final attempt still reports failure.
const WORKSPACE_REMOVAL_ATTEMPTS: usize = 6;
const WORKSPACE_REMOVAL_BASE_DELAY: Duration = Duration::from_millis(8);

/// A rendered-output harness failure with a stable boundary identity.
///
/// `message` is the human-readable description shown to the operator; `kind` names the boundary
/// so self-tests prove which harness lane rejected instead of matching prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderHarnessError {
    pub kind: RenderHarnessErrorKind,
    pub message: String,
}

/// Identifies the harness boundary that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderHarnessErrorKind {
    /// A required build artifact was absent or had the wrong file kind.
    Artifact,
    /// The owned temporary workspace could not be created or populated.
    Workspace,
    /// Node could not be started.
    Spawn,
    /// Node did not exit within the harness deadline and was killed.
    Timeout,
    /// Node exited with a failing status.
    ExitStatus,
    /// Captured output was not valid UTF-8, exceeded the capture bound, or could not be read.
    OutputDecoding,
    /// Captured output did not satisfy the harness event protocol.
    OutputProtocol,
    /// The page contained a `<script>` shape the harness does not support.
    ScriptShape,
    /// The workspace could not be removed after the run.
    Cleanup,
}

impl RenderHarnessError {
    pub(super) fn artifact(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::Artifact,
            message,
        }
    }

    pub(super) fn workspace(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::Workspace,
            message,
        }
    }

    pub(super) fn spawn(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::Spawn,
            message,
        }
    }

    pub(super) fn timeout(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::Timeout,
            message,
        }
    }

    pub(super) fn exit_status(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::ExitStatus,
            message,
        }
    }

    pub(super) fn output_decoding(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::OutputDecoding,
            message,
        }
    }

    pub(super) fn output_protocol(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::OutputProtocol,
            message,
        }
    }

    pub(super) fn script_shape(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::ScriptShape,
            message,
        }
    }

    pub(super) fn cleanup(message: String) -> Self {
        Self {
            kind: RenderHarnessErrorKind::Cleanup,
            message,
        }
    }

    /// Records a cleanup failure that happened while another failure was already being reported.
    ///
    /// The original boundary stays the failure's identity, because it explains why the case
    /// failed; the cleanup failure is appended so a leaked workspace is still visible.
    fn with_trailing_cleanup_failure(mut self, cleanup: &Self) -> Self {
        self.message
            .push_str(&format!("\nAdditionally, {}", cleanup.message));
        self
    }
}

impl fmt::Display for RenderHarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

/// One owned temporary directory holding the files a Node harness run needs.
///
/// The directory is removed by `close`, which reports removal failure, so a leaked workspace
/// cannot silently change a later run.
pub(crate) struct HarnessWorkspace {
    directory: TempDir,
}

impl HarnessWorkspace {
    pub(crate) fn create() -> Result<Self, RenderHarnessError> {
        let directory = tempfile::Builder::new()
            .prefix("moth_render_harness_")
            .tempdir()
            .map_err(|error| {
                RenderHarnessError::workspace(format!(
                    "rendered_output: failed to create the Node harness workspace: {error}"
                ))
            })?;

        Ok(Self { directory })
    }

    pub(crate) fn path(&self) -> &Path {
        self.directory.path()
    }

    /// Writes one file directly inside the workspace and returns its path.
    pub(crate) fn write(
        &self,
        file_name: &str,
        contents: impl AsRef<[u8]>,
    ) -> Result<PathBuf, RenderHarnessError> {
        let path = self.directory.path().join(file_name);
        std::fs::write(&path, contents).map_err(|error| {
            RenderHarnessError::workspace(format!(
                "rendered_output: failed to write harness file '{}': {error}",
                path.display()
            ))
        })?;

        Ok(path)
    }

    /// Writes one validated relative file path, creating its parent directories inside the
    /// workspace as needed.
    ///
    /// Artifact paths have already passed `BuiltArtifactIndex`'s output-path validation. This
    /// second boundary check prevents a future caller from using this filesystem helper with an
    /// absolute path or traversal component.
    pub(crate) fn write_relative(
        &self,
        relative_path: &Path,
        contents: impl AsRef<[u8]>,
    ) -> Result<PathBuf, RenderHarnessError> {
        // Backslash is checked against the path text (not only components) because Windows
        // treats it as a separator and would otherwise split it away before this guard runs.
        if relative_path
            .as_os_str()
            .as_encoded_bytes()
            .contains(&b'\\')
        {
            return Err(RenderHarnessError::workspace(format!(
                "rendered_output: refusing to write an unsafe relative harness path '{relative_path:?}'."
            )));
        }
        let safe_components = relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if relative_path.as_os_str().is_empty() || !safe_components {
            return Err(RenderHarnessError::workspace(format!(
                "rendered_output: refusing to write an unsafe relative harness path '{relative_path:?}'."
            )));
        }

        let path = self.directory.path().join(relative_path);
        let parent = path.parent().ok_or_else(|| {
            RenderHarnessError::workspace(format!(
                "rendered_output: relative harness path '{relative_path:?}' has no parent directory."
            ))
        })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            RenderHarnessError::workspace(format!(
                "rendered_output: failed to create harness artifact directory '{}': {error}",
                parent.display()
            ))
        })?;
        std::fs::write(&path, contents).map_err(|error| {
            RenderHarnessError::workspace(format!(
                "rendered_output: failed to write harness artifact '{}': {error}",
                path.display()
            ))
        })?;

        Ok(path)
    }

    /// Removes the workspace, reporting failure instead of discarding it.
    pub(crate) fn close(self) -> Result<(), RenderHarnessError> {
        let path = self.directory.keep();
        let mut last_error = None;

        for attempt in 0..WORKSPACE_REMOVAL_ATTEMPTS {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < WORKSPACE_REMOVAL_ATTEMPTS {
                        std::thread::sleep(WORKSPACE_REMOVAL_BASE_DELAY * (attempt as u32 + 1));
                    }
                }
            }
        }

        Err(RenderHarnessError::cleanup(format!(
            "rendered_output: failed to remove the Node harness workspace '{}': {}",
            path.display(),
            last_error.map_or_else(
                || "unknown removal failure".to_owned(),
                |error| error.to_string()
            )
        )))
    }
}

/// Runs `body` against a fresh workspace and always reports the workspace cleanup outcome.
///
/// A cleanup failure fails the run on its own when the body succeeded, and is appended to the
/// body's failure otherwise, so a leaked workspace is never invisible.
pub(crate) fn with_harness_workspace<T>(
    body: impl FnOnce(&HarnessWorkspace) -> Result<T, RenderHarnessError>,
) -> Result<T, RenderHarnessError> {
    let workspace = HarnessWorkspace::create()?;
    let outcome = body(&workspace);
    let cleanup = workspace.close();

    match (outcome, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => {
            Err(error.with_trailing_cleanup_failure(&cleanup_error))
        }
    }
}

/// Standard output of one completed Node harness process.
pub(crate) struct NodeRunOutput {
    pub stdout: String,
}

/// Verifies once per process that the selected Node runtime satisfies the emitted contracts.
///
/// WHAT: remembers one static probe outcome and re-reports it as an infrastructure failure.
/// WHY: the emitted JavaScript calls native F16 builtins directly, so an older runtime makes
///      every F16 fixture fail inside its page instead of stating the unmet prerequisite
///      once. Fatal by design: an unsupported runtime is a `Spawn` failure for every produced
///      page rather than something individual cases skip around.
pub(crate) fn require_compatible_node_runtime() -> Result<(), RenderHarnessError> {
    static PROBE_RESULT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

    let probe = PROBE_RESULT.get_or_init(|| probe_node_runtime(NODE_EXECUTABLE));
    probe
        .as_ref()
        .map_err(|message| RenderHarnessError::spawn(message.clone()))
        .copied()
}

fn probe_node_runtime(executable: &str) -> Result<(), String> {
    let mut child = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!("rendered_output: the Node harness could not start '{executable}': {error}")
        })?;

    // Only stdout carries the version string; a stalled probe is killed and reaped by the same
    // bounded deadline logic the page harness uses, never left behind. The capture handle is
    // joined before any result is inspected, so a report never leaves a live capture thread behind.
    if let Some(stdout_pipe) = child.stdout.take() {
        let stdout_capture = std::thread::spawn(move || capture_stream(stdout_pipe));
        let exit = wait_for_exit(&mut child, NODE_VERSION_PROBE_TIMEOUT);

        // wait_for_exit performs the kill-and-reap itself on every failure path, so the reports
        // below must not terminate the already-reaped child a second time.
        let plain_exit = match exit {
            Ok(status) => status,
            Err(timeout_error) => {
                let _ = join_capture(stdout_capture, "stdout");
                return Err(format!(
                    "rendered_output: the Node runtime probe did not exit; {}",
                    timeout_error.message
                ));
            }
        };
        let captured = match join_capture(stdout_capture, "stdout") {
            Ok(captured) => captured,
            Err(read_error) => {
                return Err(format!(
                    "rendered_output: the Node version could not be read: {}",
                    read_error.message
                ));
            }
        };
        let version = match String::from_utf8(captured.bytes) {
            Ok(version) => version.trim().to_owned(),
            Err(_) => "<invalid utf-8>".to_owned(),
        };
        if captured.truncated {
            return Err(
                "rendered_output: the Node version could not be read: the probe output was \
                 truncated by the capture bound."
                    .to_owned(),
            );
        }
        if !plain_exit.success() {
            return Err(format!(
                "rendered_output: '{executable} --version' exited with a failure status: {version}"
            ));
        }
        let major = parse_node_major(&version).ok_or_else(|| {
            format!("rendered_output: could not read a Node version from '{version}'")
        })?;
        if major < NODE_REQUIRED_MAJOR {
            return Err(format!(
                "rendered_output: '{executable}' is Node {version}, but emitted F16 values call \
                 native 'Math.f16round', which landed in Node 24. Make the '{executable}' \
                 executable on PATH a Node 24+ installation and rerun."
            ));
        }

        Ok(())
    } else {
        // The requested pipe was absent; the child still leaves without a pending read race.
        let termination = terminate(&mut child);
        Err(format!(
            "rendered_output: the probe could not capture its version output. termination \
             outcome: {}",
            render_termination_suffix(termination)
        ))
    }
}

/// Reads the major version from a `vMAJOR.[…]` version string.
fn parse_node_major(version: &str) -> Option<u32> {
    let digits = version.trim().strip_prefix('v')?.split('.').next()?;
    digits.parse().ok()
}

/// Runs one Node script inside `working_directory` under the harness deadline.
///
/// WHAT: spawns Node with piped output, drains both streams on their own threads, waits for exit
///       until the deadline, then kills and reaps the process if the deadline passes.
/// WHY: draining on separate threads keeps a chatty page from filling a pipe buffer and blocking
///      forever, and the deadline turns a non-terminating page into a reported timeout instead of
///      a hung suite.
pub(crate) fn run_node_script(
    script_path: &Path,
    working_directory: &Path,
) -> Result<NodeRunOutput, RenderHarnessError> {
    require_compatible_node_runtime()?;
    run_node_script_within(script_path, working_directory, NODE_EXECUTION_TIMEOUT)
}

/// Runs one Node script under a caller-chosen deadline.
///
/// The timeout is a parameter so the timeout path itself can be proved in a second rather than
/// waiting out the production budget. Production callers always use `run_node_script`.
pub(crate) fn run_node_script_within(
    script_path: &Path,
    working_directory: &Path,
    timeout: Duration,
) -> Result<NodeRunOutput, RenderHarnessError> {
    run_script_with_executable(NODE_EXECUTABLE, script_path, working_directory, timeout)
}

/// Runs one script under a caller-chosen interpreter.
///
/// The executable is a parameter so the spawn boundary can be proved with an interpreter that
/// certainly does not exist, without mutating the process-global `PATH` every other test shares.
/// Production callers always use `run_node_script`.
#[cfg(test)]
pub(crate) fn run_script_with_executable_for_test(
    executable: &str,
    script_path: &Path,
    working_directory: &Path,
    timeout: Duration,
) -> Result<NodeRunOutput, RenderHarnessError> {
    run_script_with_executable(executable, script_path, working_directory, timeout)
}

#[cfg(test)]
pub(crate) fn probe_node_runtime_for_test(executable: &str) -> Result<(), String> {
    probe_node_runtime(executable)
}

#[cfg(test)]
pub(crate) fn parse_node_major_for_test(version: &str) -> Option<u32> {
    parse_node_major(version)
}

fn run_script_with_executable(
    executable: &str,
    script_path: &Path,
    working_directory: &Path,
    timeout: Duration,
) -> Result<NodeRunOutput, RenderHarnessError> {
    let mut child = Command::new(executable)
        .arg(script_path)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            RenderHarnessError::spawn(format!(
                "rendered_output: failed to invoke '{executable}': {error}. \
                 Ensure '{NODE_EXECUTABLE}' is on PATH to use rendered-output assertions."
            ))
        })?;

    // Both pipes were requested, so a missing one is impossible in practice; if it ever happens
    // the child is still killed and reaped rather than left running behind the returned error.
    let (Some(stdout_pipe), Some(stderr_pipe)) = (child.stdout.take(), child.stderr.take()) else {
        let termination = terminate(&mut child);
        return Err(RenderHarnessError::output_decoding(format!(
            "rendered_output: a Node output pipe was not captured.{}",
            render_termination_suffix(termination)
        )));
    };

    let stdout_capture = std::thread::spawn(move || capture_stream(stdout_pipe));
    let stderr_capture = std::thread::spawn(move || capture_stream(stderr_pipe));

    let exit = wait_for_exit(&mut child, timeout);

    // Both handles are consumed before any of these results is inspected. Returning on the
    // stdout result first would drop the stderr handle unjoined, leaving a live capture thread
    // behind on exactly the failure paths this harness exists to report.
    let stdout_result = join_capture(stdout_capture, "stdout");
    let stderr_result = join_capture(stderr_capture, "stderr");

    let stderr = match stderr_result {
        Ok(captured) => captured,
        // Stderr is only ever diagnostic context for another failure. When something else already
        // failed, saying why the context is missing keeps that boundary as the reported one; when
        // nothing else failed, the capture failure is the only thing to report.
        Err(stderr_error) => {
            let another_boundary_failed = stdout_result.is_err()
                || exit.is_err()
                || exit.as_ref().is_ok_and(|status| !status.success());
            if !another_boundary_failed {
                return Err(stderr_error);
            }

            CapturedStream {
                bytes: format!("<stderr could not be captured: {}>", stderr_error.message)
                    .into_bytes(),
                truncated: false,
            }
        }
    };

    // Stderr is diagnostic context attached to a failure, never asserted output, so it is
    // described rather than decoded strictly. Replacing a timeout with a stderr-decoding failure
    // would hide the boundary that actually failed.
    let status = match exit {
        Ok(status) => status,
        Err(error) => {
            return Err(RenderHarnessError {
                kind: error.kind,
                message: format!(
                    "{}\nCaptured stderr:\n{}",
                    error.message,
                    describe_stream(&stderr)
                ),
            });
        }
    };

    if !status.success() {
        return Err(RenderHarnessError::exit_status(format!(
            "rendered_output: the Node harness exited with {status}.\nCaptured stderr:\n{}",
            describe_stream(&stderr)
        )));
    }

    // Stdout carries the harness event protocol, so it is decoded strictly.
    Ok(NodeRunOutput {
        stdout: decode_stream(&stdout_result?, "stdout")?,
    })
}

/// Waits for the child to exit, killing and reaping it once the deadline passes.
fn wait_for_exit(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, RenderHarnessError> {
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                let termination = terminate(child);
                return Err(RenderHarnessError::spawn(format!(
                    "rendered_output: failed to check whether the Node harness exited: {error}{}",
                    render_termination_suffix(termination)
                )));
            }
        }

        if Instant::now() >= deadline {
            let termination = terminate(child);
            return Err(RenderHarnessError::timeout(format!(
                "rendered_output: the Node harness did not exit within {timeout:?} and was killed. \
                 A page that never finishes is a harness failure, not a rendered-output mismatch.{}",
                render_termination_suffix(termination)
            )));
        }

        std::thread::sleep(EXIT_POLL_INTERVAL);
    }
}

/// What happened while killing and reaping the child.
///
/// Both steps are recorded because they fail independently: a child that exits between the final
/// `try_wait` and the `kill` makes the kill fail while the reap still succeeds, so a failed kill
/// must never skip the reap or the harness would claim "kill-and-reap" while leaving a zombie.
struct TerminationOutcome {
    kill: Option<std::io::Error>,
    wait: Option<std::io::Error>,
}

/// Kills the child and reaps it, attempting the reap whatever the kill did.
fn terminate(child: &mut Child) -> TerminationOutcome {
    let kill = child.kill().err();
    let wait = child.wait().err();

    TerminationOutcome { kill, wait }
}

fn render_termination_suffix(termination: TerminationOutcome) -> String {
    match (termination.kill, termination.wait) {
        (None, None) => String::new(),
        (Some(kill), None) => format!(
            " Killing the Node process failed ({kill}), but it was reaped, so it had already \
             exited."
        ),
        (None, Some(wait)) => format!(" Reaping the killed Node process failed: {wait}."),
        (Some(kill), Some(wait)) => format!(
            " Killing the Node process failed ({kill}) and reaping it also failed ({wait}), so it \
             may outlive this case."
        ),
    }
}

/// Bytes captured from one process stream, plus whether the capture bound was reached.
struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Reads a stream to EOF, keeping at most `MAX_CAPTURED_STREAM_BYTES` but always draining.
///
/// Draining past the bound matters: an unread pipe blocks the child, which would turn a noisy
/// page into a timeout instead of the bounded-capture failure it really is.
fn capture_stream(mut source: impl Read) -> std::io::Result<CapturedStream> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];

    loop {
        let read = source.read(&mut chunk)?;
        if read == 0 {
            break;
        }

        let capacity_left = MAX_CAPTURED_STREAM_BYTES - bytes.len();
        if capacity_left == 0 {
            truncated = true;
            continue;
        }

        let kept = read.min(capacity_left);
        bytes.extend_from_slice(&chunk[..kept]);
        truncated |= kept < read;
    }

    Ok(CapturedStream { bytes, truncated })
}

fn join_capture(
    handle: std::thread::JoinHandle<std::io::Result<CapturedStream>>,
    stream_name: &str,
) -> Result<CapturedStream, RenderHarnessError> {
    match handle.join() {
        Ok(Ok(captured)) => Ok(captured),
        Ok(Err(error)) => Err(RenderHarnessError::output_decoding(format!(
            "rendered_output: failed to read Node {stream_name}: {error}"
        ))),
        Err(_) => Err(RenderHarnessError::output_decoding(format!(
            "rendered_output: the Node {stream_name} capture thread panicked"
        ))),
    }
}

/// Renders a captured stream for a failure report, describing it when it cannot be shown as text.
///
/// This never feeds an assertion, so it says what it could not show instead of failing the run
/// and replacing the real failure boundary.
fn describe_stream(captured: &CapturedStream) -> String {
    let text = match std::str::from_utf8(&captured.bytes) {
        Ok(text) => text.to_owned(),
        Err(error) => format!(
            "<{} bytes that are not valid UTF-8: {error}>",
            captured.bytes.len()
        ),
    };

    if captured.truncated {
        return format!(
            "{text}\n<truncated at the {MAX_CAPTURED_STREAM_BYTES}-byte capture bound>"
        );
    }

    text
}

/// Decodes a captured stream strictly, rejecting invalid UTF-8 and truncated capture.
fn decode_stream(
    captured: &CapturedStream,
    stream_name: &str,
) -> Result<String, RenderHarnessError> {
    if captured.truncated {
        return Err(RenderHarnessError::output_decoding(format!(
            "rendered_output: Node {stream_name} exceeded the {MAX_CAPTURED_STREAM_BYTES}-byte \
             capture bound, so the harness cannot report complete output"
        )));
    }

    match std::str::from_utf8(&captured.bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) => Err(RenderHarnessError::output_decoding(format!(
            "rendered_output: Node {stream_name} is not valid UTF-8: {error}"
        ))),
    }
}
