last_number || -> Int:
    return 3
;

[:
    runtime loop unaffected before
    [loop 1 to & last_number() |number|:
        runtime-[number];
    ]
    runtime loop unaffected after
]
