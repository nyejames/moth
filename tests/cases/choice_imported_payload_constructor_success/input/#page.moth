import @choices { Response }

e = Response::Err("bad")

if e is:
    Response::Success => io.line("choice_imported_payload_constructor_success variant=Success")
    Response::Err(message) => io.line([:choice_imported_payload_constructor_success variant=Err message=[message]])
;
