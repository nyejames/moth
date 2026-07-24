list = [$children([:<li class="wrapped">[$slot]</li>]):
    <ul>
        [$slot]
    </ul>
]

show = true
label = "runtime"

rendered = [list:
    [$fresh, if show:
        fresh [label]
    ]
]

io.line(rendered)
