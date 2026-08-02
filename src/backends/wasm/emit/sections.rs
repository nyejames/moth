//! Section planning and deterministic index assignment for Wasm emission.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::lir::function::WasmLirFunction;
use crate::backends::wasm::lir::instructions::WasmLirStmt;
use crate::backends::wasm::lir::linkage::WasmImportKind;
use crate::backends::wasm::lir::module::WasmLirModule;
use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmImportId, WasmLirFunctionId, WasmLirSignature, WasmStaticDataId,
};
use crate::backends::wasm::request::WasmBackendRequest;
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use rustc_hash::FxHashMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DefinedFunctionKey {
    /// User/runtime-template function lowered into LIR.
    Lir(WasmLirFunctionId),
    /// Synthetic runtime helper emitted directly by the Wasm backend.
    Helper(WasmRuntimeHelper),
}

#[derive(Debug, Clone)]
pub(crate) struct WasmEmitPlan {
    /// Interned signature table in deterministic insertion order.
    pub type_entries: Vec<WasmLirSignature>,
    /// Reverse lookup for signature -> type index.
    pub type_index_by_signature: FxHashMap<WasmLirSignature, u32>,
    /// Function indices assigned to imports (always come first).
    pub import_function_indices: FxHashMap<WasmImportId, u32>,
    /// Function indices assigned to LIR-defined functions.
    pub function_indices: FxHashMap<WasmLirFunctionId, u32>,
    /// Function indices assigned to synthesized helpers.
    pub helper_indices: FxHashMap<WasmRuntimeHelper, u32>,
    /// Canonical order used by function/code section emission.
    pub defined_function_order: Vec<DefinedFunctionKey>,
    /// Type indices for each defined function, aligned with `defined_function_order`.
    pub defined_function_type_indices: Vec<u32>,
    /// Static-data segment offsets in linear memory.
    pub data_offsets: FxHashMap<WasmStaticDataId, u32>,
    /// Static-data segment lengths cached for literal helper calls.
    pub data_lengths: FxHashMap<WasmStaticDataId, u32>,
    /// First aligned address available for dynamic heap allocation.
    pub heap_base: u32,
    /// `heap_top` mutable global index when runtime helpers are emitted.
    pub heap_top_global_index: Option<u32>,
}

pub(crate) fn build_emit_plan(
    module: &WasmLirModule,
    request: &WasmBackendRequest,
) -> Result<WasmEmitPlan, CompilerError> {
    // WHAT: plan all shared index spaces up front.
    // WHY: Wasm sections cross-reference by index, so deterministic assignment must happen
    // before any section payload is encoded.
    let mut type_entries = Vec::new();
    let mut type_index_by_signature = FxHashMap::default();
    let mut import_function_indices = FxHashMap::default();
    let mut function_indices = FxHashMap::default();
    let mut helper_indices = FxHashMap::default();
    let mut defined_function_order = Vec::new();
    let mut defined_function_type_indices = Vec::new();

    let mut imports = module.imports.iter().collect::<Vec<_>>();
    imports.sort_by_key(|import| import.id.0);

    // WHAT: function import indices always occupy the prefix of the function index space.
    // WHY: this is required by the Wasm binary model and keeps call/export mapping stable.
    let mut next_function_index = 0u32;
    for import in imports {
        let WasmImportKind::Function(signature) = &import.kind;

        intern_signature(signature, &mut type_entries, &mut type_index_by_signature);
        import_function_indices.insert(import.id, next_function_index);
        next_function_index += 1;
    }

    let mut lir_functions = module.functions.iter().collect::<Vec<_>>();
    lir_functions.sort_by_key(|function| function.id.0);

    // WHAT: assign indices to defined LIR functions by stable function id.
    // WHY: deterministic ordering improves debugging and test reproducibility.
    for function in &lir_functions {
        let type_index = intern_signature(
            &function.signature,
            &mut type_entries,
            &mut type_index_by_signature,
        );
        function_indices.insert(function.id, next_function_index);
        defined_function_order.push(DefinedFunctionKey::Lir(function.id));
        defined_function_type_indices.push(type_index);
        next_function_index += 1;
    }

    let should_emit_helpers =
        module_uses_runtime_helpers(module) || helper_exports_requested(request);
    if should_emit_helpers {
        // WHAT: helper ordering is fixed and independent of usage count.
        // WHY: stable helper indices simplify wrapper exports and future host glue assumptions.
        for helper in helper_emit_order() {
            let signature = helper_signature(helper);
            let type_index =
                intern_signature(&signature, &mut type_entries, &mut type_index_by_signature);
            helper_indices.insert(helper, next_function_index);
            defined_function_order.push(DefinedFunctionKey::Helper(helper));
            defined_function_type_indices.push(type_index);
            next_function_index += 1;
        }
    }

    let StaticDataLayoutResult {
        data_offsets,
        data_lengths,
        heap_base,
    } = plan_static_data_layout(module)?;
    let heap_top_global_index = should_emit_helpers.then_some(0);

    Ok(WasmEmitPlan {
        type_entries,
        type_index_by_signature,
        import_function_indices,
        function_indices,
        helper_indices,
        defined_function_order,
        defined_function_type_indices,
        data_offsets,
        data_lengths,
        heap_base,
        heap_top_global_index,
    })
}

