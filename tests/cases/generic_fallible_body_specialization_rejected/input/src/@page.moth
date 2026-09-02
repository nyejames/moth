parse_number |text String| -> Int, Error!:
    return! Error([: bad [text]])
;

outer type E |text String| -> Int, E!:
    value = parse_number(text)!
    return value
;

value = outer("x") catch:

    then 0