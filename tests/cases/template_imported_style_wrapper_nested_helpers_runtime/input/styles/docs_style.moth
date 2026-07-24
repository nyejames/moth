format_table #= [$html:
    <table style="[$slot("style")]">
        [$children([:<tr>[$slot]</tr>]):[$slot]]
    </table>
]

table #= [format_table, $html:
    [$insert("style"): border-collapse: collapse; border: 1px solid;]
    [$children([:<td>[$slot]</td>]):[$slot]]
]
