load_value |ok Bool| -> String, Error!:
    if ok:
        return "value"
    ;

    return! Error("failed")
;

value = load_value(true) catch:

    then "fallback"

;
alias1 = value
alias2 = alias1
io.line([: adversarial_borrow_after_result_handler recovered-alias=[alias2]])
