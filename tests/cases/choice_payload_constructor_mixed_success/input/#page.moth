Response :: Success, Pending | retry_count Int, message String |;

p = Response::Pending(3, message = "waiting")

if p is:
    Success => io.line("choice_payload_constructor_mixed_success variant=Success")
    Pending(retry_count, message) =>
        io.line([:choice_payload_constructor_mixed_success variant=Pending retry_count=[retry_count] message=[message]])
;
