DISPLAY_TEXT must:
    display |This| -> String
;

Plain = |
    text String,
|

Box type T is DISPLAY_TEXT = |
    value T,
|

bad = Box(Plain("no evidence"))

[:unreachable [bad.value.text]]
