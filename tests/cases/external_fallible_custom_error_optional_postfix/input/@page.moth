import @test/pkg-a { PkgError }
import @test/pkg-a { fallible_custom_error_ok }

wrap || -> String, PkgError?!:
    value = fallible_custom_error_ok()!
    return value
;

value = wrap() catch |err|:
    if err is not none:
        io.line("optional_custom_error_present")
    ;
    then "fallback"
;

io.line([: [value]])
