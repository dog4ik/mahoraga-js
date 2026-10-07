use std::{cell::RefCell, collections::HashMap, rc::Rc};

use js_sys::{
    Array as JsArray, Error, Function as JsFunction, Object as JsObject, RangeError, Reflect,
    TypeError,
};
use mahoraga::{
    Args, Callable, ErrorKind, Function, FunctionCallError, RuntimeError, Value,
    value::{Array, Number, Object},
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TS_API: &str = r#"
/**
 * Evaluate a mahoraga expression.
 *
 * `context` keys become top-level identifiers in the expression. Functions in
 * `context` are callable from the expression; their `length` is the arity.
 * `maxLength` bounds the expression size (in UTF-16 units) to keep deeply
 * nested input from exhausting the stack.
 *
 * @throws {MahoragaError} when the expression fails to tokenize, parse or evaluate,
 * including when a context function throws; the thrown value is the error's `cause`.
 * @throws {TypeError} for unsupported context values or a function result.
 * @throws {RangeError} when `expr` exceeds `maxLength`, the context is nested too deep
 * or a bigint does not fit into 64 bits.
 */
export function evaluate(expr: string, context?: Record<string, unknown> | null, maxLength?: number): unknown;

export interface MahoragaError extends Error {
  name: "LexerError" | "ParserError" | "EvalError";
  help: string | null;
  /** `[start, end)` UTF-16 offsets into `expr`, so `expr.slice(...span)` works directly. */
  span: [start: number, end: number];
  expr: string;
  /** What a context function threw, when that is what failed. */
  cause?: unknown;
}
"#;

const MAX_CONTEXT_DEPTH: usize = 128;

const DEFAULT_MAX_LENGTH: u32 = 2_000;

thread_local! {
    /// What the last failing JS function threw, attached as `cause` to the resulting EvalError.
    static LAST_THROWN: RefCell<Option<JsValue>> = const { RefCell::new(None) };
}

/// A JS function exposed to mahoraga. Arity comes from `fn.length`.
struct JsCallable {
    f: JsFunction,
    name: Rc<str>,
}

impl Callable for JsCallable {
    fn call(&self, Args(args): Args) -> Result<Value, FunctionCallError> {
        let fail = |message: String, thrown: JsValue| {
            LAST_THROWN.set(Some(thrown));
            FunctionCallError::External(message)
        };
        let js_args = JsArray::new_with_length(args.len() as u32);
        for (i, v) in args.into_iter().enumerate() {
            let v = to_js(v).map_err(|e| {
                let message = format!("argument {} of {}: {}", i + 1, self.name, error_message(&e));
                fail(message, e)
            })?;
            js_args.set(i as u32, v);
        }
        let out = self
            .f
            .apply(&JsValue::UNDEFINED, &js_args)
            .map_err(|e| fail(format!("{} threw: {}", self.name, error_message(&e)), e))?;
        from_js(&out, 0).map_err(|e| {
            fail(
                format!(
                    "{} returned an unsupported value: {}",
                    self.name,
                    error_message(&e)
                ),
                e,
            )
        })
    }

    fn arity(&self) -> usize {
        self.f.length() as usize
    }
}

fn js_function(f: JsFunction) -> Value {
    let name = f.name().as_string().filter(|n| !n.is_empty());
    let name: Rc<str> = Rc::from(name.as_deref().unwrap_or("anonymous"));
    Value::Function(Rc::new(Function {
        name: name.clone(),
        help: None,
        f: Box::new(JsCallable { f, name }),
    }))
}

/// `message` of an Error, the value itself if it's a string.
fn error_message(v: &JsValue) -> String {
    if let Some(e) = v.dyn_ref::<Error>() {
        return e.message().into();
    }
    v.as_string()
        .unwrap_or_else(|| format!("non-Error value of type '{}'", type_name(v)))
}

fn from_js(v: &JsValue, depth: usize) -> Result<Value, JsValue> {
    if depth > MAX_CONTEXT_DEPTH {
        return Err(RangeError::new(&format!(
            "value is nested deeper than {MAX_CONTEXT_DEPTH} levels"
        ))
        .into());
    }
    if v.is_null() {
        return Ok(Value::Null);
    }
    if v.is_undefined() {
        return Ok(Value::Void);
    }
    if let Some(b) = v.as_bool() {
        return Ok(Value::Bool(b));
    }
    if let Some(f) = v.as_f64() {
        // JS can't tell 1.0 from 1, treat every safe integer as an integer.
        return Ok(Value::Number(if js_sys::Number::is_safe_integer(v) {
            Number::Int(f as i64)
        } else {
            Number::Float(f)
        }));
    }
    if v.is_bigint() {
        let i = i64::try_from(v.clone())
            .map_err(|_| RangeError::new("bigint does not fit into 64 bits"))?;
        return Ok(Value::Number(Number::Int(i)));
    }
    if let Some(s) = v.as_string() {
        return Ok(Value::String(s));
    }
    if JsArray::is_array(v) {
        let arr = JsArray::from(v);
        let items = arr.iter().map(|v| from_js(&v, depth + 1));
        return Ok(Value::Array(Array(items.collect::<Result<_, _>>()?)));
    }
    if is_plain_object(v) {
        return Ok(Value::Object(object_from_js(v.unchecked_ref(), depth + 1)?));
    }
    if let Some(f) = v.dyn_ref::<JsFunction>() {
        return Ok(js_function(f.clone()));
    }
    Err(TypeError::new(&format!("unsupported value of type '{}'", type_name(v))).into())
}

fn is_plain_object(v: &JsValue) -> bool {
    if !v.is_object() || v.is_function() {
        return false;
    }
    let proto = JsObject::get_prototype_of(v);
    proto.is_null() || JsObject::is(&proto, &JsObject::get_prototype_of(&JsObject::new()))
}

/// `typeof`, or the constructor name for objects (`Map`, `Date`, ...).
fn type_name(v: &JsValue) -> String {
    let type_of = v.js_typeof().as_string().unwrap_or_default();
    if type_of != "object" {
        return type_of;
    }
    Reflect::get(v, &"constructor".into())
        .and_then(|c| Reflect::get(&c, &"name".into()))
        .ok()
        .and_then(|n| n.as_string())
        .unwrap_or(type_of)
}

fn object_from_js(obj: &JsObject, depth: usize) -> Result<Object, JsValue> {
    let entries = JsObject::entries(obj);
    let mut map = HashMap::with_capacity(entries.length() as usize);
    for entry in entries.iter() {
        let entry = JsArray::from(&entry);
        let key = entry.get(0).as_string().unwrap_or_default();
        map.insert(key, from_js(&entry.get(1), depth)?);
    }
    Ok(Object(map))
}

fn to_js(value: Value) -> Result<JsValue, JsValue> {
    Ok(match value {
        Value::String(s) => JsValue::from_str(&s),
        // Integers beyond 2^53 lose precision, same as JSON.parse.
        Value::Number(Number::Int(i)) => JsValue::from_f64(i as f64),
        Value::Number(Number::Float(f)) => JsValue::from_f64(f),
        Value::Bool(b) => JsValue::from_bool(b),
        Value::Object(Object(map)) => {
            // Object.fromEntries defines own properties, so a "__proto__" key stays a plain key.
            let entries = JsArray::new_with_length(map.len() as u32);
            for (i, (k, v)) in map.into_iter().enumerate() {
                entries.set(i as u32, JsArray::of2(&k.into(), &to_js(v)?).into());
            }
            JsObject::from_entries(&entries)?.into()
        }
        Value::Array(Array(items)) => {
            let arr = JsArray::new_with_length(items.len() as u32);
            for (i, v) in items.into_iter().enumerate() {
                arr.set(i as u32, to_js(v)?);
            }
            arr.into()
        }
        Value::Function(_) => {
            return Err(TypeError::new("functions can't be converted to JS values").into());
        }
        Value::Null => JsValue::NULL,
        Value::Void => JsValue::UNDEFINED,
    })
}

/// Span offsets from mahoraga are byte indices, JS strings are indexed by UTF-16 units.
fn utf16_offset(expr: &str, byte: usize) -> usize {
    let byte = byte.min(expr.len());
    // Errors may point inside a multibyte char, round down to its start.
    let byte = (0..=byte)
        .rev()
        .find(|&b| expr.is_char_boundary(b))
        .unwrap_or(0);
    expr[..byte].encode_utf16().count()
}

fn map_err(expr: &str, err: ErrorKind, thrown: Option<JsValue>) -> JsValue {
    let (name, message) = match &err {
        ErrorKind::Lexer(e) => ("LexerError", e.inner.to_string()),
        ErrorKind::Parser(e) => ("ParserError", e.inner.to_string()),
        ErrorKind::Runtime(e) => ("EvalError", e.inner.to_string()),
    };
    let js_err = Error::new(&message);
    js_err.set_name(name);
    let span = err.span();
    let span = JsArray::of2(
        &(utf16_offset(expr, span.start) as u32).into(),
        &(utf16_offset(expr, span.end) as u32).into(),
    );
    let help = err.help().map(JsValue::from).unwrap_or(JsValue::NULL);
    // Setting fields on a fresh Error can't fail.
    let _ = Reflect::set(&js_err, &"help".into(), &help);
    let _ = Reflect::set(&js_err, &"span".into(), &span);
    let _ = Reflect::set(&js_err, &"expr".into(), &expr.into());
    if let ErrorKind::Runtime(e) = &err
        && let RuntimeError::FunctionCallError(FunctionCallError::External(_)) = &e.inner
        && let Some(thrown) = thrown
    {
        let _ = Reflect::set(&js_err, &"cause".into(), &thrown);
    }
    js_err.into()
}

#[wasm_bindgen(js_name = evaluate, skip_typescript)]
pub fn evaluate(expr: &str, context: JsValue, max_length: Option<u32>) -> Result<JsValue, JsValue> {
    let max_length = max_length.unwrap_or(DEFAULT_MAX_LENGTH) as usize;
    let len = expr.encode_utf16().count();
    if len > max_length {
        return Err(RangeError::new(&format!(
            "expression is {len} chars long, maxLength is {max_length}"
        ))
        .into());
    }
    let mut env = mahoraga::Env::std();
    if !context.is_null() && !context.is_undefined() {
        if !is_plain_object(&context) {
            return Err(TypeError::new("context must be a plain object").into());
        }
        env.attach_object(object_from_js(context.unchecked_ref(), 0)?);
    }
    let result = mahoraga::eval_str(expr, &env);
    // Always take it, so a thrown value never outlives the call that caught it.
    let thrown = LAST_THROWN.take();
    let value = result.map_err(|e| map_err(expr, e, thrown))?;
    to_js(value)
}
