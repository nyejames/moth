DISPLAY_TEXT must:
    display |This| -> String
;

ALT_DISPLAY must:
    display |This| -> String
;

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

Label must DISPLAY_TEXT
Label must ALT_DISPLAY

join_rendered type Left is DISPLAY_TEXT, Right is ALT_DISPLAY |left Left, right Right| -> String:
    return left.display() + ":" + right.display()
;

[:generic_bound_receiver_same_concrete_params_success [join_rendered(Label("same"), Label("same"))]]
