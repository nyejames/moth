;; Guest behaviours the runtime mechanics tests drive. Plain core Wasm, not the Moth host ABI.
(module
  (import "host" "read_value" (func $read_value (result i32)))
  (import "host" "record" (func $record (param i32)))
  (import "host" "advance_epoch" (func $advance_epoch))
  (import "host" "fail" (func $fail))
  (import "host" "panic" (func $panic))
  (import "host" "wrong_result_type" (func $wrong_result_type (result i32)))

  (memory (export "memory") 1)

  ;; Scalar round trip: an export argument combined with an imported result.
  (func (export "add_to_host_value") (param $addend i32) (result i32)
    (i32.add (call $read_value) (local.get $addend)))

  ;; Leaves a visible mark in host state, so a test can tell whether guest code ran.
  (func (export "record") (param $value i32)
    (call $record (local.get $value)))

  ;; Linear memory access and growth.
  (func (export "store") (param $address i32) (param $value i32)
    (i32.store (local.get $address) (local.get $value)))
  (func (export "load") (param $address i32) (result i32)
    (i32.load (local.get $address)))
  (func (export "grow_memory") (param $pages i32) (result i32)
    (memory.grow (local.get $pages)))

  ;; Faults.
  (func (export "trap")
    unreachable)
  (func $recurse_forever (export "recurse_forever")
    (call $recurse_forever))
  (func (export "advance_epoch_then_spin")
    (call $advance_epoch)
    (loop $spin (br $spin)))
  (func (export "call_failing_import")
    (call $fail))
  (func (export "call_panicking_import")
    (call $panic))
  (func (export "call_import_with_wrong_result_type") (result i32)
    (call $wrong_result_type)))
