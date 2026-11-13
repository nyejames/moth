//! The native engine: one compilation policy shared by every module built from it.
//!
//! Every engine option Lappet's own checks and documented exclusions rely on is set explicitly
//! in [`engine_config`], so a Wasmtime upgrade can't change those by changing a default. The
//! remaining Wasm proposals follow the pinned release's Cranelift profile until compiled Moth
//! output shows which of them it needs.

use super::error::RuntimeError;
use super::module::CompiledModule;
use std::num::NonZeroUsize;
use wasmtime::{Config, Engine, Inlining, Module, OptLevel, RegallocAlgorithm, Strategy};

/// Wasm frames kept on a trap for attribution. This is Wasmtime 50.0.1's own default, pinned
/// here because the plain on/off backtrace toggle is deprecated in that release.
const WASM_BACKTRACE_MAX_FRAMES: usize = 20;

/// The Wasm stack one guest entry may use. It bounds guest recursion, not native host
/// recursion or the thread's own stack.
const MAX_WASM_STACK_BYTES: usize = 512 * 1024;

/// How hard Cranelift optimises native code.
///
/// This is independent of the Moth build profile: a release Moth build may run under development
/// native optimisation and the reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeOptimisation {
    /// Fastest native compilation, for edit-and-run loops.
    Development,
    /// Optimised native code, for shipped applications.
    Release,
}

impl NativeOptimisation {
    fn cranelift_level(self) -> OptLevel {
        match self {
            Self::Development => OptLevel::None,
            Self::Release => OptLevel::Speed,
        }
    }
}

/// A Cranelift-backed Wasmtime engine.
///
/// The engine is cheap to share by reference and across threads. Modules compiled by one
/// engine can't be used with another.
pub struct RuntimeEngine {
    engine: Engine,
}

impl RuntimeEngine {
    /// Builds the engine, reporting a configuration this host can't support.
    pub fn new(optimisation: NativeOptimisation) -> Result<Self, RuntimeError> {
        let config = engine_config(optimisation);
        let engine = Engine::new(&config).map_err(RuntimeError::engine_configuration)?;
        Ok(Self { engine })
    }

    /// Validates and compiles binary core Wasm for this engine.
    ///
    /// A module that needs a Wasm proposal outside the engine's profile is rejected here as
    /// [`RuntimeError::InvalidModule`]. Wasmtime reports that case and malformed bytes through
    /// the same validation failure, so the two aren't distinguished.
    pub fn compile_module(&self, wasm_bytes: &[u8]) -> Result<CompiledModule, RuntimeError> {
        let module =
            Module::from_binary(&self.engine, wasm_bytes).map_err(RuntimeError::invalid_module)?;
        Ok(CompiledModule::new(module))
    }

    /// Advances the engine's epoch by one tick.
    ///
    /// Guest calls with a deadline are interrupted once enough ticks have passed. Lappet starts
    /// no timer, so the embedder chooses what a tick means: a frame, a request budget slice or
    /// a watchdog interval. This is safe to call from any thread while guests run.
    pub fn advance_epoch(&self) {
        self.engine.increment_epoch();
    }

    pub(super) fn wasmtime(&self) -> &Engine {
        &self.engine
    }
}

/// The one place the engine policy is written down.
fn engine_config(optimisation: NativeOptimisation) -> Config {
    let mut config = Config::new();

    // Native code generation. `Strategy::Auto` selects Cranelift today, but an implicit choice
    // isn't a policy. The single-pass allocator isn't production-ready upstream.
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(optimisation.cranelift_level());
    config.cranelift_regalloc_algorithm(RegallocAlgorithm::Backtracking);
    config.compiler_inlining(Inlining::No);

    // Wasm proposals. Exports return ordered multiple results, guest addresses are 32-bit and
    // the limit checks assume the standard page size. Relaxed SIMD is outside the V1 profile
    // because its results may differ between targets. Threads and GC aren't compiled into the
    // dependency at all.
    config.wasm_multi_value(true);
    config.wasm_memory64(false);
    config.wasm_custom_page_sizes(false);
    config.wasm_relaxed_simd(false);

    // Execution bounds. Every guest entry sets an epoch deadline, so guest code is always
    // interruptible by the embedder. Deterministic fuel metering has no V1 consumer.
    config.epoch_interruption(true);
    config.consume_fuel(false);
    config.max_wasm_stack(MAX_WASM_STACK_BYTES);

    // Traps keep a bounded Wasm backtrace for attribution. A zero frame count would switch
    // capture off.
    config.wasm_backtrace_max_frames(NonZeroUsize::new(WASM_BACKTRACE_MAX_FRAMES));

    config
}
