//! Profile JSON parser for Samply/Firefox processed profiles.
//!
//! WHAT: Reads gzip-compressed Firefox processed profile JSON, extracts
//! per-thread string/stack/frame/function tables, walks sample stacks from
//! leaf to root, and produces per-function inclusive/self sample counts with
//! caller/callee edge data.
//!
//! WHY: Samply 0.13.1 writes Firefox-format processed profiles with tables
//! inside each thread object (no top-level `shared`). The parser models only
//! the narrow subset needed for function hotspot extraction: per-thread
//! tables, per-thread samples, and stack prefix chains. Defensive validation
//! catches malformed profiles without panics.
//!
//! # What this module owns
//! - `parse_profile()` entry point that reads `profile.json.gz`
//! - `ParsedProfileSummary` output with per-function sample accounting
//! - Stack walking, string resolution, cycle detection, and weight handling
//!
//! # What this module does NOT own
//! - Hotspot extraction and filtering (see `hotspots.rs`)
//! - Owner bucket mapping (see `buckets.rs`)
//! - Artifact writing (see `artifacts.rs`)

use flate2::read::GzDecoder;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

// ---------------------------------------------------------------------------
//  Output types
// ---------------------------------------------------------------------------

/// Parsed profile summary containing per-function sample accounting.
///
/// WHAT: The complete result of parsing one profile, including total counts,
/// per-function samples, and any non-fatal warnings.
/// WHY: Named fields make the parser's output explicit and testable.
#[derive(Debug)]
pub(crate) struct ParsedProfileSummary {
    /// Total number of samples across all threads (counting each sample once).
    pub(crate) total_sample_count: usize,
    /// Total sample weight across all threads.
    pub(crate) total_sample_weight: f64,
    /// Per-function sample data, sorted by inclusive weight descending.
    pub(crate) functions: Vec<ProfileFunctionSamples>,
    /// Non-fatal warnings collected during parsing.
    pub(crate) warnings: Vec<String>,
}

/// Small structural diagnostic for profiles whose hot functions are raw addresses.
///
/// WHAT: Captures the top-level metadata and the first thread's table shape
/// without trying to reinterpret the profile.
/// WHY: When function names are raw addresses, the next question is whether
/// the saved profile lacks symbol names or whether the parser is looking in
/// the wrong place. This dump makes that distinction easier without changing
/// hotspot extraction.
#[derive(Debug, Clone)]
pub(crate) struct ProfileShapeDump {
    pub(crate) meta_product: String,
    pub(crate) meta_version: String,
    pub(crate) thread_count: usize,
    pub(crate) first_thread_func_table_keys: Vec<String>,
    pub(crate) first_20_func_names: Vec<String>,
    pub(crate) resource_table_keys: Vec<String>,
    pub(crate) libs_count: Option<usize>,
    pub(crate) first_10_libs: Vec<String>,
    pub(crate) native_symbols_present: bool,
}

/// Sample accounting for a single function across all threads.
///
/// WHAT: Tracks inclusive/self sample counts, thread presence, and
/// caller/callee edge weights for one resolved function name.
/// WHY: These fields drive hotspot extraction, percentage calculation,
/// and agent-readable summaries.
#[derive(Debug)]
pub(crate) struct ProfileFunctionSamples {
    /// Resolved function name from the profile string table.
    pub(crate) name: String,
    /// Inclusive sample weight: counted once per sample that includes this
    /// function anywhere in the stack (recursion-safe).
    pub(crate) inclusive_samples: f64,
    /// Self sample weight: counted only when this function is the leaf.
    pub(crate) self_samples: f64,
    /// Thread names where this function appears (deduplicated).
    /// Used by agent summaries.
    #[allow(dead_code)]
    pub(crate) thread_names: Vec<String>,
    /// Caller edges: which functions called this one, sorted by weight descending.
    pub(crate) callers: Vec<ProfileEdge>,
    /// Callee edges: which functions this one called, sorted by weight descending.
    pub(crate) callees: Vec<ProfileEdge>,
}

