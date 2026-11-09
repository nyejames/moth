Status :: Ready, Busy;

current Status = Status::Ready

handle |status Status|:
    if status is:
        Ready |payload| => io.line("ready")
        else => io.line("other")
    ;
;

handle(current)
