// Builds per-function call-flow trees (evaluation order + control structure) from a TS source file.
"use strict";
const ts = require("typescript");
const fs = require("fs");

const file = process.argv[2];
const text = fs.readFileSync(file, "utf8");
const sf = ts.createSourceFile("m.ts", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
const L = n => sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1;
const LE = n => sf.getLineAndCharacterOfPosition(n.getEnd()).line + 1;
const T = (n, max = 64) => {
  const s = n.getText(sf).replace(/\s+/g, " ").trim();
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
};

const imports = {};
const topFns = {};
for (const st of sf.statements) {
  if (ts.isImportDeclaration(st)) {
    const mod = st.moduleSpecifier.text, cl = st.importClause;
    if (cl && cl.namedBindings && ts.isNamedImports(cl.namedBindings))
      for (const el of cl.namedBindings.elements) imports[el.name.text] = { module: mod, typeOnly: cl.isTypeOnly || el.isTypeOnly };
  }
  if (ts.isFunctionDeclaration(st) && st.name) topFns[st.name.text] = st;
}
const BUILTIN = new Set(["Object", "Date", "JSON", "Math", "Array", "Promise", "Number", "String", "Reflect", "console", "Symbol", "Error"]);
const ITER = new Set(["map", "forEach", "filter", "reduce", "some", "every", "flatMap", "find", "findIndex", "sort"]);
const isFn = n => n && (ts.isArrowFunction(n) || ts.isFunctionExpression(n));

function scopeOf(fn) {
  const locals = {}, origins = {}, valueFrom = {};
  const visit = n => {
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.initializer) {
      const nm = n.name.text;
      let init = n.initializer;
      if (isFn(init)) locals[nm] = init;
      if (ts.isAwaitExpression(init)) init = init.expression;
      if (ts.isCallExpression(init) && ts.isIdentifier(init.expression)) {
        if (topFns[init.expression.text]) origins[nm] = init.expression.text;
        else if (imports[init.expression.text]) valueFrom[nm] = init.expression.text;
      }
    }
    ts.forEachChild(n, visit);
  };
  if (fn.body) visit(fn.body);
  return { locals, origins, valueFrom };
}
function paramsOf(fn) {
  const p = {};
  (fn.parameters || []).forEach(x => { if (ts.isIdentifier(x.name)) p[x.name.text] = x.initializer || null; });
  return p;
}
function returnedObject(fn) {
  let found = null;
  const visit = n => {
    if (found || (n !== fn && (isFn(n) || ts.isMethodDeclaration(n)))) return;
    if (ts.isReturnStatement(n) && n.expression && ts.isObjectLiteralExpression(n.expression)) found = n.expression;
    else ts.forEachChild(n, visit);
  };
  if (fn.body) ts.forEachChild(fn.body, visit);
  return found;
}

// ---------- unit discovery ----------
const units = {};
const factoryProps = {}; // factory -> prop -> {unit?, from?}
for (const [name, fn] of Object.entries(topFns)) {
  const sc = scopeOf(fn);
  units[name] = { id: name, owner: name, node: fn, scope: sc, params: paramsOf(fn), start: L(fn), end: LE(fn), async: !!(fn.modifiers || []).find(m => m.kind === ts.SyntaxKind.AsyncKeyword) };
  const obj = returnedObject(fn);
  if (!obj) continue;
  const props = {};
  obj.properties.forEach(p => {
    const pn = p.name && ts.isIdentifier(p.name) ? p.name.text : null;
    if (!pn) return;
    let body = null;
    if (ts.isMethodDeclaration(p)) body = p;
    else if (ts.isPropertyAssignment(p) && isFn(p.initializer)) body = p.initializer;
    else if (ts.isShorthandPropertyAssignment(p) && sc.locals[pn]) body = sc.locals[pn];
    if (body) {
      const id = `${name}.${pn}`;
      units[id] = { id, owner: name, method: pn, node: body, scope: sc, params: Object.assign({}, units[name].params, paramsOf(body)), start: L(body), end: LE(body), async: !!(body.modifiers || []).find(m => m.kind === ts.SyntaxKind.AsyncKeyword) };
      props[pn] = { unit: id };
    } else if (ts.isShorthandPropertyAssignment(p) && sc.valueFrom[pn]) props[pn] = { from: sc.valueFrom[pn] };
    else props[pn] = { value: true };
  });
  if (Object.values(props).some(v => v.unit)) factoryProps[name] = props;
  else Object.keys(props).forEach(k => { if (props[k].unit) delete units[props[k].unit]; });
}