/// A weighted caller/callee edge between two functions.
///
/// WHAT: Represents one direction of a call relationship with its sample weight.
/// WHY: Edge data helps agents understand call chains and identify indirect
/// contributors to hotspots.
#[derive(Debug, Clone)]
pub(crate) struct ProfileEdge {
    /// Name of the function at the other end of this edge.
    pub(crate) function_name: String,
    /// Sample weight attributed to this edge.
    pub(crate) samples: f64,
    /// Percentage of total sample weight.
    pub(crate) pct: f64,
}

// ---------------------------------------------------------------------------
//  Internal types
// ---------------------------------------------------------------------------

/// Intermediate data for one function accumulated during sample walking.
///
/// WHAT: Mutable accumulator that the parser fills while processing samples.
/// WHY: Separating accumulation from output construction keeps the sample
/// walk simple and the output types immutable.
struct FunctionData {
    inclusive_weight: f64,
    self_weight: f64,
    thread_names: HashSet<String>,
    callers: HashMap<String, f64>,
    callees: HashMap<String, f64>,
}

impl FunctionData {
    fn new() -> Self {
        Self {
            inclusive_weight: 0.0,
            self_weight: 0.0,
            thread_names: HashSet::new(),
            callers: HashMap::new(),
            callees: HashMap::new(),
        }
    }
}

/// Thread metadata extracted during parsing.
///
/// WHAT: Captures thread name and main-thread status for thread tracking.
/// WHY: Avoids passing multiple thread-related fields through the walk loop.
struct ThreadInfo {
    name: String,
    #[allow(dead_code)]
    is_main: bool,
}

/// Per-thread tables extracted from a thread object.
///
/// WHAT: Holds the string table, function/frame/stack table lookups, and
/// warnings for one thread. Each Samply 0.13.1 thread carries its own
/// tables rather than sharing them at the profile root.
///
/// WHY: Per-thread tables are the real Samply 0.13.1 format. Isolating
/// them in a struct keeps the thread processing flow readable and avoids
/// threading many table vectors through function signatures.
struct ThreadContext {
    strings: Vec<String>,
    func_to_name: Vec<usize>,
    frame_to_func: Vec<usize>,
    stack_frame: Vec<i64>,
    stack_prefix: Vec<i64>,
    warnings: Vec<String>,
}

/// Accumulator that merges per-function sample data across all threads.
///
/// WHAT: Owns the mutable function data map and running totals that
/// accumulate samples from every thread.
///
/// WHY: Samply profiles contain multiple threads, each with independent
/// tables and samples. The accumulator merges results into a single
/// `ParsedProfileSummary` so downstream consumers see unified data.
struct ProfileAccumulator {
    functions: HashMap<String, FunctionData>,
    total_sample_weight: f64,
    total_sample_count: usize,
    warnings: Vec<String>,
}