pub(crate) fn helper_emit_order() -> [WasmRuntimeHelper; 15] {
    // WHAT: canonical helper declaration order.
    // WHY: helper function indices must be deterministic for stable exports/debug output.
    [
        WasmRuntimeHelper::Alloc,
        WasmRuntimeHelper::StringNewBuffer,
        WasmRuntimeHelper::StringPushLiteral,
        WasmRuntimeHelper::StringPushHandle,
        WasmRuntimeHelper::StringFinish,
        WasmRuntimeHelper::StringPtr,
        WasmRuntimeHelper::StringLen,
        WasmRuntimeHelper::StringEqual,
        WasmRuntimeHelper::StringFromI64,
        WasmRuntimeHelper::VecNew,
        WasmRuntimeHelper::VecPushHandle,
        WasmRuntimeHelper::VecLen,
        WasmRuntimeHelper::VecGet,
        WasmRuntimeHelper::Release,
        WasmRuntimeHelper::DropIfOwned,
    ]
}

pub(crate) fn helper_signature(helper: WasmRuntimeHelper) -> WasmLirSignature {
    use WasmAbiType::{Handle, I32, I64};

    match helper {
        WasmRuntimeHelper::Alloc => WasmLirSignature {
            params: vec![I32],
            results: vec![I32],
        },
        WasmRuntimeHelper::StringNewBuffer => WasmLirSignature {
            params: vec![],
            results: vec![Handle],
        },
        WasmRuntimeHelper::StringPushLiteral => WasmLirSignature {
            params: vec![Handle, I32, I32],
            results: vec![],
        },
        WasmRuntimeHelper::StringPushHandle => WasmLirSignature {
            params: vec![Handle, Handle],
            results: vec![],
        },
        WasmRuntimeHelper::StringFinish => WasmLirSignature {
            params: vec![Handle],
            results: vec![Handle],
        },
        WasmRuntimeHelper::StringPtr => WasmLirSignature {
            params: vec![Handle],
            results: vec![I32],
        },
        WasmRuntimeHelper::StringLen => WasmLirSignature {
            params: vec![Handle],
            results: vec![I32],
        },
        WasmRuntimeHelper::StringEqual => WasmLirSignature {
            params: vec![Handle, Handle],
            results: vec![I32],
        },
        WasmRuntimeHelper::StringFromI64 => WasmLirSignature {
            params: vec![I64],
            results: vec![Handle],
        },
        WasmRuntimeHelper::VecNew => WasmLirSignature {
            params: vec![],
            results: vec![Handle],
        },
        WasmRuntimeHelper::VecPushHandle => WasmLirSignature {
            params: vec![Handle, Handle],
            results: vec![],
        },
        WasmRuntimeHelper::VecLen => WasmLirSignature {
            params: vec![Handle],
            results: vec![I32],
        },
        WasmRuntimeHelper::VecGet => WasmLirSignature {
            params: vec![Handle, I32],
            results: vec![Handle],
        },
        WasmRuntimeHelper::Release => WasmLirSignature {
            params: vec![Handle],
            results: vec![],
        },
        WasmRuntimeHelper::DropIfOwned => WasmLirSignature {
            params: vec![Handle],
            results: vec![],
        },
    }
}

