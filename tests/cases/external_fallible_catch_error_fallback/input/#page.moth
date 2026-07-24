import @test/pkg-a { fallible_text_err }

value = fallible_text_err() catch |err|:
    io.line([: [err.message]])
    then "fallback"
;

io.line([:external_fallible_catch_error_fallback value=[value]])
