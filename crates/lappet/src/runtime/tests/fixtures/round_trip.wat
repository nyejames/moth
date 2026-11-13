;; The embedding program's guest: one host import and two exports. Plain core Wasm, not the
;; Moth host ABI.
(module
  (import "host" "read_value" (func $read_value (result i32)))

  (func (export "run") (result i32)
    (call $read_value))

  (func (export "divide_host_value_by") (param $divisor i32) (result i32)
    (i32.div_s (call $read_value) (local.get $divisor))))
