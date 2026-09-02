repeater = [:
    <section>
        <p class="first">[$slot("item")]</p>
        <p class="second">[$slot("item")]</p>
        [$slot]
    </section>
]

mark_once |values ~{Int}| -> String:
    ~values.push(1) catch:
    ;
    count = values.length()
    return [:count=[count]]
;

show = true
values ~{Int} = {}

rendered = [repeater:
    [$insert("item"):
        [if show:
            [mark_once(~values)]
        ]
    ]
]

io.line(rendered)
io.line([:final-count=[values.length()]])
