import @core/time

-- Catch invalid ISO string and recover to a fallback
ts = time.timestamp_from_iso_string("not-a-date") catch:
    then time.timestamp_from_unix_milliseconds(0.0)
;

millis = time.unix_milliseconds(ts)
io.line([:timestamp_parse_catch millis=[millis]])
