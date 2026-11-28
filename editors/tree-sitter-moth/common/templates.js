// Both Moth roots enter the same authored construction grammar.
const { commaList } = require("./records");

const templateRules = {
    construction: $ => seq("[", optional(choice(
        "=",
        $.map_entries,
        seq($.construction_head, optional(seq(":", optional($.template_body)))),
        seq(":", optional($.template_body)),
        seq($.code_directive, ":", optional($.listing_body)),
        seq($.note_directive, ":", optional($.note_body)),
    )), "]"),
    construction_head: $ => commaList(choice($._value, $.formatter_directive)),
    formatter_directive: $ => choice(
        "$html", "$md", "$raw", "$escape_html",
        seq("$css", optional($.string)),
    ),
    code_directive: $ => seq("$code", optional($.string)),
    note_directive: _ => "$note",
    template_body: $ => repeat1(choice($.template_text, $.construction)),
    // Lexical precedence preserves raw whitespace and -- as text, not extras.
    // Confirm recovery and extra-token interaction with real parser generation.
    template_text: _ => token(prec(2, /[^\[\]]+/)),
    listing_body: $ => repeat1(choice($.listing_text, $.listing_brackets)),
    listing_brackets: $ => seq("[", optional($.listing_body), "]"),
    listing_text: _ => token(prec(2, /[^\[\]]+/)),
    note_body: $ => repeat1(choice($.note_text, $.note_brackets)),
    note_brackets: $ => seq("[", optional($.note_body), "]"),
    note_text: _ => token(prec(2, /[^\[\]]+/)),
};

module.exports = { templateRules };
