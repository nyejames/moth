DISPLAY_TEXT must:
    display |This| -> String
;

Plain = |
    text String,
|

Box type T is DISPLAY_TEXT = |
    value T,
|

PlainBox as Box of Plain

value PlainBox = Box(Plain("no evidence"))

[:unreachable [value.value.text]]
