# Grapher

Grapher (`springboard/apps/grapher-desktop`, crate `bifrost-grapher`, id `is.olibuijr.grapher.desktop`) plots equations
on a grid. It is a full-size window with the SDK kit: a toolbar on the unified bar, an equation list in a sidebar, and
the plot on the right.

## Equations

Each line of the sidebar is one equation, drawn in its own colour (blue, red, green, orange, purple, teal, in that
order, then again).

| Form | Meaning | Example |
|---|---|---|
| `y = f(x)` or `f(x)` | a graph of a function of `x` | `y = x^2 / 4 - 1` |
| `x = X(t), y = Y(t)` | a parametric curve | `x = cos(3t), y = sin(2t)` |
| `r = R(t)` | a polar curve, `t` is the angle in radians | `r = 2 + cos(3t)` |
| `...; t in [a, b]` | the range of `t` for a parametric or polar curve (default `[0, 2 pi]`) | `x = cos(t), y = sin(t); t in [0, 3]` |

Expressions use `+ - * / ^`, parentheses, the variables `x` and `t` (`theta` means `t`), the constants `pi`, `tau` and
`e`, and the functions `sin cos tan asin acos atan sinh cosh tanh sqrt abs ln log (base 10) exp floor ceil`. A product
can drop the `*` (`2x`, `3(x + 1)`). `^` is right-associative and binds tighter than a unary minus, so `-2^2` is -4.

A line that does not parse shows its error under the field (`unexpected character '$' at position 3`). Such an
equation is kept but not drawn. A function is cut where it leaves the domain (`sqrt(x)` for negative `x`) and where it
jumps across the plot (`1/x` at zero, `tan(x)` at its poles), so a pole does not draw a line across the plot.

## Working with the plot

- Type into the field under the list. Return starts a new equation; Backspace deletes the last character. Typing with
  nothing selected starts an equation.
- Tap an equation in the list to select it. Add and Remove act on the selected equation.
- Drag the plot to pan. Zoom In and Zoom Out keep the centre of the plot in place; Reset View returns to the origin at
  40 pixels per unit.
- Both axes share one scale, so circles stay round. The grid spacing (1, 2 or 5 times a power of ten) follows the zoom.
- Export Image writes the plot as a PNG (the size of the plot area) into the app's data folder under `exports/`, and
  the status line under the field shows the path.

## Storage

The equations are saved to `equations.txt` in the app's data folder, one per line, every time they change. The first run
starts with three examples. The view is not saved.

## Code

| File | Contents |
|---|---|
| `src/expr.rs` | tokenizer, parser and evaluator for expressions in `x` and `t` |
| `src/equation.rs` | the forms above, and sampling into polylines (one sample per pixel column for `y = f(x)`, 2001 steps for parametric and polar curves) |
| `src/view.rs` | the view (centre and scale), pan and zoom, grid steps and tick labels |
| `src/model.rs` | the equations, colours, selection and the text file |
| `src/plot.rs` | drawing the plot on a surface; the export draws the same routine into a buffer |
| `src/png.rs` | a PNG encoder (stored deflate blocks, no dependency) |
| `src/window.rs` | state, geometry in points, hit testing, touch and key handling, saving |
| `src/draw.rs` | the window chrome: toolbar, sidebar, field and status lines |

Unit tests cover the expression language, the equation forms and their sampling, the view math, the model edits, and the
PNG container. The drawing code has no test yet; check it with a Midget screenshot in Light and Dark.

## Not yet

- Keyboard zoom and a scroll-wheel or pinch zoom (the SDK's input here has no wheel or pinch event).
- An export size other than the plot's size, and a Save panel (the export goes to the app's folder).
- Implicit equations such as `x^2 + y^2 = 1`, and vertical lines `x = a`.
- The app icon: `icon.png` is referenced by the manifest and is not in the crate yet.
