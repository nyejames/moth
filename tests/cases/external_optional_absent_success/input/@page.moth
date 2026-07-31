import @test/optional

absent_text || -> String?:
    return optional.absent_text()
;

result = absent_text()
if result is |text|:
    io.line("unexpected some")
else
    io.line("external_optional_absent_success none")
;
