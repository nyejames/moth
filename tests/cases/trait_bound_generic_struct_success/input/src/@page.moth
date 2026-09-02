DISPLAY_TEXT must:
    display |This| -> String
;

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

Label must DISPLAY_TEXT

Box type T is DISPLAY_TEXT = |
    value T,
|

LabelBox as Box of Label

wrapped LabelBox = Box(Label("ok"))
direct Box of Label = Box(Label("direct"))

[:trait_bound_generic_struct_success [wrapped.value.display()] [direct.value.display()]]
