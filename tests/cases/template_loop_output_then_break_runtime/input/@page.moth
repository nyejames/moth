items = {1, 2}
list_shell #= [:<ul class="output-then-break">[$slot]</ul>]

[:
    template_loop_output_then_break_runtime
    [list_shell, loop items |item|:
        <li>before [item]</li>
        [break]
        <li>after [item]</li>
    ]
]
