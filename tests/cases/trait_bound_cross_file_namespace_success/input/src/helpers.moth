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

render type Item is DISPLAY_TEXT |item Item| -> String:
    return item.display()
;
