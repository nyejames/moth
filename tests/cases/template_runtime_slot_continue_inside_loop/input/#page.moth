row #= [:<li>[$slot]</li>]
shell #= [:<ul class="slot-continue-shell">[$slot]</ul>]
items = {1, 2}

[:
    template_runtime_slot_continue_inside_loop
    direct continue before output
    [shell:
        [loop items |item|:
            [row:
[continue]
direct hidden [item]
            ]
        ]
    ]
    nested loop continues before output
    [shell:
        [loop items |item|:
            [row:
                [loop 0 to item |inner|:
                    [continue]
                    nested hidden [inner]
                ]
            ]
        ]
    ]
    after
]
