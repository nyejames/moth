maybe_label String? = "outer"

[:
    template_if_nested_else_binding
    [if maybe_label is |label|:
        [if false:
            wrong nested then
        [else]
            nested else sees [label]
        ]
    [else]
        wrong outer else
    ]
]
