prefix = "runtime"

runtime_wrapper = [prefix:
    BEFORE [:
        first slot lives here [$slot("first")]
        bridge [$slot]
        second slot lives here [:
            deeper scene [$slot("second")]
        ]
    ] AFTER
]

using_runtime_slots = [runtime_wrapper:
    [$insert("first"): first]
    in between
    [$insert("second"): second]
]

[:template_slots_nested_runtime [using_runtime_slots]]
