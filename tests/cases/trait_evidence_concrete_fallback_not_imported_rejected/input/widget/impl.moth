import @./traits { DISPLAY_TEXT }

Label = |
    text String,
|

make_label_impl |text String| -> Label:
    return Label(text)
;

display |this Label| -> String:
    return this.text
;

Label must DISPLAY_TEXT