// ---------- walker ----------
const meaningful = items => items.some(i => i && i.t !== "def");
function block(kind, kw, label, children, line, opts = {}) {
  return { t: "block", kind, kw, label, line, children, ...opts };
}
function branches(kind, condLabel, a, b, line, lineB) {
  // Two-way branch: keep both arms when either has calls, so structure stays visible.
  if (!meaningful(a) && !meaningful(b)) return [];
  return [block(kind, "if", condLabel, a, line), block(kind, "else", "", b, lineB || line)];
}

function walkFn(fn, ctx) {
  const c = { ...ctx, params: { ...ctx.params, ...paramsOf(fn) }, argOf: null };
  if (!fn.body) return [];
  return ts.isBlock(fn.body) ? walk(fn.body, c) : walk(fn.body, c);
}

function callItem(node, ctx, isNew) {
  const callee = node.expression;
  let pre = [];
  if (!isNew && (ts.isPropertyAccessExpression(callee) || ts.isElementAccessExpression(callee))) pre = walk(callee.expression, ctx);
  const label = isNew ? "new " + T(callee, 48) : T(callee, 48);
  let target = { kind: "other" }, iterName = null;
  if (ts.isIdentifier(callee)) {
    const n = callee.text;
    if (topFns[n]) target = { kind: "fn", id: n };
    else if (imports[n]) target = { kind: "ext", id: n, module: imports[n].module };
    else if (ctx.scope.locals[n]) target = { kind: "local", id: n };
    else if (n in ctx.params) target = { kind: "param", id: n };
  } else if (ts.isPropertyAccessExpression(callee)) {
    const obj = callee.expression, m = callee.name.text;
    if (ITER.has(m)) iterName = m;
    if (ts.isIdentifier(obj)) {
      const origin = ctx.scope.origins[obj.text];
      const fp = origin && factoryProps[origin] && factoryProps[origin][m];
      if (fp && fp.unit) target = { kind: "method", id: m, unit: fp.unit, of: origin };
      else if (fp && fp.from) target = { kind: "method", id: m, of: origin, note: `function returned by ${fp.from}()` , ext: fp.from };
      else if (obj.text === "harness" && (m === "runAttempt" || m === "classify")) target = { kind: "ext", id: m, module: "harness" };
      else if (BUILTIN.has(obj.text)) target = { kind: "builtin", id: `${obj.text}.${m}` };
      else target = { kind: "method", id: m };
    } else target = { kind: "method", id: m };
  }
  const args = [], inner = [];
  (node.arguments || []).forEach(a => {
    if (isFn(a)) {
      const body = walkFn(a, ctx);
      if (meaningful(body)) inner.push(iterName
        ? block("loop", "loop", `each item of .${iterName}()`, body, L(a))
        : block("callback", "callback", `runs inside ${label}()`, body, L(a)));
    } else if (ts.isIdentifier(a) && ctx.scope.locals[a.text] && !ctx.inline.has(a.text)) {
      ctx.used.add(a.text);
      const body = walkFn(ctx.scope.locals[a.text], { ...ctx, inline: new Set([...ctx.inline, a.text]) });
      inner.push(block("callback", "callback", `${a.text}() runs inside ${label}()`, body, L(ctx.scope.locals[a.text])));
    } else args.push(...walk(a, { ...ctx, argOf: label }));
  });
  if (target.kind === "local" && !ctx.inline.has(target.id)) {
    ctx.used.add(target.id);
    const fnNode = ctx.scope.locals[target.id];
    const body = walkFn(fnNode, { ...ctx, inline: new Set([...ctx.inline, target.id]) });
    if (meaningful(body)) inner.unshift(block("inline", "body", `${target.id}()`, body, L(fnNode)));
  }
  if (target.kind === "param" && ctx.params[target.id] && isFn(ctx.params[target.id])) {
    const d = ctx.params[target.id];
    const body = walkFn(d, ctx);
    if (meaningful(body)) inner.push(block("inline", "default", `${target.id} = ${T(d, 48)} (unless the caller passes one)`, body, L(d)));
  }
  const p = node.parent;
  const item = {
    t: "call", label, target, line: L(node),
    await: !!p && ts.isAwaitExpression(p),
    optional: !!node.questionDotToken,
    argOf: ctx.argOf || null,
    inner,
  };
  return [...pre, ...args, item];
}

