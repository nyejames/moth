single_slot #= [: before [$slot] after]

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

[:template_docs_examples_runtime_coverage [single_slot] [using_runtime_slots]]
