# mahoraga-js

WebAssembly bindings for the [mahoraga](https://github.com/dog4ik/mahoraga) expression language, built for bundlers (Vite, webpack).

```js
import { evaluate } from "mahoraga-js";

evaluate("1 + 2 * 5");                                          // 11
evaluate("user.name | to_uppercase()", { user: { name: "ann" } }); // "ANN"
evaluate("price | discount(10)", { price: 50, discount: (p, pct) => p * (1 - pct / 100) }); // 45

try {
  evaluate("'test' + 2");
} catch (err) {
  // err.name: "LexerError" | "ParserError" | "EvalError"
  console.log(err.message, err.span, err.help);
}
```

Vite 8 loads the `.wasm` without plugins. webpack 5 needs `experiments: { asyncWebAssembly: true }`.

## API

`evaluate(expr, context?, maxLength = 2000)` evaluates `expr` against the standard library.
Keys of `context` become top-level identifiers.

| JS                                   | mahoraga |
|--------------------------------------|----------|
| `null`                               | `null`   |
| `undefined`                          | `void`   |
| boolean                              | bool     |
| number (safe integer)                | integer  |
| number (otherwise)                   | float    |
| bigint (fits in i64)                 | integer  |
| string                               | string   |
| plain object (or null prototype)     | object   |
| `Array`                              | array    |
| function                             | function |

Results map back the same way. Integers come back as `number`, so values beyond 2⁵³ lose precision.
JS can't distinguish `1.0` from `1`, so both become the integer `1`.

### Functions

Functions in `context` can be called from the expression, including with pipes (`x | f(a)` calls `f(x, a)`).
Context entries shadow std functions with the same name.

- Arity is `fn.length`, and calls must match it exactly. Rest and default parameters don't count toward `length`, so for a variadic function set it yourself: `Object.defineProperty(sum, "length", { value: 3 })`.
- Arguments and return values convert as in the table above. Methods are called with `this` set to `undefined`, so bind them first.
- A mahoraga function can't be passed to a JS function, and an expression can't return one.
- If the function throws, or returns an unsupported value, `evaluate` throws an `EvalError` whose `span` covers the call and whose `cause` is the original thrown value.
- A function may call `evaluate` itself. Each nested call adds to the native stack, so keep `maxLength` low when nesting deeply.

### Errors

Failures in the expression throw an `Error` whose `name` is `LexerError`, `ParserError` or `EvalError`, typed as `MahoragaError`. It carries these fields:

- `help`: a hint, or `null`
- `span`: `[start, end]` UTF-16 offsets, so `expr.slice(...err.span)` gives the offending text
- `expr`
- `cause`: what a context function threw, if that's what failed

Unsupported context values throw `TypeError` (Map, Date, class instances, symbols).
An expression longer than `maxLength`, context nested deeper than 128 levels (including cycles), or a bigint outside i64 throws `RangeError`.

## Why `maxLength`

mahoraga recurses once per nesting level. Wasm frames live on the JS engine's native stack, which is about 1 MB in V8. Input like `!!!…1` overflows it at around 5,400 chars in Node and Chrome. A stack overflow is an unrecoverable engine trap, not a `MahoragaError`. The 2,000 default leaves room for calls made from deep inside your own call stack. Raise it only if you need to.

The wasm stack itself is raised to 8 MB in `.cargo/config.toml`. The 1 MB default trapped at about 1,500 levels.

## Development

```sh
cargo install wasm-pack
wasm-pack build --release --target bundler                     # pkg/, the npm package
wasm-pack build --release --target nodejs --out-dir target/pkg-node
node --test 'tests/*.test.mjs'
```

`Cargo.toml` depends on `../mahoraga` by path. Switch it to a git rev or a crates.io version once the core crate is pushed or published.
