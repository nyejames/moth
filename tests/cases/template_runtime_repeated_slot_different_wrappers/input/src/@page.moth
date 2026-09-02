panel = [:
    <section>
        <div class="plain">[$slot("item")]</div>
        [$children([:<strong>[$slot]</strong>]):
            <div class="wrapped">[$slot("item")]</div>
        ]
        [$slot]
    </section>
]

show = true
label = "runtime"

rendered = [panel:
    [$insert("item"):
        [if show:
            value [label]
        ]
    ]
]

io.line(rendered)
