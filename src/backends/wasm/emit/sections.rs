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
use crate::backends::wasm::runtime::memory::WasmScalarStorageKind;
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
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
    /// Ryu power-table segment offset and byte length when Float formatting is used.
    pub float_format_tables_offset: Option<u32>,
    pub float_format_tables_len: Option<u32>,
    pub float_format_tables: Option<Vec<u8>>,
    /// Initial memory size adjusted so every emitted active data segment is in bounds.
    pub initial_memory_pages: u32,
    /// First aligned address available for dynamic heap allocation.
    pub heap_base: u32,
    /// `heap_top` mutable global index when memory runtime helpers are emitted.
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

    let helper_requirements =
        runtime_helper_requirements(module, helper_exports_requested(request));
    let memory_helpers_needed = helper_requirements.memory;
    let float_power_needed = helper_requirements.float_power;
    let float_remainder_needed = helper_requirements.float_remainder;
    let f32_to_f16_bits_needed = helper_requirements.f32_to_f16_bits;
    let f16_bits_to_f32_needed = helper_requirements.f16_bits_to_f32;
    // WHAT: helper ordering is fixed and independent of usage count.
    // WHY: stable helper function indices simplify exports and debug output.
    let float_format_needed = helper_requirements.float_format;
    for helper in helper_emit_order() {
        let helper_is_needed = match helper {
            WasmRuntimeHelper::FloatPower => float_power_needed,
            WasmRuntimeHelper::FloatRemainder => float_remainder_needed,
            WasmRuntimeHelper::F32ToF16Bits => f32_to_f16_bits_needed,
            WasmRuntimeHelper::F16BitsToF32 => f16_bits_to_f32_needed,
            WasmRuntimeHelper::FloatToDecimal | WasmRuntimeHelper::StringFromFloat => {
                float_format_needed
            }
            _ => memory_helpers_needed,
        };
        if !helper_is_needed {
            continue;
        }

        let signature = helper_signature(helper);
        let type_index =
            intern_signature(&signature, &mut type_entries, &mut type_index_by_signature);
        helper_indices.insert(helper, next_function_index);
        defined_function_order.push(DefinedFunctionKey::Helper(helper));
        defined_function_type_indices.push(type_index);
        next_function_index += 1;
    }

    let StaticDataLayoutResult {
        data_offsets,
        data_lengths,
        float_format_tables_offset,
        float_format_tables,
        float_format_tables_len,
        heap_base,
    } = plan_static_data_layout(module, float_format_needed)?;
    let heap_top_global_index = memory_helpers_needed.then_some(0);
    let initial_memory_pages = if float_format_needed {
        let required_pages = heap_base / 65_536 + (heap_base % 65_536 != 0) as u32;
        module.memory_plan.initial_pages.max(required_pages)
    } else {
        module.memory_plan.initial_pages
    };
    if float_format_needed
        && let Some(max_pages) = module.memory_plan.max_pages
        && initial_memory_pages > max_pages
    {
        return Err(CompilerError::compiler_error(
            "Wasm Float formatting tables exceed the configured maximum memory",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration)));
    }

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
        float_format_tables_offset,
        float_format_tables_len,
        float_format_tables,
        initial_memory_pages,
        heap_base,
        heap_top_global_index,
    })
}

pub(crate) fn helper_emit_order() -> [WasmRuntimeHelper; 22] {
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
        WasmRuntimeHelper::StringFromU64,
        WasmRuntimeHelper::VecNew,
        WasmRuntimeHelper::VecPushHandle,
        WasmRuntimeHelper::VecLen,
        WasmRuntimeHelper::VecGet,
        WasmRuntimeHelper::Release,
        WasmRuntimeHelper::DropIfOwned,
        WasmRuntimeHelper::FloatPower,
        WasmRuntimeHelper::FloatRemainder,
        WasmRuntimeHelper::F32ToF16Bits,
        WasmRuntimeHelper::F16BitsToF32,
        WasmRuntimeHelper::FloatToDecimal,
        WasmRuntimeHelper::StringFromFloat,
    ]
}

