-- Regression: valid scalar/template paths compile, invalid struct fails early

-- Valid String
io.line("hello")

-- Valid Int
io.line([: [42]])

-- Valid Float
io.line([: [3.14]])

-- Valid Bool
io.line([: [true]])

-- Valid Char (via template)
c = 'A'
[:char_value [c]]

-- Valid template passed to io
message ~= [:template_test]
io.line([: [message]])

-- Invalid struct should fail while the above compiled fine
Point = |
    x Int,
    y Int,
|

point = Point(1, 2)
io.line(point)
