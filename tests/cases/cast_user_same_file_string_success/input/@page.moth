Label = |
    text String,
|

to_string |this Label| -> String:
    return this.text
;

Label must CASTABLE_TO_STRING

label = Label(text = "hello")
message String = cast label

[:user same-file string: [message]]
