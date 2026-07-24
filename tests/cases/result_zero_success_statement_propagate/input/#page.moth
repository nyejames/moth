fail || -> Error!:
    return! Error("zero statement boom")
;

outer || -> Error!:
    fail()!
;

outer() catch |err|:
    io.line([: [err.message]])
;