pub(crate) fn helper_exports_requested(request: &WasmBackendRequest) -> bool {
    let helpers = &request.export_policy.helper_exports;
    helpers.export_memory
        || helpers.export_str_ptr
        || helpers.export_str_len
        || helpers.export_vec_new
        || helpers.export_vec_push
        || helpers.export_vec_len
        || helpers.export_vec_get
        || helpers.export_release
}

pub(crate) fn helper_name(helper: WasmRuntimeHelper) -> &'static str {
    match helper {
        WasmRuntimeHelper::Alloc => "rt_alloc",
        WasmRuntimeHelper::StringNewBuffer => "rt_string_new_buffer",
        WasmRuntimeHelper::StringPushLiteral => "rt_string_push_literal",
        WasmRuntimeHelper::StringPushHandle => "rt_string_push_handle",
        WasmRuntimeHelper::StringFinish => "rt_string_finish",
        WasmRuntimeHelper::StringPtr => "rt_string_ptr",
        WasmRuntimeHelper::StringLen => "rt_string_len",
        WasmRuntimeHelper::StringEqual => "rt_string_equal",
        WasmRuntimeHelper::StringFromI64 => "rt_string_from_i64",
        WasmRuntimeHelper::VecNew => "rt_vec_new",
        WasmRuntimeHelper::VecPushHandle => "rt_vec_push_handle",
        WasmRuntimeHelper::VecLen => "rt_vec_len",
        WasmRuntimeHelper::VecGet => "rt_vec_get",
        WasmRuntimeHelper::Release => "rt_release",
        WasmRuntimeHelper::DropIfOwned => "rt_drop_if_owned",
    }
}

pub(crate) fn plan_sections_text(module: &WasmLirModule, plan: &WasmEmitPlan) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Wasm section plan");
    let _ = writeln!(out, "  type: {}", plan.type_entries.len());
    let _ = writeln!(out, "  import: {}", module.imports.len());
    let _ = writeln!(out, "  function: {}", plan.defined_function_order.len());
    let _ = writeln!(out, "  memory: 1");
    let _ = writeln!(
        out,
        "  global: {}",
        usize::from(plan.heap_top_global_index.is_some())
    );
    // Includes only LIR-declared exports. Helper exports are controlled by request policy.
    let _ = writeln!(out, "  export: {}", module.exports.len());
    let _ = writeln!(out, "  code: {}", plan.defined_function_order.len());
    let _ = writeln!(out, "  data: {}", module.static_data.len());
    out
}

pub(crate) fn plan_indices_text(
    module: &WasmLirModule,
    plan: &WasmEmitPlan,
    lir_functions: &[&WasmLirFunction],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Wasm index maps");

    let mut signatures = plan
        .type_index_by_signature
        .iter()
        .map(|(signature, index)| (*index, signature))
        .collect::<Vec<_>>();
    signatures.sort_by_key(|(index, _)| *index);
    for (index, signature) in signatures {
        let _ = writeln!(
            out,
            "  type[{index}] params={:?} results={:?}",
            signature.params, signature.results
        );
    }

    let mut imports = module.imports.iter().collect::<Vec<_>>();
    imports.sort_by_key(|import| import.id.0);
    for import in imports {
        if let Some(index) = plan.import_function_indices.get(&import.id) {
            let _ = writeln!(
                out,
                "  import.func[{index}] {:?} {}.{}",
                import.id, import.module_name, import.item_name
            );
        }
    }

    for function in lir_functions {
        if let Some(index) = plan.function_indices.get(&function.id) {
            let _ = writeln!(
                out,
                "  func[{index}] {:?} {}",
                function.id, function.debug_name
            );
        }
    }

    let mut helpers = plan.helper_indices.iter().collect::<Vec<_>>();
    helpers.sort_by_key(|(_, index)| **index);
    for (helper, index) in helpers {
        let _ = writeln!(out, "  helper[{index}] {}", helper_name(*helper));
    }

    if let Some(global_index) = plan.heap_top_global_index {
        let _ = writeln!(out, "  global[{global_index}] heap_top");
    }

    out
}

