parse_number |text String| -> Int, Error!:
    return! Error(text)
;

load_name || -> String, Error!:
    return! Error("missing")
;

load_user || -> String, Float, Error!:
    return! Error("missing")
;

fallback_for |err Error| -> String:
    return "fallback"
;

recover_number || -> Int:
    value = parse_number("bad") catch then 0
    return value
;

recover_name || -> String:
    name = load_name() catch |err| then fallback_for(err)
    return name
;

recover_user || -> String, Float:
    name, score = load_user() catch then "guest", 0.0
    return name, score
;

value = recover_number()
name = recover_name()
user_name, score = recover_user()
io.line("value catch inline recovery")
io.line([: [value]])
io.line([: [name]])
io.line([: [user_name]])
io.line([: [score]])