impl ProfileAccumulator {
    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            total_sample_weight: 0.0,
            total_sample_count: 0,
            warnings: Vec::new(),
        }
    }

    /// Append warnings from a per-thread parse.
    fn add_warnings(&mut self, warnings: Vec<String>) {
        self.warnings.extend(warnings);
    }

    /// Look up a function name by its function-table index within a thread context.
    ///
    /// Returns `"unknown"` for out-of-range indexes or empty names.
    fn function_name(thread_ctx: &ThreadContext, func_id: usize) -> &str {
        if func_id >= thread_ctx.func_to_name.len() {
            return "unknown";
        }
        let name_id = thread_ctx.func_to_name[func_id];
        if name_id >= thread_ctx.strings.len() {
            return "unknown";
        }
        let name = thread_ctx.strings[name_id].trim();
        if name.is_empty() { "unknown" } else { name }
    }

    /// Record inclusive samples for each unique function in the stack.
    ///
    /// Uses a per-sample set to avoid double-counting recursive stacks.
    /// The weight parameter is the sample's weight (e.g., from the weight array).
    fn account_inclusive(&mut self, stack_functions: &[&str], weight: f64) {
        let mut seen = HashSet::new();
        for name in stack_functions {
            if seen.insert(*name) {
                let entry = self
                    .functions
                    .entry((*name).to_owned())
                    .or_insert_with(FunctionData::new);
                entry.inclusive_weight += weight;
            }
        }
    }

    /// Record a self sample for the leaf function.
    fn account_self(&mut self, leaf: &str, weight: f64) {
        let entry = self
            .functions
            .entry(leaf.to_owned())
            .or_insert_with(FunctionData::new);
        entry.self_weight += weight;
    }

    /// Record caller/callee edges for adjacent function pairs in the stack.
    fn account_edges(&mut self, stack_functions: &[&str], weight: f64) {
        // Build deduplicated edges: for each adjacent pair, accumulate once
        // per unique (caller, callee) combination per sample.
        let mut seen = HashSet::new();
        for window in stack_functions.windows(2) {
            let (caller, callee) = (window[0], window[1]);
            if seen.insert((caller, callee)) {
                let entry = self
                    .functions
                    .entry(caller.to_owned())
                    .or_insert_with(FunctionData::new);
                *entry.callees.entry(callee.to_owned()).or_insert(0.0) += weight;

                let entry = self
                    .functions
                    .entry(callee.to_owned())
                    .or_insert_with(FunctionData::new);
                *entry.callers.entry(caller.to_owned()).or_insert(0.0) += weight;
            }
        }
    }

    /// Record thread presence for each function in the stack.
    fn account_threads(&mut self, stack_functions: &[&str], thread: &ThreadInfo) {
        for name in stack_functions {
            let entry = self
                .functions
                .entry((*name).to_owned())
                .or_insert_with(FunctionData::new);
            entry.thread_names.insert(thread.name.clone());
        }
    }

    /// Process a single sample with its weight.
    ///
    /// Walks the stack from leaf to root using per-thread tables, resolves
    /// function names, and dispatches to the accounting helpers.
    fn process_sample(
        &mut self,
        thread_ctx: &ThreadContext,
        stack_index: usize,
        thread: &ThreadInfo,
        weight: f64,
    ) -> Result<(), String> {
        // Walk the prefix chain from leaf to root.
        let mut frames = Vec::new();
        let mut current = stack_index as i64;
        let mut visited = HashSet::new();

        while current >= 0 {
            let idx = current as usize;
            if idx >= thread_ctx.stack_frame.len() {
                return Err(format!("Stack index {} out of bounds", idx));
            }
            if !visited.insert(idx) {
                return Err(format!(
                    "Cycle detected in stack prefix chain at index {}",
                    idx
                ));
            }
            frames.push(idx);
            current = thread_ctx.stack_prefix[idx];
        }

        // Reverse to caller-to-callee order.
        frames.reverse();

        // Resolve function names into owned strings to avoid borrow conflicts.
        // Collecting into Vec<String> ends the borrow of thread_ctx.strings before
        // we call the mutable accounting methods.
        // Map: stack index → frame index (via stack_frame) → func index (via frame_to_func) → name.
        let stack_functions: Vec<String> = frames
            .iter()
            .map(|&stack_idx| {
                let frame_id = thread_ctx.stack_frame[stack_idx];
                let frame_func = thread_ctx.frame_to_func[frame_id as usize];
                Self::function_name(thread_ctx, frame_func).to_owned()
            })
            .collect();

        if stack_functions.is_empty() {
            return Ok(());
        }

        let leaf = stack_functions[stack_functions.len() - 1].clone();
        let stack_refs: Vec<&str> = stack_functions.iter().map(|s| s.as_str()).collect();

        self.account_inclusive(&stack_refs, weight);
        self.account_self(&leaf, weight);
        self.account_edges(&stack_refs, weight);
        self.account_threads(&stack_refs, thread);

        Ok(())
    }

    /// Convert accumulated function data into the output summary.
    fn build_summary(self) -> ParsedProfileSummary {
        let mut functions = Vec::new();

        for (name, data) in self.functions {
            let callers = build_edges(&data.callers, self.total_sample_weight);
            let callees = build_edges(&data.callees, self.total_sample_weight);
            let mut thread_names: Vec<String> = data.thread_names.into_iter().collect();
            thread_names.sort();

            functions.push(ProfileFunctionSamples {
                name,
                inclusive_samples: data.inclusive_weight,
                self_samples: data.self_weight,
                thread_names,
                callers,
                callees,
            });
        }

        // Sort by inclusive weight descending, then self weight descending.
        functions.sort_by(|a, b| {
            b.inclusive_samples
                .total_cmp(&a.inclusive_samples)
                .then(b.self_samples.total_cmp(&a.self_samples))
        });

        ParsedProfileSummary {
            total_sample_count: self.total_sample_count,
            total_sample_weight: self.total_sample_weight,
            functions,
            warnings: self.warnings,
        }
    }
}

