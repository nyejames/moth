import @test/optional

accept_optional_string |value String?| -> String:
    if value is |text|:
        return "some " + text
    else
        return "none"
    ;
;

present_text || -> String?:
    return optional.present_text()
;

result = accept_optional_string(present_text())
io.line([:external_optional_type_identity [result]])
