DISPLAY_TEXT must:
    display |This| -> String
;

Plain = |
    text String,
|

display |this Plain| -> String:
    return this.text
;

Box type T is DISPLAY_TEXT = |
    value T,
|

bad Box of Plain = Box(Plain("method only"))

[:unreachable [bad.value.display()]]
