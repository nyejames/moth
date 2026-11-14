;; Staged-text protocol. Plain core Wasm, not compiled Moth.
(module
  (import "host" "stage" (func $stage (result i64)))
  (import "host" "record" (func $record (param i32)))
  (import "moth-abi-v1" "staged_text_take" (func $take (param i64 i32 i32)))
  (memory (export "memory") 1)

  (func (export "take_staged") (param $destination i32) (param $length i32)
    (call $take (call $stage) (local.get $destination) (local.get $length)))

  (func (export "take_twice")
    (local $ticket i64)
    (local.set $ticket (call $stage))
    (call $take (local.get $ticket) (i32.const 0) (i32.const 5))
    (call $take (local.get $ticket) (i32.const 0) (i32.const 5)))

  (func (export "load_byte") (param $address i32) (result i32)
    (i32.load8_u (local.get $address)))

  ;; Leaves the ticket live and then enters another host import.
  (func (export "stage_then_record")
    (drop (call $stage))
    (call $record (i32.const 1)))

  ;; Leaves the ticket live when the entry returns.
  (func (export "stage_and_return")
    (drop (call $stage))))
