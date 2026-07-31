render_count |count $Int| -> String:
    return [:count=[count]]
;

snapshot_count |value Int| -> String:
    return [:snapshot=[value]]
;

count $Int = 3

[:reactive_parameter_success [render_count(count)] [snapshot_count(count)]]