/// Build sorted `ProfileEdge` entries from a raw edge map.
fn build_edges(edges: &HashMap<String, f64>, total_weight: f64) -> Vec<ProfileEdge> {
    let mut result: Vec<ProfileEdge> = edges
        .iter()
        .map(|(name, &weight)| ProfileEdge {
            function_name: name.clone(),
            samples: weight,
            pct: if total_weight > 0.0 {
                (weight / total_weight) * 100.0
            } else {
                0.0
            },
        })
        .collect();

    result.sort_by(|a, b| b.samples.total_cmp(&a.samples));
    result
}

// ---------------------------------------------------------------------------
//  Entry point
// ---------------------------------------------------------------------------

/// Parse a gzip-compressed Firefox processed profile.
///
/// WHAT: Reads `profile.json.gz`, decompresses it, parses the JSON, validates
/// the required per-thread table shapes, walks all thread samples, and returns
/// a `ParsedProfileSummary` with per-function sample accounting.
///
/// WHY: This is the entry point called after Samply recording succeeds. The
/// parser is intentionally narrow: it models only the Firefox processed-profile
/// subset needed for function hotspots.
pub(crate) fn parse_profile(path: &Path) -> Result<ParsedProfileSummary, String> {
    let json = read_profile_json(path)?;
    parse_profile_json(&json, path)
}

pub(crate) fn parse_profile_shape_dump(path: &Path) -> Result<ProfileShapeDump, String> {
    let json = read_profile_json(path)?;
    profile_shape_dump_from_json(&json, path)
}

/// Read and decompress a gzip-compressed profile file into a JSON string.
fn read_profile_json(path: &Path) -> Result<String, String> {
    let file = File::open(path)
        .map_err(|e| format!("Failed to open profile '{}': {}", path.display(), e))?;

    let reader = BufReader::new(file);
    let mut decoder = GzDecoder::new(reader);
    let mut json_string = String::new();
    decoder
        .read_to_string(&mut json_string)
        .map_err(|e| format!("Failed to decompress profile '{}': {}", path.display(), e))?;

    Ok(json_string)
}

/// Parse a profile from a pre-read JSON string.
///
/// WHAT: Validates the required per-thread table shapes, extracts tables
/// from each thread, walks all thread samples, and returns sample accounting.
///
/// WHY: Separating JSON reading from parsing lets tests provide JSON directly
/// without creating gzip files.
pub(crate) fn parse_profile_json(
    json_string: &str,
    path: &Path,
) -> Result<ParsedProfileSummary, String> {
    let root: Value = serde_json::from_str(json_string).map_err(|e| {
        format!(
            "Failed to parse profile JSON from '{}': {}",
            path.display(),
            e
        )
    })?;

    let mut accumulator = ProfileAccumulator::new();
    parse_all_threads(&mut accumulator, &root, path)?;
    Ok(accumulator.build_summary())
}

pub(crate) fn profile_shape_dump_from_json(
    json_string: &str,
    path: &Path,
) -> Result<ProfileShapeDump, String> {
    let root: Value = serde_json::from_str(json_string).map_err(|e| {
        format!(
            "Failed to parse profile JSON from '{}': {}",
            path.display(),
            e
        )
    })?;

    Ok(build_profile_shape_dump(&root))
}

fn build_profile_shape_dump(root: &Value) -> ProfileShapeDump {
    let meta = root.get("meta").and_then(|value| value.as_object());
    let meta_product = meta_string(meta, "product");
    let meta_version = meta_string(meta, "version");

    let threads = root
        .get("threads")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let first_thread = threads.first();

    let first_thread_func_table_keys = first_thread
        .and_then(|thread| thread.get("funcTable"))
        .and_then(|table| table.as_object())
        .map(sorted_object_keys)
        .unwrap_or_default();
    let first_20_func_names = first_thread
        .map(first_thread_function_names)
        .unwrap_or_default();

    let resource_table_keys = root
        .get("resourceTable")
        .or_else(|| first_thread.and_then(|thread| thread.get("resourceTable")))
        .and_then(|table| table.as_object())
        .map(sorted_object_keys)
        .unwrap_or_default();

    let libs = root.get("libs").and_then(|value| value.as_array());
    let libs_count = libs.map(|items| items.len());
    let first_10_libs = libs
        .map(|items| items.iter().take(10).map(display_lib_entry).collect())
        .unwrap_or_default();

    ProfileShapeDump {
        meta_product,
        meta_version,
        thread_count: threads.len(),
        first_thread_func_table_keys,
        first_20_func_names,
        resource_table_keys,
        libs_count,
        first_10_libs,
        native_symbols_present: contains_key_recursive(root, "nativeSymbols"),
    }
}

