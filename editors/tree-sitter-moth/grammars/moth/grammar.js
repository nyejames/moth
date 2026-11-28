const { mothRules } = require("../../common/moth");

module.exports = grammar({
    name: "moth",
    extras: $ => [/\s+/, $.comment],
    word: $ => $.identifier,
    rules: {
        source_file: $ => repeat(choice($.binding, $.construction, $.documentation)),
        binding: $ => seq(
            field("name", $.identifier),
            optional(field("type", $.type_annotation)),
            field("operator", choice("=", "#=", "~=")),
            field("value", $._value),
        ),
        type_annotation: $ => seq(optional(choice("#", "~")), $.type_name),
        type_name: $ => choice($.builtin_type, $.identifier),
        builtin_type: _ => choice(
            "Bool", "String", "Char", "Byte", "None", "True", "False", "Error",
            "I8", "I16", "I32", "I64", "U8", "U16", "U32", "U64",
            "F16", "F32", "F64",
            /Dec(0|[1-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-6])?/,
        ),
        documentation: $ => seq("$doc", $._value),
        ...mothRules,
    },
});
