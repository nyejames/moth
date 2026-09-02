parse_number |text String| -> Int, Error!:
    return! Error(text)
;

recover || -> Int:
    value = parse_number("bad") catch:
        then 0
    ;
    return value
;

value = recover()
io.line("value catch block recovery")
io.line([: [value]])
