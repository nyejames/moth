ErrA = |
    message String,
|

ErrB = |
    message String,
|

inner || -> Int, ErrA!:
    return! ErrA("failed")
;

outer || -> Int, ErrB!:
    value = inner()!
    return value
;

outer()
