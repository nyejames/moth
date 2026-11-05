nested = [:
    <section>
        [:
            <article>[$slot("body")]</article>
        ]
        [$slot]
    </section>
]

show = true
label = "runtime"

rendered = [nested:
    [$insert("body"):
        [if show:
            nested [label]
        ]
    ]
]

io.line(rendered)
