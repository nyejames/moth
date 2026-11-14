;; Authored workloads for the runtime policy measurements. Plain core Wasm, not compiled Moth,
;; so the numbers they produce say nothing about the Moth host ABI or Moth code generation.
(module
  (import "host" "read_input" (func $read_input (param i32) (result i32)))
  (import "host" "record_depth" (func $record_depth (param i32)))

  (memory (export "memory") 1)

  ;; Game-like step: one short call per entity per frame. It makes three scalar host calls
  ;; and updates a 16-byte entity record in linear memory.
  (func (export "step_entity") (param $entity i32) (result i32)
    (local $record i32)
    (local $position i32)
    (local.set $record (i32.shl (i32.and (local.get $entity) (i32.const 1023)) (i32.const 4)))
    (local.set $position
      (i32.add
        (i32.load (local.get $record))
        (i32.add
          (call $read_input (i32.const 0))
          (i32.mul (call $read_input (i32.const 1)) (call $read_input (i32.const 2))))))
    (i32.store (local.get $record) (local.get $position))
    (i32.store offset=4 (local.get $record)
      (i32.add (i32.load offset=4 (local.get $record)) (i32.const 1)))
    (local.get $position))

  ;; Server-like request: grow memory, fill the new pages, then checksum them.
  (func (export "handle_request") (param $pages i32) (result i32)
    (local $address i32)
    (local $end i32)
    (local $checksum i32)
    (local.set $address (i32.shl (memory.grow (local.get $pages)) (i32.const 16)))
    (local.set $end (i32.add (local.get $address) (i32.shl (local.get $pages) (i32.const 16))))
    (block $done
      (loop $fill
        (br_if $done (i32.ge_u (local.get $address) (local.get $end)))
        (i32.store (local.get $address) (i32.mul (local.get $address) (i32.const 2654435761)))
        (local.set $checksum (i32.xor (local.get $checksum) (i32.load (local.get $address))))
        (local.set $address (i32.add (local.get $address) (i32.const 4)))
        (br $fill)))
    (local.get $checksum))

  ;; Compute-bound loop with no host call or memory access: Collatz steps summed over a range.
  (func (export "sum_collatz_steps") (param $limit i64) (result i64)
    (local $start i64)
    (local $value i64)
    (local $steps i64)
    (local.set $start (i64.const 1))
    (block $done
      (loop $next_start
        (br_if $done (i64.gt_u (local.get $start) (local.get $limit)))
        (local.set $value (local.get $start))
        (block $reached_one
          (loop $step
            (br_if $reached_one (i64.le_u (local.get $value) (i64.const 1)))
            (if (i64.eqz (i64.and (local.get $value) (i64.const 1)))
              (then (local.set $value (i64.shr_u (local.get $value) (i64.const 1))))
              (else (local.set $value
                (i64.add (i64.mul (local.get $value) (i64.const 3)) (i64.const 1)))))
            (local.set $steps (i64.add (local.get $steps) (i64.const 1)))
            (br $step)))
        (local.set $start (i64.add (local.get $start) (i64.const 1)))
        (br $next_start)))
    (local.get $steps))

  ;; Host-call cost in isolation. The store keeps both loops observable, so the
  ;; difference is the host call rather than a deleted loop.
  (func (export "loop_with_host_call") (param $count i32) (result i32)
    (local $index i32)
    (local $sum i32)
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $index) (local.get $count)))
        (local.set $sum (i32.add (local.get $sum) (call $read_input (local.get $index))))
        (i32.store (i32.shl (i32.and (local.get $index) (i32.const 1023)) (i32.const 2)) (local.get $sum))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $next)))
    (local.get $sum))
  (func (export "loop_without_host_call") (param $count i32) (result i32)
    (local $index i32)
    (local $sum i32)
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $index) (local.get $count)))
        (local.set $sum (i32.add (local.get $sum) (local.get $index)))
        (i32.store (i32.shl (i32.and (local.get $index) (i32.const 1023)) (i32.const 2)) (local.get $sum))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $next)))
    (local.get $sum))

  ;; Recursion depth probe: reports each depth to the host until the Wasm stack is exhausted.
  (func $descend (export "descend") (param $depth i32)
    (call $record_depth (local.get $depth))
    (call $descend (i32.add (local.get $depth) (i32.const 1)))))
