list = [:
    <ul>
        [$children([:<li>[$slot]</li>]):
            [$slot]
        ]
    </ul>
]

show = true
first = "one"
second = "two"

rendered = [list:
    [if show:
        [first]
    ]
    [if show:
        [second]
    ]
]

io.line(rendered)
