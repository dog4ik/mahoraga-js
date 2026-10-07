import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const { evaluate } = createRequire(import.meta.url)("../target/pkg-node/mahoraga_js.js");

function throwsMahoraga(fn, name) {
  try {
    fn();
  } catch (err) {
    assert.ok(err instanceof Error);
    assert.equal(err.name, name);
    return err;
  }
  assert.fail("did not throw");
}

test("arithmetic", () => {
  assert.equal(evaluate("1 + 2 * 5"), 11);
  assert.equal(evaluate("1.5 * 2"), 3);
});

test("scalars round trip", () => {
  const ctx = { i: 7, f: 1.25, b: true, n: null, u: undefined, s: "hi", big: 5n };
  assert.equal(evaluate("i", ctx), 7);
  assert.equal(evaluate("f", ctx), 1.25);
  assert.equal(evaluate("b", ctx), true);
  assert.equal(evaluate("n", ctx), null);
  assert.equal(evaluate("u", ctx), undefined);
  assert.equal(evaluate("s", ctx), "hi");
  assert.equal(evaluate("big", ctx), 5);
});

test("nested context", () => {
  const ctx = { user: { name: "ann", tags: ["a", "b"] } };
  assert.equal(evaluate("user.name | to_uppercase()", ctx), "ANN");
  assert.equal(evaluate("user.tags[1]", ctx), "b");
  assert.deepEqual(evaluate("user", ctx), { name: "ann", tags: ["a", "b"] });
});

test("integers index arrays", () => {
  assert.equal(evaluate("xs[i]", { xs: [10, 20], i: 1 }), 20);
});

test("missing identifier is undefined", () => {
  assert.equal(evaluate("missing"), undefined);
});

test("null prototype context", () => {
  const ctx = Object.assign(Object.create(null), { x: 1 });
  assert.equal(evaluate("x", ctx), 1);
});

test("__proto__ key stays a plain property", () => {
  const ctx = JSON.parse('{"o": {"__proto__": {"polluted": true}}}');
  const out = evaluate("o", ctx);
  assert.equal(Object.getPrototypeOf(out), Object.prototype);
  assert.deepEqual(Object.keys(out), ["__proto__"]);
  assert.equal({}.polluted, undefined);
});

test("eval error", () => {
  const err = throwsMahoraga(() => evaluate("'test' + 2"), "EvalError");
  assert.match(err.help, /to_s/);
  assert.deepEqual(err.span, [0, 10]);
  assert.equal(err.expr, "'test' + 2");
});

test("parser error", () => {
  const err = throwsMahoraga(() => evaluate("1 +"), "ParserError");
  const [start, end] = err.span;
  assert.ok(0 <= start && start <= end && end <= 3);
});

test("lexer error", () => {
  throwsMahoraga(() => evaluate("'unterminated"), "LexerError");
});

test("spans are UTF-16 offsets", () => {
  for (const expr of ["'ёжик' + 2", "'🦀🦀' + 2"]) {
    const err = throwsMahoraga(() => evaluate(expr), "EvalError");
    assert.deepEqual(err.span, [0, expr.length]);
  }
});

test("unsupported context values", () => {
  for (const x of [new Map(), new Date(), Symbol("s")]) {
    assert.throws(() => evaluate("x", { x }), TypeError);
  }
  assert.throws(() => evaluate("x", [1]), TypeError);
});

test("context functions are callable", () => {
  const ctx = { x: 2, double: (n) => n * 2, utils: { add: (a, b) => a + b } };
  assert.equal(evaluate("double(x)", ctx), 4);
  assert.equal(evaluate("x | double()", ctx), 4);
  assert.equal(evaluate("utils.add(1, 2) + double(3)", ctx), 9);
});

