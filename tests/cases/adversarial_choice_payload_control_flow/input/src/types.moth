TaskResult ::
    Success |
        value Int,
        label String,
    |,
    Failure |
        reason String,
        code Int,
    |,
    Pending,
;

format_task |t TaskResult| -> String:
    if t is:
        Success(value, label) => return "ok"
        Failure(reason, code) => return "err"
        Pending => return "pending"
    ;
;
