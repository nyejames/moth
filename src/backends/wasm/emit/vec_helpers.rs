//! Vec-handle runtime helper emission.
//!
//! WHAT: synthesizes the four vec-handle helpers used by the moth_start() -> Vec<String> ABI:
//!   moth_vec_new, moth_vec_push, moth_vec_len, moth_vec_get
//!
//! WHY: separated from helpers.rs to keep per-helper emission focused on one handle type.
//! Vec handles are the runtime accumulator for entry start() fragment strings only — this is
//! not generic collection lowering.
//!
//! Vec handle layout (12 bytes):
//!   offset 0: data_ptr  (i32) — pointer to contiguous i32 string-handle array
//!   offset 4: len       (i32) — logical element count
//!   offset 8: capacity  (i32) — allocated element capacity (in elements, not bytes)

use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use wasm_encoder::{Function, Instruction, MemArg, ValType};

/// Emit the function body for one of the four vec-handle runtime helpers.
///
/// `alloc_index` is the Wasm function index of `rt_alloc`, required by VecNew and VecPushHandle.
pub(crate) fn emit_vec_helper(
    helper: WasmRuntimeHelper,
    alloc_index: u32,
) -> Result<Function, CompilerError> {
    let mut function = match helper {
        WasmRuntimeHelper::VecNew => {
            // no params  |  local 0: vec_handle
            Function::new(vec![(1, ValType::I32)])
        }
        WasmRuntimeHelper::VecPushHandle => {
            // param 0: vec_handle, param 1: string_handle
            // local 2: new_len, local 3: new_cap, local 4: new_region, local 5: data_ptr
            Function::new(vec![(4, ValType::I32)])
        }
        WasmRuntimeHelper::VecLen | WasmRuntimeHelper::VecGet => Function::new(Vec::new()),
        _ => {
            return Err(CompilerError::compiler_error(
                "emit_vec_helper called with non-vec helper variant",
            ));
        }
    };

    match helper {
        WasmRuntimeHelper::VecNew => {
            // WHAT: allocate a 12-byte vec header {data_ptr, len, capacity}.
            // WHY: entry start() returns Vec<String>, so runtime fragment accumulation
            //      needs a stable handle layout parallel to string buffers.
            const VEC_HANDLE: u32 = 0;

            function.instruction(&Instruction::I32Const(12));
            function.instruction(&Instruction::Call(alloc_index));
            function.instruction(&Instruction::LocalTee(VEC_HANDLE));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32Store(memarg(0)));
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32Store(memarg(4)));
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Const(0));
            function.instruction(&Instruction::I32Store(memarg(8)));
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::VecPushHandle => {
            const VEC_HANDLE: u32 = 0;
            const STRING_HANDLE: u32 = 1;
            const NEW_LEN: u32 = 2;
            const NEW_CAP: u32 = 3;
            const NEW_REGION: u32 = 4;
            const DATA_PTR: u32 = 5;

            // new_len = len + 1
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalSet(NEW_LEN));

            // if new_len > capacity, grow backing storage
            function.instruction(&Instruction::LocalGet(NEW_LEN));
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(8)));
            function.instruction(&Instruction::I32GtU);
            function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                // new_cap = max(new_len * 2, 4)
                function.instruction(&Instruction::LocalGet(NEW_LEN));
                function.instruction(&Instruction::I32Const(2));
                function.instruction(&Instruction::I32Mul);
                function.instruction(&Instruction::LocalSet(NEW_CAP));
                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::I32Const(4));
                function.instruction(&Instruction::I32LtU);
                function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    function.instruction(&Instruction::I32Const(4));
                    function.instruction(&Instruction::LocalSet(NEW_CAP));
                }
                function.instruction(&Instruction::End);

                // new_region = rt_alloc(new_cap * 4)
                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::I32Const(4));
                function.instruction(&Instruction::I32Mul);
                function.instruction(&Instruction::Call(alloc_index));
                function.instruction(&Instruction::LocalSet(NEW_REGION));

                // Copy existing elements if any.
                function.instruction(&Instruction::LocalGet(VEC_HANDLE));
                function.instruction(&Instruction::I32Load(memarg(4)));
                function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    function.instruction(&Instruction::LocalGet(NEW_REGION));
                    function.instruction(&Instruction::LocalGet(VEC_HANDLE));
                    function.instruction(&Instruction::I32Load(memarg(0)));
                    function.instruction(&Instruction::LocalGet(VEC_HANDLE));
                    function.instruction(&Instruction::I32Load(memarg(4)));
                    function.instruction(&Instruction::I32Const(4));
                    function.instruction(&Instruction::I32Mul);
                    function.instruction(&Instruction::MemoryCopy {
                        src_mem: 0,
                        dst_mem: 0,
                    });
                }
                function.instruction(&Instruction::End);

                function.instruction(&Instruction::LocalGet(VEC_HANDLE));
                function.instruction(&Instruction::LocalGet(NEW_REGION));
                function.instruction(&Instruction::I32Store(memarg(0)));

                function.instruction(&Instruction::LocalGet(VEC_HANDLE));
                function.instruction(&Instruction::LocalGet(NEW_CAP));
                function.instruction(&Instruction::I32Store(memarg(8)));
            }
            function.instruction(&Instruction::End);

            // data_ptr = vec.data_ptr
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalSet(DATA_PTR));

            // data_ptr[len * 4] = string_handle
            function.instruction(&Instruction::LocalGet(DATA_PTR));
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::I32Const(4));
            function.instruction(&Instruction::I32Mul);
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalGet(STRING_HANDLE));
            function.instruction(&Instruction::I32Store(memarg(0)));

            // vec.len = new_len
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::LocalGet(NEW_LEN));
            function.instruction(&Instruction::I32Store(memarg(4)));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::VecLen => {
            const VEC_HANDLE: u32 = 0;
            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(4)));
            function.instruction(&Instruction::Return);
        }
        WasmRuntimeHelper::VecGet => {
            const VEC_HANDLE: u32 = 0;
            const INDEX: u32 = 1;

            function.instruction(&Instruction::LocalGet(VEC_HANDLE));
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::LocalGet(INDEX));
            function.instruction(&Instruction::I32Const(4));
            function.instruction(&Instruction::I32Mul);
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::I32Load(memarg(0)));
            function.instruction(&Instruction::Return);
        }
        _ => {}
    }

    function.instruction(&Instruction::End);
    Ok(function)
}

fn memarg(offset: u64) -> MemArg {
    // WHAT: all vec helper accesses target memory index 0 with 4-byte alignment.
    MemArg {
        offset,
        align: 2,
        memory_index: 0,
    }
}