function walk(node, ctx) {
  if (!node) return [];
  if (ts.isBlock(node)) return node.statements.flatMap(s => walk(s, ctx));
  if (ts.isVariableStatement(node)) return walk(node.declarationList, ctx);
  if (ts.isVariableDeclarationList(node)) return node.declarations.flatMap(d => {
    if (isFn(d.initializer) && ts.isIdentifier(d.name)) return [{ t: "def", name: d.name.text, line: L(d), _node: d.initializer }];
    return walk(d.initializer, ctx);
  });
  if (ts.isExpressionStatement(node)) return walk(node.expression, ctx);
  if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node) || ts.isSatisfiesExpression?.(node)) return walk(node.expression, ctx);
  if (ts.isIfStatement(node)) {
    const cond = walk(node.expression, { ...ctx, argOf: "(condition)" });
    const inner = { ...ctx, cond: ctx.cond + 1 };
    const a = walk(node.thenStatement, inner);
    const b = node.elseStatement ? walk(node.elseStatement, inner) : [];
    const lbl = `(${T(node.expression, 72)})`;
    if (!node.elseStatement) return [...cond, ...(meaningful(a) ? [block("cond", "if", lbl, a, L(node))] : [])];
    return [...cond, ...branches("cond", lbl, a, b, L(node), L(node.elseStatement))];
  }
  if (ts.isConditionalExpression(node)) {
    const cond = walk(node.condition, { ...ctx, argOf: "(condition)" });
    const inner = { ...ctx, cond: ctx.cond + 1 };
    const a = walk(node.whenTrue, inner), b = walk(node.whenFalse, inner);
    const lbl = `(${T(node.condition, 72)})`;
    if (!meaningful(a) && !meaningful(b)) return cond;
    if (!meaningful(b)) return [...cond, block("cond", "if", lbl, a, L(node))];
    if (!meaningful(a)) return [...cond, block("cond", "unless", lbl, b, L(node.whenFalse))];
    return [...cond, ...branches("cond", lbl, a, b, L(node), L(node.whenFalse))];
  }
  if (ts.isBinaryExpression(node)) {
    const op = node.operatorToken.kind, K = ts.SyntaxKind;
    if (op === K.AmpersandAmpersandToken || op === K.BarBarToken || op === K.QuestionQuestionToken) {
      const left = walk(node.left, ctx);
      const right = walk(node.right, { ...ctx, cond: ctx.cond + 1 });
      if (!meaningful(right)) return left;
      const kw = op === K.AmpersandAmpersandToken ? "only if" : op === K.BarBarToken ? "unless" : "if nullish";
      return [...left, block("cond", kw, T(node.left, 72), right, L(node))];
    }
    return [...walk(node.left, ctx), ...walk(node.right, ctx)];
  }
  if (ts.isReturnStatement(node)) {
    const e = node.expression;
    if (e && ts.isObjectLiteralExpression(e) && ctx.factory) {
      const methods = Object.entries(factoryProps[ctx.factory] || {}).filter(([, v]) => v.unit || v.from).map(([k, v]) => ({ name: k, unit: v.unit || null, from: v.from || null }));
      const pre = e.properties.flatMap(p => ts.isPropertyAssignment(p) && !isFn(p.initializer) ? walk(p.initializer, ctx) : ts.isSpreadAssignment(p) ? walk(p.expression, ctx) : []);
      return [...pre, { t: "exit", kind: "return", label: "controller object", methods, line: L(node), early: ctx.cond > 0 }];
    }
    return [...(e ? walk(e, ctx) : []), { t: "exit", kind: "return", label: e ? T(e, 46) : "", line: L(node), early: ctx.cond > 0 }];
  }
  if (ts.isThrowStatement(node)) return [...walk(node.expression, ctx), { t: "exit", kind: "throw", label: T(node.expression, 46), line: L(node) }];
  if (ts.isTryStatement(node)) {
    const out = [block("try", "try", "", walk(node.tryBlock, ctx), L(node))];
    if (node.catchClause) {
      const v = node.catchClause.variableDeclaration;
      out.push(block("catch", "catch", v ? `(${v.name.getText(sf)})` : "", walk(node.catchClause.block, { ...ctx, cond: ctx.cond + 1 }), L(node.catchClause)));
    }
    if (node.finallyBlock) out.push(block("finally", "finally", "always runs", walk(node.finallyBlock, ctx), L(node.finallyBlock)));
    return out.filter(b => meaningful(b.children) || b.kind === "try");
  }
  if (ts.isForStatement(node) || ts.isWhileStatement(node) || ts.isDoStatement(node)) {
    const head = ts.isForStatement(node) ? `for (${T(node.initializer || node, 20)}; ${node.condition ? T(node.condition, 30) : ""}; …)` : `while (${T(node.expression, 50)})`;
    const pre = ts.isForStatement(node) ? walk(node.initializer, ctx) : [];
    const body = [...(ts.isForStatement(node) ? walk(node.condition, ctx) : walk(node.expression, ctx)), ...walk(node.statement, { ...ctx, cond: ctx.cond + 1 }), ...(ts.isForStatement(node) ? walk(node.incrementor, ctx) : [])];
    return [...pre, ...(meaningful(body) ? [block("loop", "loop", head, body, L(node))] : [])];
  }
  if (ts.isForOfStatement(node) || ts.isForInStatement(node)) {
    const pre = walk(node.expression, ctx);
    const body = walk(node.statement, { ...ctx, cond: ctx.cond + 1 });
    return [...pre, ...(meaningful(body) ? [block("loop", "loop", `for (${T(node.initializer, 24)} ${ts.isForOfStatement(node) ? "of" : "in"} ${T(node.expression, 30)})`, body, L(node))] : [])];
  }
  if (ts.isSwitchStatement(node)) {
    const pre = walk(node.expression, ctx);
    const cases = node.caseBlock.clauses.map(cl => block("cond", ts.isCaseClause(cl) ? "case" : "default", ts.isCaseClause(cl) ? T(cl.expression, 40) : "", cl.statements.flatMap(s => walk(s, { ...ctx, cond: ctx.cond + 1 })), L(cl)));
    return [...pre, ...cases.filter(c => meaningful(c.children))];
  }
  if (ts.isCallExpression(node)) return callItem(node, ctx, false);
  if (ts.isNewExpression(node)) return callItem(node, ctx, true);
  if (isFn(node)) {
    const body = walkFn(node, ctx);
    return meaningful(body) ? [block("deferred", "later", "inline function, runs when invoked", body, L(node))] : [];
  }
  if (ts.isObjectLiteralExpression(node)) return node.properties.flatMap(p => {
    if (ts.isPropertyAssignment(p)) return walk(p.initializer, ctx);
    if (ts.isSpreadAssignment(p)) return walk(p.expression, ctx);
    if (ts.isMethodDeclaration(p)) { const b = walkFn(p, ctx); return meaningful(b) ? [block("deferred", "later", `method ${T(p.name, 30)}()`, b, L(p))] : []; }
    return [];
  });
  const out = [];
  ts.forEachChild(node, ch => { out.push(...walk(ch, ctx)); });
  return out;
}

