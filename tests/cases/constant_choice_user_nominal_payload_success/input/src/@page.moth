-- Constant choice constructor with a payload field whose type is a user nominal.
-- Regression: before the frontend type boundary cleanup, unresolved parsed type
-- references in choice variant shells could silently fall back to builtin `none`.

Payload = |
    text String,
|

Status ::
    Empty,
    Loaded | payload Payload |,
;

status #= Status::Loaded(Payload("ok"))

io.line("ok")
