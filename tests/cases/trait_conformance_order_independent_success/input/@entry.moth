Widget must LOCAL_RENDER

render |this Widget| -> String:
    return this.name
;

LOCAL_RENDER must:
    render |This| -> String
;

Widget = |
    name String,
|

widget = Widget("ordered")
[:trait_conformance_order_independent_success [widget.render()]]
