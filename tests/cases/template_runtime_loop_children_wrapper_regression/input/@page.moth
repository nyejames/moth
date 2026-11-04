-- Runtime regression: $children() wrapper around a runtime template loop.
-- Verifies the wrapper wraps the loop's emitted aggregate output once
-- and is skipped when the loop emits nothing.

list = [:
    <ul>
        [$children([:<li>[$slot]</li>]):
            [$slot]
        ]
    </ul>
]

items = {"one", "two"}
empty {String} = {}

rendered = [list:
    [loop items |item|:
        [item]
    ]
]

rendered_empty = [list:
    [loop empty |item|:
        [item]
    ]
]

io.line(rendered)
io.line(rendered_empty)
