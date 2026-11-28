const { mothRules } = require("../../common/moth");

module.exports = grammar({
    name: "moth_template",
    extras: $ => [/\s+/, $.comment],
    word: $ => $.identifier,
    rules: {
        template_file: $ => repeat(choice($.template_text, $.construction)),
        ...mothRules,
    },
});
