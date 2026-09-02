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

Maybe type T is DISPLAY_TEXT ::
    Some | value T |,
    Missing,
;

label type T is DISPLAY_TEXT |this Maybe of T| -> String:
    return "maybe"
;

value Maybe of Label = Maybe::Some(Label("ok"))

[:trait_bound_generic_choice_success [value.label()]]
