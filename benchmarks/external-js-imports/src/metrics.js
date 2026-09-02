import { mothOk, mothErr } from "@moth/runtime";

/**
 * @moth.opaque MetricHandle
 */

/**
 * @moth.sig create_metric |name String, value Int| -> MetricHandle
 */
export function createMetric(name, value) {
    return { name, value };
}

/**
 * @moth.sig metric_label |metric MetricHandle| -> String
 */
export function metricLabel(metric) {
    return `${metric.name}:${metric.value}`;
}

/**
 * @moth.sig set_metric_value |metric ~MetricHandle, value Int|
 */
export function setMetricValue(metric, value) {
    metric.value = value;
}

/**
 * @moth.sig load_metric_label |id String| -> String, Error!
 */
export function loadMetricLabel(id) {
    if (id === "") {
        return mothErr(404, "Missing metric id");
    }

    return mothOk(`metric:${id}`);
}
