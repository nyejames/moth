# Quick note of noticed bad diagnostics to fix

## HTML warnings in long `$html` bodies point at the template opener

```moth
sidebar #= [$html:
    <p>(more than about 1 KB of markup)</p>
    <script>
        console.log("hello")
    </script>
]
```

Short bodies now underline the exact `<script` tag, but a body longer than about 1 KB still labels `[$html:` on the declaration line. The body token's span is stored in the extended span table, and `SourceSpan::narrow_inline` cannot read its start offset without the source's `ExtendedSpanBuilder`, which formatter execution can't reach. Fix it by threading span-builder access into `TemplateFormatter::format` or by recording a report-time offset fact that narrows the span when the diagnostic is frozen.

## Mutable alias conflicts don't label the later alias use

```moth
data ~= 42
first ~= data
~data = 8
io.line([:after first=[first]])
```

This is now rejected with MOTH-BORROW-0007 "Cannot mutably access `data` because `first` may alias the same value." The diagnostic labels the alias creation on line 2 but not the use of `first` on line 4 that keeps the alias live. `INVALID_MUTABLE_ACCESS` supports one related site, so it needs a second label slot for the live use.
