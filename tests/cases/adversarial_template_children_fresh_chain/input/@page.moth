list = [$children([:<li>[$slot]</li>]):
    <ul>
        [$slot]
    </ul>
]

runtime ~= "middle"
output = [list:
    [:first]
    [$fresh: [:[runtime]-fresh]]
    [:third]
]

io.line([: [output]])