pub(crate) fn helper_signature(helper: WasmRuntimeHelper) -> WasmLirSignature {
    use WasmAbiType::{F32, F64, Handle, I32, I64};

    match helper {
        WasmRuntimeHelper::FloatPower | WasmRuntimeHelper::FloatRemainder => WasmLirSignature {
            params: vec![F64, F64],
            results: vec![F64],
        },
        WasmRuntimeHelper::F32ToF16Bits => WasmLirSignature {
            params: vec![F32],
            results: vec![I32],
        },
        WasmRuntimeHelper::F16BitsToF32 => WasmLirSignature {
            params: vec![I32],
            results: vec![F32],
        },
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
        WasmRuntimeHelper::StringFromI64 | WasmRuntimeHelper::StringFromU64 => WasmLirSignature {
            params: vec![I64],
            results: vec![Handle],
        },
        WasmRuntimeHelper::FloatToDecimal => WasmLirSignature {
            params: vec![F64, I32],
            results: vec![I64, I32],
        },
        WasmRuntimeHelper::StringFromFloat => WasmLirSignature {
            params: vec![F64, I32],
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
        WasmRuntimeHelper::StringFromU64 => "rt_string_from_u64",
        WasmRuntimeHelper::VecNew => "rt_vec_new",
        WasmRuntimeHelper::VecPushHandle => "rt_vec_push_handle",
        WasmRuntimeHelper::VecLen => "rt_vec_len",
        WasmRuntimeHelper::VecGet => "rt_vec_get",
        WasmRuntimeHelper::Release => "rt_release",
        WasmRuntimeHelper::DropIfOwned => "rt_drop_if_owned",
        WasmRuntimeHelper::FloatPower => "rt_float_power",
        WasmRuntimeHelper::FloatRemainder => "rt_float_remainder",
        WasmRuntimeHelper::F32ToF16Bits => "rt_f32_to_f16_bits",
        WasmRuntimeHelper::F16BitsToF32 => "rt_f16_bits_to_f32",
        WasmRuntimeHelper::FloatToDecimal => "rt_float_to_decimal",
        WasmRuntimeHelper::StringFromFloat => "rt_string_from_float",
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
    let emitted_data_count =
        module.static_data.len() + usize::from(plan.float_format_tables_offset.is_some());
    let _ = writeln!(out, "  data: {emitted_data_count}");
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
    if let (Some(offset), Some(len)) = (
        plan.float_format_tables_offset,
        plan.float_format_tables_len,
    ) {
        let _ = writeln!(out, "  float_format_tables offset={offset} len={len}");
    }
    let _ = writeln!(out, "  heap_base={}", plan.heap_base);
    out
}

struct RuntimeHelperRequirements {
    memory: bool,
    float_power: bool,
    float_remainder: bool,
    f32_to_f16_bits: bool,
    f16_bits_to_f32: bool,
    float_format: bool,
}

fn runtime_helper_requirements(
    module: &WasmLirModule,
    memory_helpers_exported: bool,
) -> RuntimeHelperRequirements {
    let mut requirements = RuntimeHelperRequirements {
        memory: memory_helpers_exported,
        float_power: false,
        float_remainder: false,
        f32_to_f16_bits: false,
        f16_bits_to_f32: false,
        float_format: false,
    };

    for function in &module.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                match statement {
                    WasmLirStmt::RoundF16 { .. } => {
                        requirements.f32_to_f16_bits = true;
                        requirements.f16_bits_to_f32 = true;
                    }
                    WasmLirStmt::LoadScalar {
                        kind: WasmScalarStorageKind::F16,
                        ..
                    } => requirements.f16_bits_to_f32 = true,
                    WasmLirStmt::StoreScalar {
                        kind: WasmScalarStorageKind::F16,
                        ..
                    } => requirements.f32_to_f16_bits = true,
                    WasmLirStmt::StringNewBuffer { .. }
                    | WasmLirStmt::StringPushLiteral { .. }
                    | WasmLirStmt::StringPushHandle { .. }
                    | WasmLirStmt::StringFromI64 { .. }
                    | WasmLirStmt::StringFromU64 { .. }
                    | WasmLirStmt::StringFinish { .. }
                    | WasmLirStmt::StringEq { .. }
                    | WasmLirStmt::StringNe { .. }
                    | WasmLirStmt::VecNew { .. }
                    | WasmLirStmt::VecPushHandle { .. }
                    | WasmLirStmt::DropIfOwned { .. } => requirements.memory = true,
                    WasmLirStmt::StringFromFloat { .. } => {
                        requirements.memory = true;
                        requirements.float_format = true;
                    }
                    WasmLirStmt::CheckedFloatOp {
                        operator: NumericOperator::Power,
                        ..
                    } => requirements.float_power = true,
                    WasmLirStmt::CheckedFloatOp {
                        operator: NumericOperator::Remainder,
                        ..
                    } => requirements.float_remainder = true,
                    _ => {}
                }
            }
        }
    }

    requirements
}

