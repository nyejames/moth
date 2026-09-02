Label = |
    text String,
|

to_string |this Label| -> String:
    return this.text
;

Label must CASTABLE_TO_STRING

show type T is CASTABLE_TO_STRING |value T| -> String:
    return cast value
;

message = show(Label(text = "from generic"))

[:generic user evidence: [message]]
