(comment) @comment
(string) @string
(character) @character
(number) @number
(boolean) @boolean
(none) @constant.builtin
(record_field name: (identifier) @property)
(field_separator) @operator
(map_separator) @operator
["(" ")" "[" "]"] @punctuation.bracket
"," @punctuation.delimiter
"=" @operator
(template_text) @string
(note_body) @comment
(formatter_directive ["$html" "$css" "$md" "$raw" "$escape_html"] @keyword.directive)
"$code" @keyword.directive
"$note" @keyword.directive
":" @punctuation.delimiter
