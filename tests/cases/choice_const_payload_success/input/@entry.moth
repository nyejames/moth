Response :: Success, Err | message String |;

err #= Response::Err("bad")

if err is:
    Success => io.line("choice_const_payload_success variant=Success")
    Err(message) => io.line([:choice_const_payload_success variant=Err message=[message]])
;
