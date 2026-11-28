// The same named-entry notation serves the two grammars. Each root owns _value.
function commaList(rule) {
    return seq(rule, repeat(seq(",", rule)), optional(","));
}

const recordRules = {
    record: $ => seq("(", optional($.record_fields), ")"),
    record_fields: $ => commaList($.record_field),
    record_field: $ => seq(
        field("name", $.identifier),
        $.field_separator,
        field("value", $._value),
    ),
    // Immediate token prevents comments or line breaks between the label and =.
    // The trailing whitespace may include a line break.
    field_separator: _ => token.immediate(/[^\S\r\n]+=\s+/),
    map_entries: $ => commaList($.map_entry),
    map_entry: $ => seq(field("key", $._value), $.map_separator, field("value", $._value)),
    // First slice accepts whitespace on both sides. Comment trivia before = is a
    // separate map-specific boundary to implement and test at activation.
    map_separator: _ => token.immediate(/\s+=\s+/),
};

module.exports = { commaList, recordRules };
