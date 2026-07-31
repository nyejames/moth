import @core/time {
    Duration,
    TimeMark,
    Timestamp,
    mark_now,
    elapsed_since,
    duration_between,
    timestamp_now,
    duration_from_seconds,
    duration_from_milliseconds,
    as_seconds,
    as_milliseconds,
    is_negative,
    abs,
    clamp,
    timestamp_from_unix_seconds,
    timestamp_from_unix_milliseconds,
    unix_seconds,
    unix_milliseconds,
    to_iso_string,
}

-- Duration construction and conversions
d1 = duration_from_seconds(1.5)
d2 = duration_from_milliseconds(-250.0)
sec = as_seconds(d1)
millis = as_milliseconds(d1)
neg = is_negative(d2)
abs_d = abs(d2)
abs_sec = as_seconds(abs_d)
clamped = clamp(d1, duration_from_milliseconds(0.0), duration_from_milliseconds(1000.0))
clamped_sec = as_seconds(clamped)

-- TimeMark delta (use deterministic derived values, not live elapsed)
mark1 = mark_now()
mark2 = mark_now()
delta = duration_between(mark1, mark2)
elapsed = elapsed_since(mark1)
delta_ok = as_milliseconds(delta) >= 0.0
elapsed_ok = as_milliseconds(elapsed) >= 0.0

-- Timestamp construction and conversions
ts_now = timestamp_now()
ts0 = timestamp_from_unix_milliseconds(0.0)
ts1 = timestamp_from_unix_seconds(1.0)
ts_millis = unix_milliseconds(ts0)
ts_sec = unix_seconds(ts1)
ts_iso = to_iso_string(ts0)

io.line([:duration sec=[sec] millis=[millis] neg=[neg] abs_sec=[abs_sec] clamped_sec=[clamped_sec]])
io.line([:delta_ok=[delta_ok] elapsed_ok=[elapsed_ok]])
io.line([:timestamp ts_millis=[ts_millis] ts_sec=[ts_sec] iso=[ts_iso]])
