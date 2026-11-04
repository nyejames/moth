Response :: Err | message String |, Success;

use_choice |r Response| -> String:
    if r is:
        Success => return "ok"
        Response::Err(message) => return "err"
    ;
;

label = use_choice(Response::Success)
[:choice_payload_record_declaration_success label=[label]]