fn meta_string(meta: Option<&serde_json::Map<String, Value>>, key: &str) -> String {
    meta.and_then(|map| map.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn sorted_object_keys(map: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

fn first_thread_function_names(thread: &Value) -> Vec<String> {
    let strings = thread
        .get("stringArray")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let names = thread
        .get("funcTable")
        .and_then(|table| table.get("name"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    names
        .iter()
        .take(20)
        .map(|name_index| {
            name_index
                .as_u64()
                .and_then(|index| strings.get(index as usize))
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_string()
        })
        .collect()
}

fn display_lib_entry(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return truncate_shape_text(&value.to_string());
    };

    for key in ["debugName", "name", "path"] {
        if let Some(text) = object.get(key).and_then(|entry| entry.as_str()) {
            return truncate_shape_text(text);
        }
    }

    truncate_shape_text(&value.to_string())
}

fn truncate_shape_text(text: &str) -> String {
    const LIMIT: usize = 140;
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(LIMIT - 3).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        text.to_string()
    }
}

fn contains_key_recursive(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|child| contains_key_recursive(child, key))
        }
        Value::Array(items) => items.iter().any(|child| contains_key_recursive(child, key)),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
//  Thread parsing
// ---------------------------------------------------------------------------

/// Parse all threads and their per-thread tables.
///
/// WHAT: Iterates over `threads[*]`, extracts per-thread tables and sample
/// data, and dispatches each sample to the accumulator's accounting helpers.
///
/// WHY: Samply 0.13.1 stores string/func/frame/stack tables inside each
/// thread object rather than in a top-level `shared` object. Each thread
/// is processed independently with its own tables, then results merge into
/// the shared accumulator.
fn parse_all_threads(
    accumulator: &mut ProfileAccumulator,
    root: &Value,
    path: &Path,
) -> Result<(), String> {
    let threads = match root.get("threads") {
        Some(Value::Array(t)) => t,
        _ => {
            return Err(format!(
                "Profile '{}' is missing required 'threads' array.",
                path.display()
            ));
        }
    };

    if threads.is_empty() {
        accumulator.warnings.push(format!(
            "Profile '{}' has an empty threads array.",
            path.display()
        ));
    }

    for thread_value in threads {
        parse_thread(accumulator, thread_value, path)?;
    }

    Ok(())
}

/// Parse a single thread's tables and samples.
///
/// WHAT: Extracts the thread's own string/func/frame/stack tables, then
/// walks its samples using those tables. Results accumulate into the
/// shared `ProfileAccumulator` so cross-thread functions merge by name.
///
/// WHY: Each thread is self-contained in Samply 0.13.1 output. Processing
/// one thread at a time keeps the control flow clear and lets us report
/// per-thread warnings without losing context.
fn parse_thread(
    accumulator: &mut ProfileAccumulator,
    thread: &Value,
    path: &Path,
) -> Result<(), String> {
    // Extract thread metadata.
    let thread_name = match thread.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_owned(),
        None => {
            accumulator
                .warnings
                .push("Thread is missing 'name' field.".to_string());
            "unknown".to_owned()
        }
    };

    let is_main = thread
        .get("isMainThread")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let thread_info = ThreadInfo {
        name: thread_name,
        is_main,
    };

    // Extract per-thread tables (Samply 0.13.1 format).
    let mut thread_ctx = parse_thread_tables(thread, path)?;

    // Merge thread-level warnings into the accumulator before processing
    // samples, so the borrow of thread_ctx for process_sample is not
    // conflicted by a partial move of warnings.
    accumulator.add_warnings(std::mem::take(&mut thread_ctx.warnings));

    // Extract samples from this thread.
    let samples = match thread.get("samples") {
        Some(s) => s,
        None => {
            accumulator.warnings.push(format!(
                "Thread '{}' has no 'samples' field.",
                thread_info.name
            ));
            return Ok(());
        }
    };

    let stacks = match samples.get("stack").and_then(|v| v.as_array()) {
        Some(s) => s,
        None => {
            accumulator.warnings.push(format!(
                "Thread '{}' samples is missing 'stack' array.",
                thread_info.name
            ));
            return Ok(());
        }
    };

    // Parse weight type and weights.
    let weight_type = samples
        .get("weightType")
        .and_then(|v| v.as_str())
        .unwrap_or("samples");

    if weight_type != "samples" {
        accumulator.warnings.push(format!(
            "Thread '{}' has non-standard weightType '{}'; weights must be numeric.",
            thread_info.name, weight_type
        ));
    }

    let weights = parse_weights(
        samples,
        stacks.len(),
        &thread_info.name,
        &mut accumulator.warnings,
    );

    // Process each sample using per-thread tables.
    for (i, stack_value) in stacks.iter().enumerate() {
        // Null stacks are skipped (no sample to process).
        if stack_value.is_null() {
            continue;
        }

        let stack_index = match stack_value.as_u64() {
            Some(idx) => idx as usize,
            None => {
                accumulator.warnings.push(format!(
                    "Thread '{}' sample {} has non-integer stack value.",
                    thread_info.name, i
                ));
                continue;
            }
        };

        if stack_index >= thread_ctx.stack_frame.len() {
            return Err(format!(
                "Thread '{}' sample {} references stack index {} which is out of bounds (max {}).",
                thread_info.name,
                i,
                stack_index,
                thread_ctx.stack_frame.len().saturating_sub(1)
            ));
        }

        let weight = weights[i];
        accumulator.total_sample_weight += weight;
        accumulator.total_sample_count += 1;

        // Process the sample, passing the weight for per-function accounting.
        accumulator.process_sample(&thread_ctx, stack_index, &thread_info, weight)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  Per-thread table parsing
// ---------------------------------------------------------------------------

/// Extract and validate tables from a single thread object.
///
/// WHAT: Reads `thread.stringArray`, `thread.funcTable.name`,
/// `thread.frameTable.func`, `thread.stackTable.prefix`, and
/// `thread.stackTable.frame`. Validates that indexes are in range
/// and the prefix chain has no cycles.
///
/// WHY: Samply 0.13.1 stores tables per-thread rather than in a
/// top-level `shared` object. Each thread's tables are independent,
/// so function/frame/stack indexes are local to that thread.
fn parse_thread_tables(thread: &Value, path: &Path) -> Result<ThreadContext, String> {
    let mut warnings = Vec::new();

    // String table
    let string_array = thread
        .get("stringArray")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            format!(
                "Profile '{}' thread is missing required 'stringArray' array.",
                path.display()
            )
        })?;

    let strings: Vec<String> = string_array
        .iter()
        .enumerate()
        .map(|(i, v)| match v.as_str() {
            Some(s) => s.to_owned(),
            None => {
                warnings.push(format!("thread.stringArray[{}] is not a string", i));
                String::new()
            }
        })
        .collect();

    // Function table
    let func_table = require_object(thread, "funcTable", path)?;
    let func_names = require_array_from_map(func_table, "name", path)?;
    let func_to_name: Vec<usize> = func_names
        .iter()
        .enumerate()
        .map(|(i, v)| match v.as_u64() {
            Some(n) => n as usize,
            None => {
                warnings.push(format!("thread.funcTable.name[{}] is not an integer", i));
                0
            }
        })
        .collect();

    // Frame table
    let frame_table = require_object(thread, "frameTable", path)?;
    let frame_funcs = require_array_from_map(frame_table, "func", path)?;
    let frame_to_func: Vec<usize> = frame_funcs
        .iter()
        .enumerate()
        .map(|(i, v)| match v.as_u64() {
            Some(n) => n as usize,
            None => {
                warnings.push(format!("thread.frameTable.func[{}] is not an integer", i));
                0
            }
        })
        .collect();

    // Stack table
    let stack_table = require_object(thread, "stackTable", path)?;
    let prefix_array = require_array_from_map(stack_table, "prefix", path)?;
    let frame_array = require_array_from_map(stack_table, "frame", path)?;

    if prefix_array.len() != frame_array.len() {
        return Err(format!(
            "Profile '{}' thread has mismatched stackTable lengths: prefix={}, frame={}",
            path.display(),
            prefix_array.len(),
            frame_array.len()
        ));
    }

    let stack_prefix: Vec<i64> = prefix_array
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if v.is_null() {
                // Null prefix is the standard representation for root stacks
                // in Samply processed profiles.
                return -1;
            }

            match v.as_i64() {
                Some(n) => n,
                None => {
                    warnings.push(format!("thread.stackTable.prefix[{}] is not an integer", i));
                    -1
                }
            }
        })
        .collect();

    let stack_frame: Vec<i64> = frame_array
        .iter()
        .enumerate()
        .map(|(i, v)| match v.as_i64() {
            Some(n) => n,
            None => {
                warnings.push(format!("thread.stackTable.frame[{}] is not an integer", i));
                0
            }
        })
        .collect();

    // Validate prefix chain: no cycles, no out-of-range references.
    validate_prefix_chain(&stack_prefix, path)?;

    Ok(ThreadContext {
        strings,
        func_to_name,
        frame_to_func,
        stack_frame,
        stack_prefix,
        warnings,
    })
}

