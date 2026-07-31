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

join_rendered type Left is DISPLAY_TEXT, Right is HAS_NAME |left Left, right Right| -> String:
    return left.display() + ":" + right.name()
;

[:generic_bound_receiver_multiple_params_success [join_rendered(Label("left"), Badge("right"))]]
