DISPLAY_TEXT must:
    display |This| -> String
;

display |this String| -> String:
    return this
;

String must DISPLAY_TEXT

Box type T is DISPLAY_TEXT = |
    value T,
|

bad Box of String = Box("local only")

[:unreachable [bad.value]]