function finalize(items, ctx, returnedNames) {
  const out = [];
  for (const it of items) {
    if (it.t === "def") {
      if (ctx.used.has(it.name) || returnedNames.has(it.name)) continue;
      const body = finalize(walkFn(it._node, { ...ctx, inline: new Set([...ctx.inline, it.name]) }), ctx, returnedNames);
      if (meaningful(body)) out.push(block("deferred", "later", `${it.name}() defined here, runs when invoked`, body, it.line));
      continue;
    }
    if (it.t === "block") it.children = finalize(it.children, ctx, returnedNames);
    if (it.t === "call") it.inner = finalize(it.inner, ctx, returnedNames);
    out.push(it);
  }
  return out;
}

const result = {};
for (const u of Object.values(units)) {
  const ctx = { scope: u.scope, params: u.params, argOf: null, cond: 0, inline: new Set(u.method ? [u.method] : []), used: new Set(), factory: u.method ? null : (factoryProps[u.id] ? u.id : null) };
  const raw = walkFn(u.node, ctx);
  const returned = new Set(u.method ? [] : Object.keys(factoryProps[u.id] || {}));
  const tree = finalize(raw, ctx, returned);
  const params = (u.node.parameters || []).map(p => p.name.getText(sf) + (p.initializer ? " = …" : ""));
  result[u.id] = { id: u.id, owner: u.owner, method: u.method || null, start: u.start, end: u.end, async: u.async, params, tree };
}
process.stdout.write(JSON.stringify(result));
