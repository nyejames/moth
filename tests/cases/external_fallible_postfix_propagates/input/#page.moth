import @test/pkg-a { fallible_text_ok }

wrap_external || -> String, Error!:
    value = fallible_text_ok("ok")!
    return value
;

value = wrap_external() catch:
    then "fallback"
;

io.line([:external_fallible_postfix_propagates value=[value]])
