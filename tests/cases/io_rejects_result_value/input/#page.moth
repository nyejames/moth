-- io.line([: []]) rejects unhandled error-returning operands before string-content typing.

can_error || -> Int, Error!:
    return! Error("not a number")
;

io.line([: [can_error()]])
