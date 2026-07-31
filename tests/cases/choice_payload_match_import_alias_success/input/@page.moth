import @types { Response as Reply }

use_choice |r Reply| -> String:
    if r is:
        Reply::Err(message) if message is "bad" => return "err"
        Success => return "ok"
        else => return "else"
    ;
;

label = use_choice(Reply::Err("bad"))
[:choice_payload_match_import_alias_success label=[label]]