pub(crate) fn plan_data_layout_text(module: &WasmLirModule, plan: &WasmEmitPlan) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Wasm data layout");

    let mut static_data = module.static_data.iter().collect::<Vec<_>>();
    static_data.sort_by_key(|segment| segment.id.0);
    for segment in static_data {
        if let Some(offset) = plan.data_offsets.get(&segment.id).copied() {
            let _ = writeln!(
                out,
                "  data[{}] offset={} len={} name={}",
                segment.id.0,
                offset,
                segment.bytes.len(),
                segment.debug_name
            );
        } else {
            let _ = writeln!(
                out,
                "  data[{}] offset=<missing> len={} name={}",
                segment.id.0,
                segment.bytes.len(),
                segment.debug_name
            );
        }
    }
    let _ = writeln!(out, "  heap_base={}", plan.heap_base);
    out
}

fn module_uses_runtime_helpers(module: &WasmLirModule) -> bool {
    // WHAT: scan for statements that require synthesized runtime helper bodies.
    // WHY: helper emission should be demand-driven so modules without helper operations stay lean.
    for function in &module.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                if matches!(
                    statement,
                    WasmLirStmt::StringNewBuffer { .. }
                        | WasmLirStmt::StringPushLiteral { .. }
                        | WasmLirStmt::StringPushHandle { .. }
                        | WasmLirStmt::StringFromI64 { .. }
                        | WasmLirStmt::StringFinish { .. }
                        | WasmLirStmt::StringEq { .. }
                        | WasmLirStmt::StringNe { .. }
                        | WasmLirStmt::VecNew { .. }
                        | WasmLirStmt::VecPushHandle { .. }
                        | WasmLirStmt::DropIfOwned { .. }
                ) {
                    return true;
                }
            }
        }
    }

    false
}

struct StaticDataLayoutResult {
    data_offsets: FxHashMap<WasmStaticDataId, u32>,
    data_lengths: FxHashMap<WasmStaticDataId, u32>,
    heap_base: u32,
}

fn plan_static_data_layout(
    module: &WasmLirModule,
) -> Result<StaticDataLayoutResult, CompilerError> {
    // WHAT: place static segments by stable `WasmStaticDataId`, aligned to 8 bytes.
    // WHY: deterministic layout keeps literal pointer tests reproducible and preserves
    // handle/pointer alignment assumptions for runtime helpers.
    let mut data_offsets = FxHashMap::default();
    let mut data_lengths = FxHashMap::default();

    let mut static_data = module.static_data.iter().collect::<Vec<_>>();
    static_data.sort_by_key(|segment| segment.id.0);

    let mut cursor = module.memory_plan.static_data_base;
    for segment in static_data {
        cursor = align_to(cursor, 8);
        data_offsets.insert(segment.id, cursor);
        data_lengths.insert(segment.id, segment.bytes.len() as u32);
        cursor = cursor
            .checked_add(segment.bytes.len() as u32)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                "Wasm static data layout overflowed u32 address space while planning data segments",
            )
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
            })?;
    }

    Ok(StaticDataLayoutResult {
        data_offsets,
        data_lengths,
        heap_base: align_to(cursor, 8),
    })
}

fn intern_signature(
    signature: &WasmLirSignature,
    type_entries: &mut Vec<WasmLirSignature>,
    type_index_by_signature: &mut FxHashMap<WasmLirSignature, u32>,
) -> u32 {
    // WHAT: deduplicate signatures in first-seen order.
    // WHY: this yields stable type indices across runs while avoiding duplicate entries.
    if let Some(existing) = type_index_by_signature.get(signature).copied() {
        return existing;
    }

    let index = type_entries.len() as u32;
    type_entries.push(signature.clone());
    type_index_by_signature.insert(signature.clone(), index);
    index
}

fn align_to(value: u32, alignment: u32) -> u32 {
    // WHAT: round `value` up to the next `alignment` boundary.
    // WHY: static data and heap base require predictable alignment guarantees.
    if alignment == 0 {
        return value;
    }

    let remainder = value % alignment;
    if remainder == 0 {
        value
    } else {
        value + (alignment - remainder)
    }
}
