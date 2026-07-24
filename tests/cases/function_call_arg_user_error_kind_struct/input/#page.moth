ErrorKind = |
    message String,
|

log_error |message ErrorKind|:
    io.line([:error-kind-message=[message.message]])
;

log_error(ErrorKind("something went wrong"))
