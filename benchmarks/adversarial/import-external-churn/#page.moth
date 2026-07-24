import @data/constants { project_name, version, base_count }
import @data/models { Metric, Status }
import @utils/math { scaled_count, pi_value, hypotenuse }
import @utils/text { prefix, has_marker, label_length }
import @utils/random { random_index, random_score }
import @utils/metrics { build_metric, format_metric, status_for }
import @core/math { sqrt }
import @core/text { length }
import @core/random { random_int }
import @./external-api.js {
    create_handle,
    call_handle,
    handle_name,
    handle_label,
    ApiHandle,
}

page_title #= [: [project_name] v[version]]

items ~{String} = {}
loop 0 to base_count |i|:
    ~items.push([: item-[i]]) catch:
    ;
;

metrics ~{Metric} = {}
loop 0 to scaled_count() |i|:
    phase = (i / 1.0) * pi_value()
    ~metrics.push(build_metric([: metric-[i]], phase)) catch:
    ;
;

handle ~ApiHandle = create_handle("main")
call_count = call_handle(~handle)
api_label = handle_name(handle)
label_text = handle_label(handle, "api")
random_index_value = random_index()
random_score_value = random_score()
random_core_value = random_int(0, 10)
hypotenuse_value = hypotenuse(3.0, 4.0)
sqrt_value = sqrt(25.0)
label_len = label_length(project_name)
name_len = length(project_name)

render_items |items {String}| -> String:
    output ~= [: <ul>]
    loop items |item|:
        if has_marker(item):
            output = [: [output]<li class="marked">[item]</li>]
        ;
        if not has_marker(item):
            output = [: [output]<li>[item]</li>]
        ;
    ;
    output = [: [output]</ul>]
    return output
;

render_metrics |metrics {Metric}| -> String:
    output ~= [: <div class="metrics">]
    loop metrics |metric|:
        status = status_for(metric.value)
        status_text = format_status(status)
        formatted = format_metric(metric)
        output = [: [output]<p>[formatted] [status_text]</p>]
    ;
    output = [: [output]</div>]
    return output
;

format_status |status Status| -> String:
    if status is:
        Ok => return "ok"
        Warning(code) => return [: warn-[code] ]
        Error(code, message) => return [: err-[code]-[message] ]
    ;
    return "unknown"
;

[:
    <!doctype html>
    <html>
    <head><title>[page_title]</title></head>
    <body>
        <h1>[page_title]</h1>
        <p>Handle: [api_label] [call_count]</p>
        <p>Label: [label_text]</p>
        <p>Random: [random_index_value] / [random_score_value] / [random_core_value]</p>
        <p>Hypotenuse: [hypotenuse_value] / [sqrt_value]</p>
        <p>Label length: [label_len] / [name_len]</p>
        [render_items(items)]
        [render_metrics(metrics)]
    </body>
    </html>
]
