import @core/time

-- Use epoch values for stable output
ts0 = time.timestamp_from_unix_milliseconds(0.0)
ts1 = time.timestamp_from_unix_milliseconds(1000.0)
ts2 = time.timestamp_from_unix_seconds(1.0)

iso0 = time.to_iso_string(ts0)
iso1 = time.to_iso_string(ts1)
millis0 = time.unix_milliseconds(ts0)
sec1 = time.unix_seconds(ts1)
millis2 = time.unix_milliseconds(ts2)

io.line([:timestamp_conversions iso0=[iso0] iso1=[iso1] millis0=[millis0] sec1=[sec1] millis2=[millis2]])
