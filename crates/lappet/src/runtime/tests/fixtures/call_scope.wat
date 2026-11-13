;; Hand-written call-scope helpers. Each step reports itself, then traps when armed.
(module
  (import "host" "step" (func $step (param i32)))
  (memory (export "memory") 1)
  (global $open (mut i32) (i32.const 0))
  (global $bump (mut i32) (i32.const 64))
  (global $trap (mut i32) (i32.const 0))

  (func (export "arm") (param $which i32)
    (global.set $trap (local.get $which)))

  (func (export "moth_abi_v1_scope_open")
    (call $step (i32.const 1))
    (if (i32.eq (global.get $trap) (i32.const 1))
      (then (unreachable)))
    (if (global.get $open)
      (then (unreachable)))
    (global.set $open (i32.const 1)))

  (func (export "moth_abi_v1_scope_reserve") (param $size i32) (param $align i32) (result i32)
    (local $address i32)
    (call $step (i32.const 2))
    (if (i32.eq (global.get $trap) (i32.const 2))
      (then (unreachable)))
    (if (i32.eqz (global.get $open))
      (then (unreachable)))
    (if (i32.eqz (local.get $size))
      (then (unreachable)))
    (if (i32.and
          (i32.ne (local.get $align) (i32.const 4))
          (i32.ne (local.get $align) (i32.const 8)))
      (then (unreachable)))
    (local.set $address
      (i32.and
        (i32.add (global.get $bump) (i32.sub (local.get $align) (i32.const 1)))
        (i32.sub (i32.const 0) (local.get $align))))
    (global.set $bump (i32.add (local.get $address) (local.get $size)))
    (local.get $address))

  (func (export "moth_abi_v1_scope_close")
    (call $step (i32.const 3))
    (if (i32.eqz (global.get $open))
      (then (unreachable)))
    (global.set $open (i32.const 0))
    (global.set $bump (i32.const 64)))

  (func (export "work") (param $argument i32) (param $length i32) (param $frame i32) (result i32)
    (call $step (i32.const 4))
    (if (i32.eq (global.get $trap) (i32.const 4))
      (then (unreachable)))
    (if (local.get $argument)
      (then
        (i32.store (local.get $frame) (local.get $argument))
        (i32.store offset=4 (local.get $frame) (i32.const 2)))
      (else
        (i32.store (local.get $frame) (i32.const 0))
        (i32.store offset=4 (local.get $frame) (i32.const 0))))
    (i32.const 0))

  (func (export "work_bad_status") (param $argument i32) (param $length i32) (param $frame i32) (result i32)
    (call $step (i32.const 4))
    (i32.const 9))

  (func (export "work_bad_span") (param $argument i32) (param $length i32) (param $frame i32) (result i32)
    (call $step (i32.const 4))
    (i32.store (local.get $frame) (i32.const 32))
    (i32.store offset=4 (local.get $frame) (i32.const 0))
    (i32.const 0))

  (func (export "work_bad_utf8") (param $argument i32) (param $length i32) (param $frame i32) (result i32)
    (call $step (i32.const 4))
    (i32.store8 (i32.const 32) (i32.const 255))
    (i32.store (local.get $frame) (i32.const 32))
    (i32.store offset=4 (local.get $frame) (i32.const 1))
    (i32.const 0))

  (func (export "work_declared_error") (param $argument i32) (param $length i32) (param $frame i32) (result i32)
    (call $step (i32.const 4))
    (i32.const 1)))
