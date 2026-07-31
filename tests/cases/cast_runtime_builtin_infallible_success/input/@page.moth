as_float |source Int| -> Float:
    return cast source
;

as_string_int |source Int| -> String:
    return cast source
;

as_string_float |source Float| -> String:
    return cast source
;

as_string_bool |source Bool| -> String:
    return cast source
;

as_string_char |source Char| -> String:
    return cast source
;

as_int_char |source Char| -> Int:
    return cast source
;

as_error |source String| -> Error:
    return cast source
;

as_string_error |source Error| -> String:
    return cast source
;

ratio_text = as_string_float(as_float(5) + 0.5)
count = as_string_int(2)
pi = as_string_float(3.25)
flag = as_string_bool(false)
letter = as_string_char('Z')
code = as_int_char('Z')
err = as_error("boom")
msg = as_string_error(err)

[: runtime infallible casts: [ratio_text]|[count]|[pi]|[flag]|[letter]|[code]|[err.message]|[msg]]
