-- Template heads reject unhandled error-returning operands before string-content typing.

can_error || -> Int, Error!:
    return! Error("not a number")
;

[: [can_error()]]
