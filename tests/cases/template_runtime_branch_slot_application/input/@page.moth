card = [:
    <section>
        <h1>[$slot("title")]</h1>
        <div>[$slot]</div>
        <aside>[$slot("aside")]</aside>
    </section>
]

show_title = false
maybe_title String? = "Priya"
body = "runtime body"

rendered = [card:
    [$insert("title"):
        [if show_title:
            Hidden title
        [else if maybe_title is |title|]
            Hello [title]
        [else]
            Missing title
        ]
    ]

    [if show_title:
        Hidden body
    [else]
        Body [body]
    ]
]

io.line(rendered)
