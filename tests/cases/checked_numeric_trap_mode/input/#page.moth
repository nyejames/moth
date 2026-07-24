maybe_int_overflow |do_it Bool|:
    if do_it:
        x = 2147483647
        y = x + 1
    ;
;

maybe_float_nonfinite |do_it Bool|:
    if do_it:
        x = 1e200
        y = x * 1e200
    ;
;

custom_channel_overflow |do_it Bool| -> Int, String!:
    if do_it:
        x = 2147483647
        return x + 1
    ;

    return 0
;

maybe_int_overflow(false)
maybe_float_nonfinite(false)
ignored = custom_channel_overflow(false) catch:
    then 0
;

io.line([: trap helpers emitted])
