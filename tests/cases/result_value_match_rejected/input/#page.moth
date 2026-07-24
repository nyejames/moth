parse_number |text String| -> Int, Error!:
    return! Error(text)
;

label = if parse_number("bad") is:
    0 => then "zero"
    else => then "other"
;

io.line([: [label]])
