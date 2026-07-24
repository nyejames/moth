Token = |
    raw String,
|

to_string |this Token| -> String:
    return "string:" + this.raw
;

to_error |this Token| -> Error:
    return Error(message = "error:" + this.raw, code = 1)
;

Token must CASTABLE_TO_STRING
Token must CASTABLE_TO_ERROR

token = Token(raw = "x")
text String = cast token
err Error = cast token

[:user multiple targets: [text]|[err.message]]
