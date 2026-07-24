DISPLAY_TEXT must:
    display |This| -> String
;

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

render type T is DISPLAY_TEXT |value T| -> String:
    return value.display()
;

label = render(Label("looks structural"))
io.line([: [label]])
