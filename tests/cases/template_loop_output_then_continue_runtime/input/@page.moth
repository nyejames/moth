items = {1, 2}
list_shell #= [:<ul class="output-then-continue">[$slot]</ul>]

[:
    template_loop_output_then_continue_runtime
    [list_shell, loop items |item|:
        <li>before [item]</li>
        [continue]
        <li>after [item]</li>
    ]
]
