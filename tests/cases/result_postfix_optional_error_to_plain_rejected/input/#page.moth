SmallError = |
    message String,
|

inner || -> Int, SmallError?!:
    return! SmallError("optional error")
;

outer || -> Int, SmallError!:
    value = inner()!
    return value
;

outer()
