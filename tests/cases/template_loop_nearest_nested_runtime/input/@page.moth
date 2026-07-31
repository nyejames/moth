outer_items = {1, 2}
inner_items = {1, 2}

[:
    template_loop_nearest_nested_runtime
    [loop outer_items |outer|:
        outer [outer] start;
        [loop inner_items |inner|:
            [if inner is 1:
                [continue]
            ]
            inner [outer]:[inner] after;
        ]
        outer [outer] done;
    ]
]
