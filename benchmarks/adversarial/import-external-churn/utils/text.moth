import @core/text { length, contains, starts_with, ends_with }
import @data/constants { project_name }

prefix |input String| -> String:
    return [: [project_name] / [input] ]
;

has_marker |input String| -> Bool:
    return contains(input, "churn")
;

is_release |input String| -> Bool:
    return ends_with(input, "release")
;

label_length |input String| -> Int:
    return length(input)
;
