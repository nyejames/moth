import @data/models { Metric, Status }
import @utils/math { wave }
import @utils/text { prefix }
import @utils/random { random_score }

build_metric |name String, phase Float| -> Metric:
    value = wave(phase)
    return Metric(name, value)
;

format_metric |metric Metric| -> String:
    return [: [prefix(metric.name)] = [metric.value] ]
;

status_for |value Float| -> Status:
    if value < 0.0:
        return Status::Error(1, "negative")
    ;
    if value > 1.0:
        return Status::Warning(2)
    ;
    return Status::Ok
;
