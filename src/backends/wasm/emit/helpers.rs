//! Synthesized runtime helper function emission.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use wasm_encoder::{Function, Instruction, MemArg, ValType};

pub(crate) fn emit_helper_function(
    helper: WasmRuntimeHelper,
    plan: &WasmEmitPlan,
) -> Result<Function, CompilerError> {
    // WHAT: helpers share one bump-allocation/global model.
    // WHY: correctness-first runtime scaffolding until richer ownership/runtime logic lands.
    let heap_top_global = plan.heap_top_global_index.ok_or_else(|| {
        CompilerError::compiler_error(
            "Wasm emission expected heap_top global while synthesizing runtime helpers",
        )
        .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
    })?;

    let alloc_index = plan
        .helper_indices
        .get(&WasmRuntimeHelper::Alloc)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error("Wasm emission missing rt_alloc helper index")
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })?;

    // Vec-handle helpers have their own focused emitter.
    if matches!(
        helper,
        WasmRuntimeHelper::VecNew
            | WasmRuntimeHelper::VecPushHandle
            | WasmRuntimeHelper::VecLen
            | WasmRuntimeHelper::VecGet
    ) {
        return super::vec_helpers::emit_vec_helper(helper, alloc_index);
    }

    // Local declarations: each helper declares only non-parameter locals.
    // Named constants below document the local index layout per helper.
    let mut function = match helper {
        WasmRuntimeHelper::Alloc => {
            // param 0: size  |  local 1: old_top (scratch)
            Function::new(vec![(1, ValType::I32)])
        }
        WasmRuntimeHelper::StringNewBuffer => {
            // no params  |  local 0: buffer_handle (scratch)
            Function::new(vec![(1, ValType::I32)])
        }
        WasmRuntimeHelper::StringPushLiteral => {
            // param 0: buffer, param 1: src_ptr, param 2: src_len
            // local 3: new_len, local 4: new_cap, local 5: new_region
            Function::new(vec![(3, ValType::I32)])
        }
        WasmRuntimeHelper::StringPushHandle => {
            // param 0: buffer, param 1: source_handle
            // local 2: src_ptr, local 3: src_len, local 4: new_len, local 5: new_cap, local 6: new_region
            Function::new(vec![(5, ValType::I32)])
        }
        WasmRuntimeHelper::StringFinish => {
            // param 0: buffer  |  local 1: result_handle (scratch)
            Function::new(vec![(1, ValType::I32)])
        }
        WasmRuntimeHelper::StringEqual => {
            // param 0: lhs handle, param 1: rhs handle
            // locals 2..7: lhs_ptr, rhs_ptr, lhs_len, rhs_len, index, result
            Function::new(vec![(6, ValType::I32)])
        }
        WasmRuntimeHelper::StringFromI64 => {
            // param 0: value_i64 | local 1: buffer_handle
            Function::new(vec![(1, ValType::I32)])
        }
        WasmRuntimeHelper::StringPtr
        | WasmRuntimeHelper::StringLen
        | WasmRuntimeHelper::Release
        | WasmRuntimeHelper::DropIfOwned => Function::new(Vec::new()),
        WasmRuntimeHelper::VecNew
        | WasmRuntimeHelper::VecPushHandle
        | WasmRuntimeHelper::VecLen
        | WasmRuntimeHelper::VecGet => {
            unreachable!("vec helpers are dispatched early to vec_helpers::emit_vec_helper")
        }
    };

    match helper {
        WasmRuntimeHelper::Alloc => {
            // WHAT: return current heap_top and then advance by requested size.
            // WHY: simple monotonic bump allocator for runtime objects.
            const SIZE: u32 = 0;
            const OLD_TOP: u32 = 1;

            function.instruction(&Instruction::GlobalGet(heap_top_global));
            function.instruction(&Instruction::LocalTee(OLD_TOP));
            function.instruction(&Instruction::LocalGet(SIZE));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::GlobalSet(heap_top_global));
            function.instruction(&Instruction::LocalGet(OLD_TOP));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringNewBuffer => {
            // WHAT: allocate and initialize a 12-byte `{content_ptr, content_len, capacity}` buffer header.
            // WHY: the 3-field layout supports true append semantics for multi-fragment concatenation.
            //
            // Buffer layout (12 bytes):
            //   offset 0: content_ptr  (i32) — pointer to accumulated bytes region
            //   offset 4: content_len  (i32) — current byte count
            //   offset 8: capacity     (i32) — allocated byte capacity of the content region
            const BUFFER_HANDLE: u32 = 0;

            function.instruction(&Instruction::I32Const(12));
            function.instruction(&Instruction::Call(alloc_index));
            function.instruction(&Instruction::LocalTee(BUFFER_HANDLE));
            // Initialize content_ptr = 0
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32Store(memarg(0)));
            function.instruction(&Instruction::LocalGet(BUFFER_HANDLE));
            // Initialize content_len = 0
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32Store(memarg(4)));
            function.instruction(&Instruction::LocalGet(BUFFER_HANDLE));
            // Initialize capacity = 0
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32Store(memarg(8)));
            function.instruction(&Instruction::LocalGet(BUFFER_HANDLE));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringPushLiteral => {
            // WHAT: append static literal bytes into the buffer's content region.
            // WHY: true append semantics — multi-fragment templates produce concatenated output.
            const BUFFER: u32 = 0;
            const SRC_PTR: u32 = 1;
            const SRC_LEN: u32 = 2;
            const NEW_LEN: u32 = 3;
            const NEW_CAP: u32 = 4;
            const NEW_REGION: u32 = 5;

            // new_len = content_len + src_len
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::LocalGet(SRC_LEN));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalSet(NEW_LEN));

            // if new_len > capacity, grow
            function.instruction(&Instruction::LocalGet(NEW_LEN));
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(8)));
            function.instruction(&Instruction::I32GtU);
            function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                // new_cap = new_len * 2 (simple growth policy)
                function.instruction(&Instruction::LocalGet(NEW_LEN));
                function.instruction(&Instruction::I32Const(2));
                function.instruction(&Instruction::I32Mul);
                function.instruction(&Instruction::LocalSet(NEW_CAP));

                // new_region = rt_alloc(new_cap)
                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::Call(alloc_index));
                function.instruction(&Instruction::LocalSet(NEW_REGION));

                // memory.copy(new_region, old content_ptr, content_len)
                function.instruction(&Instruction::LocalGet(NEW_REGION));
                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::I32Load(memarg(0)));
                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::I32Load(memarg(4)));
                function.instruction(&Instruction::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });

                // buffer.content_ptr = new_region
                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::LocalGet(NEW_REGION));
                function.instruction(&Instruction::I32Store(memarg(0)));

                // buffer.capacity = new_cap
                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::I32Store(memarg(8)));
            }
            function.instruction(&Instruction::End);

            // memory.copy(content_ptr + content_len, src_ptr, src_len)
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalGet(SRC_PTR));
            function.instruction(&Instruction::LocalGet(SRC_LEN));
            function.instruction(&Instruction::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            });

            // buffer.content_len = new_len
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::LocalGet(NEW_LEN));
            function.instruction(&Instruction::I32Store(memarg(4)));

            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringPushHandle => {
            // WHAT: read {ptr, len} from a finalized string handle and append those bytes.
            // WHY: handle concatenation uses the same grow+copy model as literal push.
            const BUFFER: u32 = 0;
            const SOURCE_HANDLE: u32 = 1;
            const SRC_PTR: u32 = 2;
            const SRC_LEN: u32 = 3;
            const NEW_LEN: u32 = 4;
            const NEW_CAP: u32 = 5;
            const NEW_REGION: u32 = 6;

            // src_ptr = source_handle.ptr (offset 0)
            function.instruction(&Instruction::LocalGet(SOURCE_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalSet(SRC_PTR));
            // src_len = source_handle.len (offset 4)
            function.instruction(&Instruction::LocalGet(SOURCE_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::LocalSet(SRC_LEN));

            // new_len = content_len + src_len
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::LocalGet(SRC_LEN));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalSet(NEW_LEN));

            // if new_len > capacity, grow
            function.instruction(&Instruction::LocalGet(NEW_LEN));
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(8)));
            function.instruction(&Instruction::I32GtU);
            function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                function.instruction(&Instruction::LocalGet(NEW_LEN));
                function.instruction(&Instruction::I32Const(2));
                function.instruction(&Instruction::I32Mul);
                function.instruction(&Instruction::LocalSet(NEW_CAP));

                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::Call(alloc_index));
                function.instruction(&Instruction::LocalSet(NEW_REGION));

                function.instruction(&Instruction::LocalGet(NEW_REGION));
                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::I32Load(memarg(0)));
                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::I32Load(memarg(4)));
                function.instruction(&Instruction::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });

                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::LocalGet(NEW_REGION));
                function.instruction(&Instruction::I32Store(memarg(0)));

                function.instruction(&Instruction::LocalGet(BUFFER));
                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::I32Store(memarg(8)));
            }
            function.instruction(&Instruction::End);

            // memory.copy(content_ptr + content_len, src_ptr, src_len)
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalGet(SRC_PTR));
            function.instruction(&Instruction::LocalGet(SRC_LEN));
            function.instruction(&Instruction::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            });

            // buffer.content_len = new_len
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::LocalGet(NEW_LEN));
            function.instruction(&Instruction::I32Store(memarg(4)));

            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringFinish => {
            // WHAT: allocate 8-byte finalized string handle {ptr, len} from buffer state.
            // WHY: finalized strings have a simpler 2-field layout for host ABI compatibility.
            //
            // Finalized string layout (8 bytes):
            //   offset 0: ptr (i32) — pointer to UTF-8 byte content
            //   offset 4: len (i32) — byte length
            const BUFFER: u32 = 0;
            const RESULT_HANDLE: u32 = 1;

            // result_handle = rt_alloc(8)
            function.instruction(&Instruction::I32Const(8));
            function.instruction(&Instruction::Call(alloc_index));
            function.instruction(&Instruction::LocalSet(RESULT_HANDLE));

            // result_handle.ptr = buffer.content_ptr
            function.instruction(&Instruction::LocalGet(RESULT_HANDLE));
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::I32Store(memarg(0)));

            // result_handle.len = buffer.content_len
            function.instruction(&Instruction::LocalGet(RESULT_HANDLE));
            function.instruction(&Instruction::LocalGet(BUFFER));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::I32Store(memarg(4)));

            function.instruction(&Instruction::LocalGet(RESULT_HANDLE));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringPtr => {
            // WHAT: read ptr field from finalized 8-byte string handle.
            const HANDLE: u32 = 0;
            function.instruction(&Instruction::LocalGet(HANDLE));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringLen => {
            // WHAT: read len field from finalized 8-byte string handle.
            const HANDLE: u32 = 0;
            function.instruction(&Instruction::LocalGet(HANDLE));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringEqual => {
            // WHAT: compare finalized strings by UTF-8 byte content.
            // WHY: handles are addresses, so identity comparison would make equal strings
            // compare unequal whenever they were materialized through different allocations.
            const LHS: u32 = 0;
            const RHS: u32 = 1;
            const LHS_PTR: u32 = 2;
            const RHS_PTR: u32 = 3;
            const LHS_LEN: u32 = 4;
            const RHS_LEN: u32 = 5;
            const INDEX: u32 = 6;
            const RESULT: u32 = 7;

            // Read the two finalized `{ptr, len}` headers.
            function.instruction(&Instruction::LocalGet(LHS));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalSet(LHS_PTR));
            function.instruction(&Instruction::LocalGet(RHS));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalSet(RHS_PTR));
            function.instruction(&Instruction::LocalGet(LHS));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::LocalSet(LHS_LEN));
            function.instruction(&Instruction::LocalGet(RHS));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::LocalSet(RHS_LEN));

            // RESULT is set on every path that leaves the comparison loop.
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::LocalSet(RESULT));

            // A length mismatch cannot compare equal. Otherwise compare each UTF-8 byte.
            function.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            function.instruction(&Instruction::LocalGet(LHS_LEN));
            function.instruction(&Instruction::LocalGet(RHS_LEN));
            function.instruction(&Instruction::I32Ne);
            function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            function.instruction(&Instruction::Br(1));
            function.instruction(&Instruction::End);

            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::LocalSet(INDEX));
            function.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

            // End-of-input after equal lengths means the strings are equal.
            function.instruction(&Instruction::LocalGet(INDEX));
            function.instruction(&Instruction::LocalGet(LHS_LEN));
            function.instruction(&Instruction::I32GeU);
            function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::LocalSet(RESULT));
            function.instruction(&Instruction::Br(2));
            function.instruction(&Instruction::End);

            // Compare one unsigned byte from each UTF-8 content region.
            function.instruction(&Instruction::LocalGet(LHS_PTR));
            function.instruction(&Instruction::LocalGet(INDEX));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::I32Load8U(byte_memarg(0)));
            function.instruction(&Instruction::LocalGet(RHS_PTR));
            function.instruction(&Instruction::LocalGet(INDEX));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::I32Load8U(byte_memarg(0)));
            function.instruction(&Instruction::I32Ne);
            function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            function.instruction(&Instruction::Br(2));
            function.instruction(&Instruction::End);

            function.instruction(&Instruction::LocalGet(INDEX));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalSet(INDEX));
            function.instruction(&Instruction::Br(0));
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::LocalGet(RESULT));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::StringFromI64 => {
            // WHAT: materialize a string handle from an i64 interpolation chunk.
            // WHY: frontend template coercion currently models numeric->string as `"" + value`.
            // A dedicated helper keeps lowering deterministic while scalar formatting support is
            // incrementally implemented.
            let string_new_buffer_index = plan
                .helper_indices
                .get(&WasmRuntimeHelper::StringNewBuffer)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Wasm emission missing rt_string_new_buffer helper index",
                    )
                    .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
                })?;
            let string_finish_index = plan
                .helper_indices
                .get(&WasmRuntimeHelper::StringFinish)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Wasm emission missing rt_string_finish helper index",
                    )
                    .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
                })?;

            const VALUE_I64: u32 = 0;
            const BUFFER_HANDLE: u32 = 1;

            // Keep the input value consumed so helper semantics remain explicit.
            function.instruction(&Instruction::LocalGet(VALUE_I64));
            function.instruction(&Instruction::Drop);

            // Temporary phase behavior: emit an empty finalized string handle.
            function.instruction(&Instruction::Call(string_new_buffer_index));
            function.instruction(&Instruction::LocalSet(BUFFER_HANDLE));
            function.instruction(&Instruction::LocalGet(BUFFER_HANDLE));
            function.instruction(&Instruction::Call(string_finish_index));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::Release | WasmRuntimeHelper::DropIfOwned => {
            // WHAT: release/drop helpers are conservative no-ops for both string and vec handles.
            // WHY: ownership-eliding/free semantics are introduced incrementally after baseline correctness.
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::VecNew
        | WasmRuntimeHelper::VecPushHandle
        | WasmRuntimeHelper::VecLen
        | WasmRuntimeHelper::VecGet => {
            unreachable!("vec helpers are dispatched early to vec_helpers::emit_vec_helper")
        }
    }

    function.instruction(&Instruction::End);
    Ok(function)
}

fn memarg(offset: u64) -> MemArg {
    // WHAT: all helper runtime memory accesses target memory index 0 with 4-byte alignment.
    // WHY: uses one internal 32-bit linear memory and i32 load/store fields.
    MemArg {
        offset,
        align: 2,
        memory_index: 0,
    }
}

fn byte_memarg(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 0,
        memory_index: 0,
    }
}
