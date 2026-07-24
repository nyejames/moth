import @./internal {read_or_raise}

export:
    exported_read_or_raise type T, E |value T, err E, fail Bool| -> T, E!:
        result = read_or_raise(value, err, fail)!
        return result
    ;
;
