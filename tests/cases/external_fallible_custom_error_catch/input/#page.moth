import @test/pkg-a { fallible_custom_error_ok }

value = fallible_custom_error_ok() catch:
    then "fallback"
;

io.line([:external_fallible_custom_error_catch value=[value]])
