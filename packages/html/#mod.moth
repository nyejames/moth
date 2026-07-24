-- Basic HTML components and utility formatters for HTML project builds.
-- Provided by the HTML project builder as a source-backed Builder package.
import @web/canvas {
    Canvas2d,
    get_canvas as raw_get_canvas,
    context_2d as raw_context_2d,
    set_fill_style as raw_set_fill_style,
    fill_rect as raw_fill_rect,
    clear_rect as raw_clear_rect,
    stroke_rect as raw_stroke_rect,
}

set_fill_style |this ~Canvas, color String|:
    raw_set_fill_style(~this.context, color)
;

fill_rect |this ~Canvas, x Float, y Float, width Float, height Float|:
    raw_fill_rect(~this.context, x, y, width, height)
;

clear_rect |this ~Canvas, x Float, y Float, width Float, height Float|:
    raw_clear_rect(~this.context, x, y, width, height)
;

stroke_rect |this ~Canvas, x Float, y Float, width Float, height Float|:
    raw_stroke_rect(~this.context, x, y, width, height)
;

export:
    -- Basic tags
    p #String = [$html:<p style="[$slot("style")]">[$slot]</p>]
    h1 #String = [$html:<h1 style="[$slot("style")]">[$slot]</h1>]
    h2 #String = [$html:<h2 style="[$slot("style")]">[$slot]</h2>]
    h3 #String = [$html:<h3 style="[$slot("style")]">[$slot]</h3>]
    h4 #String = [$html:<h4 style="[$slot("style")]">[$slot]</h4>]
    h5 #String = [$html:<h5 style="[$slot("style")]">[$slot]</h5>]
    h6 #String = [$html:<h6 style="[$slot("style")]">[$slot]</h6>]
    
    small #String = [$html:<small style="[$slot("style")]">[$slot]</small>]
    large #String = [$html:<large style="[$slot("style")]">[$slot]</large>]

    img #String = [$html:<img src="[$slot(1)]" style="[$slot("style")]" alt="[$slot]">]

    -- Formatting helpers
    bold #String = [$html:<b style="[$slot("style")]">[$slot]</b>]
    italic #String = [$html:<i style="[$slot("style")]">[$slot]</i>]
    underline #String = [$html:<u style="[$slot("style")]">[$slot]</u>]
    strikethrough #String = [$html:<s style="[$slot("style")]">[$slot]</s>]

    -- Structure
    div #String = [$html:<div style="[$slot("style")]">[$slot]</div>]
    span #String = [$html:<span style="[$slot("style")]">[$slot]</span>]

    -- Code blocks
    codeblock #= [$html:
        <pre style="[$slot("style")]">[$slot]</pre>
    ]

    -- Basic table components
    table #String = [$html:
    <table style="[$slot("style")]">
        [$note: The first row can be styled differently to the rest of the table rows.]
        <tr style="[$slot("header_style")]">
            [$slot(1)]
        </tr>

        [$note: Each remaining row can be styled differently.]
        [$children([$html:<tr style="[$slot("row_style")]">[$slot]</tr>]):
            [$slot]
        ]
    </table>
    ]

    td #= [$html:
        [$children([$html: <td style="[$slot("style")]">[$slot]</td>]):
            [$slot]
        ]
    ]

    th #= [$html:
        [$children([$html:
            <th scope="col" style="[$slot("style")]">[$slot]</th>
        ]):
            [$slot]
        ]
    ]

    -- Utility style inserts
    center #String = [$insert("style"):text-align: center;]
    right #String = [$insert("style"):text-align: right;]
    left #String = [$insert("style"):text-align: left;]
    smaller #String = [$insert("style"):font-size: smaller;]
    medium #String = [$insert("style"):font-size: medium;]
    larger #String = [$insert("style"):font-size: larger;]

    -- Basic Canvas component
    canvas #String = [$html:<canvas id="[$slot("id")]" style="[$slot("style")]"></canvas>]

    -- Get the basic canvas element and 2D context for drawing (opaque JS interop)
    get_canvas_context |id String| -> Canvas2d, Error!:
        canvas_element = raw_get_canvas(id)!
        return raw_context_2d(canvas_element)!
    ;

    -- Source-owned wrapper that provides method-style canvas drawing ergonomics.
    Canvas = |
        context Canvas2d,
    |

    get_canvas |id String| -> Canvas, Error!:
        context = get_canvas_context(id)!
        return Canvas(context)
    ;
;
