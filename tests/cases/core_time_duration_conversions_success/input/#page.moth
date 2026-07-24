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

-- Deterministic values, not live current time
d1 = duration_from_seconds(1.5)
d2 = duration_from_milliseconds(-250.0)
d3 = duration_from_milliseconds(500.0)

sec = as_seconds(d1)
millis = as_milliseconds(d1)
neg = is_negative(d2)
abs_d = abs(d2)
abs_sec = as_seconds(abs_d)
clamped = clamp(d3, duration_from_milliseconds(0.0), duration_from_milliseconds(1000.0))
clamped_sec = as_seconds(clamped)

io.line([:duration_conversions sec=[sec] millis=[millis] neg=[neg] abs_sec=[abs_sec] clamped_sec=[clamped_sec]])
