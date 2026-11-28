// Shared expression subset for ordinary code and implicit template roots.
const { lexicalRules } = require("./lexical");
const { recordRules } = require("./records");
const { templateRules } = require("./templates");

const mothRules = {
    ...lexicalRules(),
    ...recordRules,
    ...templateRules,
    _value: $ => choice($._literal, $.identifier, $.record, $.construction),
};

module.exports = { mothRules };
