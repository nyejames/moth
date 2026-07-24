Status ::
    Ready,
    Loading |
        progress Int,
    |,
;

status = Status::Loading(42)

if status is:
    Loading(progress as p) if p is 42 => io.line("guarded-renamed-progress=42")
    Ready => io.line("ready")
    else => io.line("other")
;
