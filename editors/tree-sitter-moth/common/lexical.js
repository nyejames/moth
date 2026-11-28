// Shared spelling rules. Compiler and MON codec remain authoritative for validity.
const number = /-?[0-9](_?[0-9])*(\.[0-9](_?[0-9])*)?(e[+-]?[0-9](_?[0-9])*)?/;
const identifier = /[_\p{Alphabetic}][_\p{Alphabetic}\p{N}]*/u;
const mothString = /"([^"\\\r\n]|\\["\\nrt])*"/;
const mothChar = /'([^'\\\r\n]|\\['\\nrt])'/;
const monString = /"([^"\\]|\\(["\\nrt]|u\{[0-9a-fA-F]{1,6}\}))*"/;
const monChar = /'([^'\\]|\\(['\\nrt]|u\{[0-9a-fA-F]{1,6}\}))'/;

function lexicalRules(mon = false) {
    return {
        identifier: _ => identifier,
        number: _ => number,
        string: _ => mon ? monString : mothString,
        character: _ => mon ? monChar : mothChar,
        boolean: _ => choice("true", "false"),
        none: _ => "none",
        comment: _ => token(seq("--", /[^\r\n]*/)),
        _literal: $ => choice($.string, $.character, $.number, $.boolean, $.none),
    };
}

module.exports = { lexicalRules };
