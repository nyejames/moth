const { lexicalRules } = require("../../common/lexical");
const { commaList, recordRules } = require("../../common/records");

module.exports = grammar({
    name: "mon",
    extras: $ => [/\s+/, $.comment],
    word: $ => $.identifier,
    rules: {
        document: $ => optional(choice($.record, $.record_fields)),
        ...lexicalRules(true),
        ...recordRules,
        _value: $ => choice($._literal, $.record, $.sequence, $.map),
        sequence: $ => seq("[", optional(commaList($._value)), "]"),
        map: $ => seq("[", choice("=", $.map_entries), "]"),
    },
});
