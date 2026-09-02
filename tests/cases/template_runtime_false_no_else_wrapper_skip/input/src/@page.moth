list #= [$children([:<li class="skipped-item">[$slot]</li>]):
    <ul class="skipped-list">[$slot]</ul>
]

render || -> String:
    return [list:
        ["skip-head", if false:
            hidden branch
        ]
    ]
;

[:template_runtime_false_no_else_wrapper_skip
   before
   [render()]
   after
]
