panel #= [:
    <div class="panel">
        <h2>[$slot("heading")]</h2>
        <p>[$slot]</p>
    </div>
]

items = {"alpha", "skip", "beta"}

render |use_fallback Bool, source {String}| -> String:
    return [panel:
        [$insert("heading"):
            [if use_fallback:
                fallback heading
            [else]
                primary heading
            ]
        ]

        [loop source |item|:
            [if item is "skip":
                [continue]
            ]
            [:item [item]]
        ]
    ]
;

[:template_runtime_slot_in_branch_and_loop
   [render(false, items)]
   [render(true, items)]
]
