import @core/time {
    Duration,
    duration_from_seconds,
    duration_from_milliseconds,
    as_seconds,
    as_milliseconds,
    is_negative,
    abs,
    clamp,
}

-- Duration helpers are ordinary external free functions.
d = duration_from_seconds(2.5)
sec = as_seconds(d)
millis = as_milliseconds(d)
neg = is_negative(d)
abs_d = abs(d)
abs_sec = as_seconds(abs_d)
clamped = clamp(d, duration_from_milliseconds(0.0), duration_from_milliseconds(3000.0))
clamped_sec = as_seconds(clamped)

io.line([:duration_free_functions sec=[sec] millis=[millis] neg=[neg] abs_sec=[abs_sec] clamped_sec=[clamped_sec]])
