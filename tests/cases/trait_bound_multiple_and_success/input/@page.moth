DISPLAY_TEXT must:
    display |This| -> String
;

HAS_NAME must:
    name |This| -> String
;

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

name |this Label| -> String:
    return this.text
;

Label must DISPLAY_TEXT, HAS_NAME

identity type T is DISPLAY_TEXT and HAS_NAME |value T| -> T:
    return value
;

label = identity(Label("both"))
[:trait_bound_multiple_and_success [label.display()] [label.name()]]