/// Validate that the stack prefix chain has no cycles and no out-of-range indexes.
fn validate_prefix_chain(prefix: &[i64], path: &Path) -> Result<(), String> {
    for (i, &p) in prefix.iter().enumerate() {
        if p >= 0 && (p as usize) >= prefix.len() {
            return Err(format!(
                "Profile '{}' has out-of-range prefix at stack index {}: {}",
                path.display(),
                i,
                p
            ));
        }
        if p >= 0 && (p as usize) >= i {
            return Err(format!(
                "Profile '{}' has forward prefix at stack index {}: {} (must reference earlier stack)",
                path.display(),
                i,
                p
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
//  Weight parsing
// ---------------------------------------------------------------------------

/// Parse the weight array for a thread's samples.
///
/// Returns one weight per sample. If the weight array is missing or
/// malformed, returns 1.0 for each sample.
fn parse_weights(
    samples: &Value,
    sample_count: usize,
    thread_name: &str,
    warnings: &mut Vec<String>,
) -> Vec<f64> {
    let Some(weight_value) = samples.get("weight") else {
        return vec![1.0; sample_count];
    };

    let Some(weight_array) = weight_value.as_array() else {
        warnings.push(format!(
            "Thread '{}' has non-array 'weight' field; using default weights.",
            thread_name
        ));
        return vec![1.0; sample_count];
    };

    if weight_array.len() != sample_count {
        warnings.push(format!(
            "Thread '{}' weight array length ({}) does not match stack count ({}); using default weights.",
            thread_name,
            weight_array.len(),
            sample_count
        ));
        return vec![1.0; sample_count];
    }

    let mut weights = Vec::with_capacity(sample_count);
    for (i, w) in weight_array.iter().enumerate() {
        match w.as_f64() {
            Some(v) if v.is_finite() => weights.push(v),
            _ => {
                warnings.push(format!(
                    "Thread '{}' weight[{}] is not a finite number; using 1.0.",
                    thread_name, i
                ));
                weights.push(1.0);
            }
        }
    }

    weights
}

// ---------------------------------------------------------------------------
//  JSON access helpers
// ---------------------------------------------------------------------------

/// Require a field from a JSON object map to be an array.
fn require_array_from_map<'a>(
    map: &'a serde_json::Map<String, Value>,
    field: &str,
    path: &Path,
) -> Result<&'a Vec<Value>, String> {
    map.get(field).and_then(|v| v.as_array()).ok_or_else(|| {
        format!(
            "Profile '{}' is missing required array '{}'.",
            path.display(),
            field
        )
    })
}

/// Require a field to be a JSON object, returning an error if missing or wrong type.
fn require_object<'a>(
    parent: &'a Value,
    field: &str,
    path: &Path,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    parent
        .get(field)
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            format!(
                "Profile '{}' is missing required object '{}'.",
                path.display(),
                field
            )
        })
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
