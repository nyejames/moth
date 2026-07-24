import @test/pkg-a { fallible_text_ok }

wrap_external || -> String, String!:
    value = fallible_text_ok("bad")!
    return value
;

value = wrap_external() catch:
    then "fallback"
;

io.line([: [value]])
