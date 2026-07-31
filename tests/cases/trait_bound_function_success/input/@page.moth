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

identity type T is DISPLAY_TEXT |value T| -> T:
    return value
;

label = identity(Label("ok"))
[:trait_bound_function_success [label.display()]]
