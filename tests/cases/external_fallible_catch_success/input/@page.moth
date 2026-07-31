import @test/pkg-a { fallible_text_ok }

value = fallible_text_ok("kept") catch:
    then "fallback"
;

io.line([:external_fallible_catch_success value=[value]])
