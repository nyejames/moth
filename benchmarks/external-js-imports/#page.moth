import @./metrics.js {
    create_metric,
    load_metric_label,
    metric_label,
    set_metric_value,
    MetricHandle,
}
import @./formatters.js as fmt
import @core/math { PI }

metric ~MetricHandle = create_metric("alpha", 7)
set_metric_value(~metric, 21)

direct_label = metric_label(metric)
fallback_label = load_metric_label("") catch:
    then "metric:missing"
;

summary = fmt.join_label("external-js", direct_label)
emphasized = fmt.emphasize(fallback_label)
angle = PI / 2.0

[:
    <section>
        <h1>[summary]</h1>
        <p>[emphasized]</p>
        <p>angle=[angle]</p>
    </section>
]
