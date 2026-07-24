import @core/time {
    Timestamp,
}
import @core/time

parse_epoch_plus_one || -> Timestamp, Error!:
    return time.timestamp_from_iso_string("1970-01-01T00:00:01.000Z")!
;

-- Parse a valid ISO string with postfix propagation inside a fallible helper,
-- then recover at the top-level start boundary.
ts = parse_epoch_plus_one() catch:
    then time.timestamp_from_unix_milliseconds(0.0)
;

millis = time.unix_milliseconds(ts)
iso = time.to_iso_string(ts)

io.line([:timestamp_parse millis=[millis] iso=[iso]])