struct StaticDataLayoutResult {
    data_offsets: FxHashMap<WasmStaticDataId, u32>,
    data_lengths: FxHashMap<WasmStaticDataId, u32>,
    float_format_tables_offset: Option<u32>,
    float_format_tables: Option<Vec<u8>>,
    float_format_tables_len: Option<u32>,
    heap_base: u32,
}

fn plan_static_data_layout(
    module: &WasmLirModule,
    float_format_needed: bool,
) -> Result<StaticDataLayoutResult, CompilerError> {
    // WHAT: place static segments by stable id, then append Ryu tables only for dynamic float
    // formatting. All segments are aligned to 8 bytes for deterministic heap placement.
    let mut data_offsets = FxHashMap::default();
    let mut data_lengths = FxHashMap::default();

    let mut static_data = module.static_data.iter().collect::<Vec<_>>();
    static_data.sort_by_key(|segment| segment.id.0);

    let mut cursor = module.memory_plan.static_data_base;
    for segment in static_data {
        cursor = align_to(cursor, 8).ok_or_else(|| {
            static_data_layout_error("Wasm static data layout overflowed while aligning a segment")
        })?;
        let len = u32::try_from(segment.bytes.len()).map_err(|_| {
            static_data_layout_error("Wasm static data segment length exceeded u32 address space")
        })?;
        data_offsets.insert(segment.id, cursor);
        data_lengths.insert(segment.id, len);
        cursor = cursor.checked_add(len).ok_or_else(|| {
            static_data_layout_error(
                "Wasm static data layout overflowed u32 address space while planning data segments",
            )
        })?;
    }

    cursor = align_to(cursor, 8).ok_or_else(|| {
        static_data_layout_error("Wasm static data layout overflowed while aligning Ryu tables")
    })?;
    let (float_format_tables_offset, float_format_tables, float_format_tables_len) =
        if float_format_needed {
            let tables = super::float_format::ryu_table_data();
            let len = tables.len() as u32;
            let offset = cursor;
            cursor = cursor.checked_add(len).ok_or_else(|| {
                static_data_layout_error("Wasm Ryu table layout overflowed u32 address space")
            })?;
            (Some(offset), Some(tables), Some(len))
        } else {
            (None, None, None)
        };

    Ok(StaticDataLayoutResult {
        data_offsets,
        data_lengths,
        float_format_tables_offset,
        float_format_tables,
        float_format_tables_len,
        heap_base: align_to(cursor, 8).ok_or_else(|| {
            static_data_layout_error("Wasm static data layout overflowed while aligning heap base")
        })?,
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

fn static_data_layout_error(message: &str) -> CompilerError {
    CompilerError::compiler_error(message)
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
}

fn align_to(value: u32, alignment: u32) -> Option<u32> {
    if alignment == 0 {
        return Some(value);
    }

    let remainder = value % alignment;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(alignment - remainder)
    }
}
