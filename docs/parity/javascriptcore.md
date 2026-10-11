# JavaScriptCore parity: script values and syntax check

Wave 2 closes two rows of the JavaScriptCore section of the parity matrix. Both are pure Rust, std only, and
built on the existing JSON reader (`bifrost_core::json`). Nothing here runs a script: BifrOSt's only script engine
stays inside the browser shell, and apps still reach scripts through a web view.

## What was built

- `springboard/sdk/src/script_value.rs` (`ScriptValue`): a typed tree for values that cross between an app and a
  script. It reads the eval answer (`ScriptValue::from_eval(ty, json)`, the same `ty`/`json` pair as
  `webview_dev::EvalValue`), applies the JavaScript conversion rules (`to_bool`, `to_number`, `to_text`),
  checks types (`type_name`, `is_undefined`, `is_null`, `as_str`, `as_number`, `as_bool`), reads properties and
  elements (`property`, `element`), and writes JSON (`to_json`). Object keys keep their insertion order.
  Functions, symbols, promises, dates and DOM elements are out of scope; an `element` answer is an error.
- `springboard/sdk/src/script_syntax.rs` (`check`, `SyntaxError`): a lexical well-formedness check. It reads strings,
  template literals with `${}` nesting, line and block comments, and regular expression literals (a `/` after an
  operator or at the start opens one), and it matches brackets. It reports unterminated strings, comments, regular
  expressions and templates, unclosed groups, and mismatched or stray closers, each with a line and column.
  It does not check keywords, declarations or operator placement, so a script can pass here and still fail to run.

Both modules have their own `#[cfg(test)] mod tests` (6 tests in `script_value`, 7 in `script_syntax`).
No tests were run in this wave.

## Official API names mirrored

- `JSValue` (creating and converting values, type tests, property access):
  https://developer.apple.com/documentation/javascriptcore/jsvalue
- Framework landing page (the execution model that `ScriptValue` maps onto):
  https://developer.apple.com/documentation/javascriptcore
- Syntax check (`JSCheckScriptSyntax`, C API). The DocC path returned 404 during this wave, so the name comes from the
  parity matrix row, not from the rendered page.

## REA evidence

- Landing page: `ev_af92cb78a4b52ec66c588d66a24008817c9913d5ee9a67cbd7a9131bd30eca81`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/javascriptcore`)

## Matrix rows closed

| Framework | Feature (as in data/javascriptcore.json) | New status | BifrOSt path |
|---|---|---|---|
| JavaScriptCore | JavaScript values | partial | springboard/sdk/src/script_value.rs |
| JavaScriptCore | Syntax checking | partial | springboard/sdk/src/script_syntax.rs |

`JavaScript values` stays partial: there is no way to call a function or invoke a method on a value, and no
object identity. `Syntax checking` stays partial: it is lexical only.

## Left open

- Native object export (`JSExport`) needs a script engine that the SDK can reach. Not built.
- Standalone script evaluation without a web view, and exception handling for standalone scripts. Not built.
- The C API and big integers have no Rust counterpart here.
