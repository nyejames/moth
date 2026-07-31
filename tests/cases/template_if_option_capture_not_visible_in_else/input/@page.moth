maybe_name String? = none
fallback_name = "guest"

[:
    template_if_option_capture_not_visible_in_else
    [if maybe_name is |present_name|:
        hello [present_name]
    [else]
        hello [fallback_name]
    ]
]
