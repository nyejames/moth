DISPLAY_TEXT must:
    display |This| -> String
;

HAS_NAME must:
    name |This| -> String
;

Label = |
    text String,
|

Badge = |
    text String,
|

display |this Label| -> String:
    return this.text
;

name |this Badge| -> String:
    return this.text
;

Label must DISPLAY_TEXT
Badge must HAS_NAME

choose_left type Left is DISPLAY_TEXT, Right is HAS_NAME |left Left, right Right| -> Left:
    return left
;

label = choose_left(Label("left"), Badge("right"))
[:trait_bound_multiple_params_success [label.display()]]
