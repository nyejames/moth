list #= [$children([:<li class="branch-item">[$slot]</li>]):
    <ul class="branch-list">[$slot]</ul>
]

render |pick_first Bool| -> String:
    return [list:
        ["branch-head", if pick_first:
            first branch
        [else]
            fallback branch
        ]
    ]
;

[:template_runtime_branch_fallback_wrapped_output
   [render(true)]
   [render(false)]
]