test("context functions convert arguments and results", () => {
  const ctx = {
    o: { b: 1, a: [2] },
    keys: (o) => Object.keys(o).sort(),
    wrap: (xs) => ({ first: xs[0], n: xs.length }),
  };
  assert.deepEqual(evaluate("keys(o)", ctx), ["a", "b"]);
  assert.deepEqual(evaluate("wrap([7, 8]).first", ctx), 7);
  assert.equal(evaluate("wrap(['x']).n", ctx), 1);
});

test("context functions shadow std functions", () => {
  assert.equal(evaluate("len()", { len: () => 42 }), 42);
});

test("context function arity is its length", () => {
  const err = throwsMahoraga(() => evaluate("f(1, 2)", { f: (a) => a }), "EvalError");
  assert.match(err.message, /expected 1/);
  const sum = (...xs) => xs.reduce((a, b) => a + b, 0);
  Object.defineProperty(sum, "length", { value: 3 });
  assert.equal(evaluate("sum(1, 2, 3)", { sum }), 6);
});

test("a throwing context function becomes an EvalError", () => {
  const boom = new Error("boom");
  const expr = "1 + fail(1)";
  const err = throwsMahoraga(() => evaluate(expr, { fail: (_) => { throw boom; } }), "EvalError");
  assert.match(err.message, /fail threw: boom/);
  assert.equal(err.cause, boom);
  assert.equal(expr.slice(...err.span), "fail(1)");

  const str = throwsMahoraga(() => evaluate("f()", { f() { throw "nope"; } }), "EvalError");
  assert.match(str.message, /nope/);
  assert.equal(str.cause, "nope");
});

test("errors not thrown by a context function have no cause", () => {
  const ctx = { fail: () => { throw new Error("boom"); } };
  assert.throws(() => evaluate("fail()", ctx));
  const err = throwsMahoraga(() => evaluate("'a' + 1", ctx), "EvalError");
  assert.equal(err.cause, undefined);
});

test("unsupported context function values", () => {
  const ret = throwsMahoraga(() => evaluate("f()", { f: () => new Map() }), "EvalError");
  assert.match(ret.message, /f returned an unsupported value/);
  assert.ok(ret.cause instanceof TypeError);

  const arg = throwsMahoraga(() => evaluate("f([to_s])", { f: (x) => x }), "EvalError");
  assert.match(arg.message, /argument 1 of f/);
  assert.ok(arg.cause instanceof TypeError);

  assert.throws(() => evaluate("f", { f: () => 1 }), TypeError);
});

test("context functions may call evaluate", () => {
  const ctx = {
    inner: (s) => evaluate(s, { y: 10 }),
    swallow: () => {
      try {
        evaluate("'a' + 1");
      } catch {}
      return 1;
    },
  };
  assert.equal(evaluate("inner('y * 2') + 1", ctx), 21);
  assert.equal(evaluate("swallow()", ctx), 1);
});

test("bigint overflow", () => {
  assert.throws(() => evaluate("x", { x: 2n ** 63n }), RangeError);
});

test("self referencing context", () => {
  const d = {};
  d.d = d;
  assert.throws(() => evaluate("d", d), RangeError);
});

test("maxLength", () => {
  assert.throws(() => evaluate("1".repeat(2001)), RangeError);
  assert.throws(() => evaluate("1 + 1", null, 3), RangeError);
  assert.equal(evaluate("1 + 1", null, 5), 2);
});

test("deep nesting at maxLength never traps", () => {
  const n = 2000;
  const cases = [
    "(".repeat(n),
    "!".repeat(n - 1) + "1",
    "-".repeat(n - 1) + "1",
    "1?".repeat(499) + "1" + ":1".repeat(499),
    "[".repeat(n / 2) + "]".repeat(n / 2),
    "to_s(".repeat(n / 6) + "1" + ")".repeat(n / 6),
  ];
  for (const expr of cases) {
    assert.ok(expr.length <= n);
    try {
      evaluate(expr);
    } catch (err) {
      assert.ok(["LexerError", "ParserError", "EvalError"].includes(err.name), String(err));
    }
  }
  assert.equal(evaluate("1 + 1"), 2);
});
