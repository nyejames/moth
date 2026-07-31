import @test/optional

present_text || -> String?:
    return optional.present_text()
;

result = present_text()
if result is |text|:
    io.line([:external_optional_present_success [text]])
;
