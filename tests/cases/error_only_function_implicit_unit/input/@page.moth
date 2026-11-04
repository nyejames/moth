-- error-only function with empty success path keeps implicit unit-success behavior

fail || -> Error!:
    return! Error("boom")
;

fail() catch |err|:
    io.line([: [err.message]])
;
io.line("error_only_function_implicit_unit")
