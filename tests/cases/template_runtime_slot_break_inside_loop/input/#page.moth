row #= [:<li>[$slot]</li>]
items = {1, 2, 3}

[:
    template_runtime_slot_break_inside_loop
    [loop items |item|:
        [if item is 1:
            [row:
                before [item];
                after [item];
            ]
        ]
        [if item is 2:
            [row:
                before [item];
                [break]
            ]
        ]
    ]
]
