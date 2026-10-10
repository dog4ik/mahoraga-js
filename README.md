# mahoraga-js

WebAssembly bindings for the [mahoraga](https://github.com/dog4ik/mahoraga) expression language

```sh
npm install mahoraga-js
```

```js
import { evaluate } from "mahoraga-js";

evaluate("1 + 2 * 5");                                          // 11
evaluate("user.name | to_uppercase()", { user: { name: "ann" } }); // "ANN"
evaluate("price | discount(10)", { price: 50, discount: (p, pct) => p * (1 - pct / 100) }); // 45

function adder(n: number, mahoraga_closure: (n: number) => number) {
    console.log("Hi from javascript!");
    let result = mahoraga_closure(n);
    console.log(`Mahoraga result: ${result}`);
    return (add: number) => result + add;
}

evaluate("adder(value, |v| v + 10)(10)", { value: 0, adder }); 
// Hi from javascript!
// Mahoraga result: 10
// 20

try {
  evaluate("'test' + 2");
} catch (err) {
  // err.name: "LexerError" | "ParserError" | "EvalError"
  console.log(err.message, err.span, err.help);
}
```

Vite 8 loads the `.wasm` without plugins, but the dev server's dependency pre-bundling must skip the package.
Otherwise the wasm and its JS glue end up as separate module instances and loading fails with `wasm is undefined`:

```js
// vite.config.js
export default defineConfig({
  optimizeDeps: { exclude: ["mahoraga-js"] },
});
```

Node 24+ can import it directly, with an experimental-feature warning.

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
| function                             | closure  |

## Development

```sh
cargo install wasm-pack
wasm-pack build --release --target bundler                     # pkg/, the npm package
wasm-pack build --release --target nodejs --out-dir target/pkg-node
node --test 'tests/*.test.mjs'
```
