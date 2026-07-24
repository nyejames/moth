list = [:
    <ol>
        [$slot]
    </ol>
    <footer>[$slot("summary")]</footer>
]

row = [:<li data-copy="[$slot(1)]">[$slot]</li>]

items = {"one", "two"}

rendered = [list:
    [loop items |item|:
        [row, item: [item]]
    ]

    [$insert("summary"): done]
]

io.line(rendered)
