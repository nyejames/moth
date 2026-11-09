Result ::
    Ok,
    Err |
        message String,
    |,
;

r = Result::Err("bad")

if r is:
    Ok => io.line("ok")
    Err(message as error_message) => io.line([: [error_message]])
    else => io.line("else")
;

[:pattern_choice_payload_existing_capture_alias_success]
