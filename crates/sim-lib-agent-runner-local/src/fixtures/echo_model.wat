(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 8192))
  (data (i32.const 0) "__CARD_FRAME__")
  (data (i32.const 2048) "__RESPONSE_FRAME__")
  (data (i32.const 4096) "__REQUEST_FRAME__")

  (func (export "sim_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $ptr))

  (func (export "sim_model_card") (result i64)
    (i64.const __CARD_REF__))

  (func $request_matches (param $ptr i32) (param $len i32) (result i32)
    (local $index i32)
    (if (i32.ne (local.get $len) (i32.const __REQUEST_LEN__))
      (then
        (return (i32.const 0))))
    (loop $scan
      (if (i32.eq (local.get $index) (local.get $len))
        (then
          (return (i32.const 1))))
      (if
        (i32.ne
          (i32.load8_u (i32.add (local.get $ptr) (local.get $index)))
          (i32.load8_u (i32.add (i32.const 4096) (local.get $index))))
        (then
          (return (i32.const 0))))
      (local.set $index (i32.add (local.get $index) (i32.const 1)))
      (br $scan))
    (i32.const 1))

  (func (export "sim_model_infer") (param $ptr i32) (param $len i32) (result i64)
    (if (result i64) (call $request_matches (local.get $ptr) (local.get $len))
      (then
        (i64.const __RESPONSE_REF__))
      (else
        (i64.const 0)))))
