const_prefix = [:const]

slot_wrapper = [const_prefix:
    pre [$slot("a")] mid [$slot] end [$slot("b")]
]

runtime_value = "rv"
rendered = [slot_wrapper:
    [$insert("a"): [:A-[runtime_value]]]
    middle
    [$insert("b"): [:B-[runtime_value]]]
]

io.line([: [rendered]])
