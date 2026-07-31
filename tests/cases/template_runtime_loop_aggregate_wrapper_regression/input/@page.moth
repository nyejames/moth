-- Runtime regression for template loop output, aggregate wrapper once,
-- zero-iteration wrapper skip, and break/continue output preservation.

shell #= [:<ol class="loop-shell">shell-start[$slot]shell-end</ol>]

items = {"item-a", "item-b", "item-c", "item-d", "item-e"}
empty {String} = {}

render_loop |source {String}| -> String:
    return [shell, loop source |item, index|:
        [if index is 1:
            [continue]
        ]
        <li>[item]</li>
        [if index is 3:
            [break]
        ]
    ]
;

[:
    template_runtime_loop_aggregate_wrapper_regression
    [render_loop(items)]
    [render_loop(empty)]
]
