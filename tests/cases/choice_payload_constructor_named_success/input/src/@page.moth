Response :: Err | message String |, Pending | retry_count Int, message String |, Success;

e = Response::Err(message = "bad")
p = Response::Pending(retry_count = 3, message = "waiting")

if e is:
    Err(message as err_message) =>
        if p is:
            Pending(retry_count as p_retry_count, message as pending_message) =>
                io.line([:choice_payload_constructor_named_success err_variant=Err err_message=[err_message] pending_variant=Pending pending_retry_count=[p_retry_count] pending_message=[pending_message]])
            else =>
        ;
    else =>
;
