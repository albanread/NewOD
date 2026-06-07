Module: dylan-lexer
Precedence: c

// Sprint 55 Phase-0 — Dylan-side AST->DFM lowering (straight-line subset).
//
// Ports the SIMPLEST slice of the Rust lowering (src/nod-sema/src/lower.rs) to
// Dylan and reproduces the `dump-dfm` text byte-for-byte
// (src/nod-dfm/src/format.rs). Phase 0 handles ONLY:
//   * integer / boolean / string literals
//   * binary ops (arith + comparisons), integer/Top operands
//   * direct calls to known top-level names
//   * functions whose body is one straight-line expression ending in Return
//
// It emits ONLY Const / PrimOp / DirectCall computations and a Return
// terminator, always with empty safepoint roots and never `is_no_alloc` — the
// Phase-0 surface from docs/journal/2026-06-07-sprint-55-lowering-plan.md.
//
// The whole game is the byte-match, and the byte-match is the EMISSION ORDER:
// temp ids and block ids are monotonic counters, so reproducing the exact
// order fresh-temp / new-block fire reproduces the dump. Mirrored Rust line
// refs are cited inline (lower.rs / format.rs / ir.rs).
//
// Bundled in `Module: dylan-lexer` alongside dylan-lexer/parser/sema, so it
// freely calls lex, parse-dylan-with-precedence, the <ast-*> accessors,
// token-source-text, integer-to-string, etc.
//
// Per the slot-default GAP (see dylan-parser.dylan): every slot carries an
// explicit init-keyword and is supplied at `make` time; flags are <object>.

// ─── DFM IR — Dylan mirrors of nod-dfm/src/ir.rs (Phase-0 subset) ──────────

// A temporary (ir.rs Temporary). We store the rendered type label directly
// ("<integer>" etc.), since the dump only needs TypeEstimate::name().
define class <dfm-temp> (<object>)
  slot temp-id   :: <integer>,     init-keyword: id:;
  slot temp-type :: <byte-string>, init-keyword: type:;
end class;

// One computation (ir.rs Computation). Phase 0 builds only const / primop /
// directcall, so a single tagged record is simpler than a class hierarchy.
define class <dfm-comp> (<object>)
  slot comp-kind   :: <byte-string>,     init-keyword: kind:;
  slot comp-dst    :: <integer>,         init-keyword: dst:;
  slot comp-cval   :: <object>,          init-keyword: cval:;    // "Integer(5)" or #f
  slot comp-op     :: <object>,          init-keyword: op:;      // "AddInt" or #f
  slot comp-args   :: <stretchy-vector>, init-keyword: args:;    // <integer> temp ids
  slot comp-callee :: <object>,          init-keyword: callee:;  // name or #f
end class;

// A block (ir.rs Block). Phase 0 makes exactly the entry block. Terminator is
// inlined: block-term-kind is "return"; block-term-value is the <integer> temp
// id, or #f for a bare `Return`.
define class <dfm-block> (<object>)
  slot block-id         :: <integer>,         init-keyword: id:;
  slot block-label      :: <byte-string>,     init-keyword: label:;
  slot block-params     :: <stretchy-vector>, init-keyword: params:;
  slot block-comps      :: <stretchy-vector>, init-keyword: comps:;
  slot block-term-kind  :: <byte-string>,     init-keyword: term-kind:;
  slot block-term-value :: <object>,          init-keyword: term-value:;
end class;

// A function (ir.rs Function). func-temps is the master temp list (so we can
// answer "type of temp N" cheaply, mirroring Function::temp_type).
define class <dfm-func> (<object>)
  slot func-name        :: <byte-string>,     init-keyword: name:;
  slot func-params      :: <stretchy-vector>, init-keyword: params:;
  slot func-blocks      :: <stretchy-vector>, init-keyword: blocks:;
  slot func-temps       :: <stretchy-vector>, init-keyword: temps:;
  slot func-return-type :: <byte-string>,     init-keyword: return-type:;
end class;

// ─── FunctionBuilder — mirrors lower.rs FunctionBuilder ────────────────────

define class <fn-builder> (<object>)
  slot fb-func       :: <dfm-func>, init-keyword: func:;
  slot fb-current    :: <integer>,  init-keyword: current:;
  slot fb-next-temp  :: <integer>,  init-keyword: next-temp:;
  slot fb-next-block :: <integer>,  init-keyword: next-block:;
  slot fb-last-temp  :: <object>,   init-keyword: last-temp:;
  // LocalEnv (lower.rs LocalEnv = name -> TempId), as parallel vectors: the
  // bindings visible in the current scope (Phase 0: just the params).
  slot fb-env-names  :: <stretchy-vector>, init-keyword: env-names:;
  slot fb-env-temps  :: <stretchy-vector>, init-keyword: env-temps:;
end class;

// FunctionBuilder::new — entry = BlockId(0) "entry", Return{None}, next_temp=0,
// next_block=1, current=entry.
define function make-fn-builder (name :: <byte-string>) => (b :: <fn-builder>)
  let entry = make(<dfm-block>,
                   id: 0, label: "entry",
                   params: make(<stretchy-vector>),
                   comps:  make(<stretchy-vector>),
                   term-kind: "return", term-value: #f);
  let blocks = make(<stretchy-vector>);
  add!(blocks, entry);
  let func = make(<dfm-func>,
                  name: name,
                  params: make(<stretchy-vector>),
                  blocks: blocks,
                  temps:  make(<stretchy-vector>),
                  return-type: "<unit>");
  make(<fn-builder>,
       func: func, current: 0, next-temp: 0, next-block: 1, last-temp: #f,
       env-names: make(<stretchy-vector>), env-temps: make(<stretchy-vector>))
end function;

// LocalEnv bind / lookup. `fb-lookup` returns the bound temp id (most-recent
// binding wins, scanning back to front) or #f if the name isn't bound.
define function fb-bind (b :: <fn-builder>, name :: <byte-string>, temp :: <integer>) => ()
  add!(fb-env-names(b), name);
  add!(fb-env-temps(b), temp);
end function;

define function fb-lookup (b :: <fn-builder>, name :: <byte-string>)
 => (temp :: <object>)
  let names = fb-env-names(b);
  let temps = fb-env-temps(b);
  let i = size(names) - 1;
  let found = #f;
  until (i < 0 | found)
    if (names[i] = name) found := temps[i]; end;
    i := i - 1;
  end;
  found
end function;

// fresh_temp — allocate the next temp id, record its type, return id.
define function fb-fresh-temp (b :: <fn-builder>, ty :: <byte-string>)
 => (id :: <integer>)
  let id = fb-next-temp(b);
  fb-next-temp(b) := id + 1;
  add!(func-temps(fb-func(b)), make(<dfm-temp>, id: id, type: ty));
  id
end function;

// push — append a computation to the current block.
define function fb-push (b :: <fn-builder>, c :: <dfm-comp>) => ()
  let blk = func-blocks(fb-func(b))[fb-current(b)];
  add!(block-comps(blk), c);
end function;

// terminate_current — set the current block's Return terminator (value is an
// <integer> temp id, or #f for bare Return).
define function fb-terminate-return (b :: <fn-builder>, value :: <object>) => ()
  let blk = func-blocks(fb-func(b))[fb-current(b)];
  block-term-kind(blk) := "return";
  block-term-value(blk) := value;
end function;

// Function::temp_type — rendered type label of a temp id (Top fallback).
define function fb-temp-type (b :: <fn-builder>, id :: <integer>)
 => (ty :: <byte-string>)
  temp-type-of(func-temps(fb-func(b)), id)
end function;

// Shared temp-type lookup over a temp list.
define function temp-type-of (temps :: <stretchy-vector>, id :: <integer>)
 => (ty :: <byte-string>)
  let n = size(temps);
  let i = 0;
  let found = #f;
  until (i >= n | found)
    if (temp-id(temps[i]) = id) found := temp-type(temps[i]); end;
    i := i + 1;
  end;
  if (found) found else "<top>" end
end function;

// ── small helpers ──

define function pair-args (a :: <integer>, b :: <integer>)
 => (v :: <stretchy-vector>)
  let v = make(<stretchy-vector>);
  add!(v, a);
  add!(v, b);
  v
end function;

// ─── Type mapping — mirrors type_from_expr (lower.rs) for scalar cases ─────

define function type-name-to-label (type-name :: <byte-string>)
 => (label :: <byte-string>)
  if (type-name = "<integer>")            "<integer>"
  elseif (type-name = "<single-float>")   "<single-float>"
  elseif (type-name = "<double-float>")   "<double-float>"
  elseif (type-name = "<float>")          "<double-float>"
  elseif (type-name = "<boolean>")        "<boolean>"
  elseif (type-name = "<character>")      "<character>"
  elseif (type-name = "<string>")         "<string>"
  elseif (type-name = "<byte-string>")    "<string>"
  else                                    "<top>"
  end
end function;

// ─── Top-name return-type map (mirrors TopNames::return_type) ───────────────

define class <name-ret-map> (<object>)
  slot nrm-names  :: <stretchy-vector>, init-keyword: names:;
  slot nrm-labels :: <stretchy-vector>, init-keyword: labels:;
end class;

define function nrm-lookup (m :: <name-ret-map>, name :: <byte-string>)
 => (label :: <byte-string>)
  let names = nrm-names(m);
  let n = size(names);
  let i = 0;
  let found = #f;
  until (i >= n | found)
    if (names[i] = name) found := nrm-labels(m)[i]; end;
    i := i + 1;
  end;
  if (found) found else "<top>" end
end function;

// Declared return label of a `define function`, or #f if none.
define function defn-declared-return-label (defn :: <ast-body-definition>,
                                            source :: <byte-string>)
 => (label :: <object>)
  let rspec = defn-return(defn);
  if (~ rspec)
    #f
  else
    let vals = ret-values(rspec);
    if (size(vals) = 0)
      #f
    else
      let tn = vals[0];
      let ty = typed-name-type(tn);
      let type-name =
        if (ty)
          if (instance?(ty, <ast-variable-ref>))
            token-source-text(varref-tok(ty), source)
          else
            ""
          end
        else
          token-source-text(typed-name-tok(tn), source)
        end;
      type-name-to-label(type-name)
    end
  end
end function;

// Build name -> declared-return-label map over top-level `define function`s.
define function build-name-ret-map (items :: <stretchy-vector>,
                                    source :: <byte-string>)
 => (m :: <name-ret-map>)
  let names  = make(<stretchy-vector>);
  let labels = make(<stretchy-vector>);
  let n = size(items);
  let i = 0;
  until (i >= n)
    let item = items[i];
    if (instance?(item, <ast-body-definition>))
      let word = token-source-text(defn-word(item), source);
      if (word = "function")
        let name-tok = defn-method-name(item);
        if (name-tok)
          let name = token-source-text(name-tok, source);
          let lbl  = defn-declared-return-label(item, source);
          add!(names, name);
          add!(labels, if (lbl) lbl else "<top>" end);
        end;
      end;
    end;
    i := i + 1;
  end;
  make(<name-ret-map>, names: names, labels: labels)
end function;

// ─── select_binop — mirrors lower.rs select_binop (Phase-0 int / Top) ───────

define function select-binop (op-text :: <byte-string>,
                              lt :: <byte-string>, rt :: <byte-string>)
 => (prim :: <object>)
  let int-ok? = (lt = "<integer>" | lt = "<top>") & (rt = "<integer>" | rt = "<top>");
  if (~ int-ok?)            #f
  elseif (op-text = "+")    "AddInt"
  elseif (op-text = "-")    "SubInt"
  elseif (op-text = "*")    "MulInt"
  elseif (op-text = "/")    "DivInt"
  elseif (op-text = "mod")  "ModInt"
  elseif (op-text = "rem")  "RemInt"
  elseif (op-text = "=")    "EqInt"
  elseif (op-text = "==")   "EqInt"
  elseif (op-text = "~=")   "NeInt"
  elseif (op-text = "~==")  "NeInt"
  elseif (op-text = "<")    "LtInt"
  elseif (op-text = ">")    "GtInt"
  elseif (op-text = "<=")   "LeInt"
  elseif (op-text = ">=")   "GeInt"
  else                      #f
  end
end function;

// PrimOp::result_type label: arith -> <integer>, comparison -> <boolean>.
define function primop-result-label (prim :: <byte-string>)
 => (label :: <byte-string>)
  if (prim = "AddInt" | prim = "SubInt" | prim = "MulInt"
        | prim = "DivInt" | prim = "ModInt" | prim = "RemInt")
    "<integer>"
  else
    "<boolean>"
  end
end function;

// ─── lower-expr — mirrors lower.rs lower_expr (Phase-0 subset) ──────────────
//
// Lowers one expression node into computations on `b`, returning its result
// temp id (an <integer>), or #f if the node is outside Phase-0 scope (the
// caller treats #f as "fixture not yet Dylan-lowerable").

define function lower-expr (b :: <fn-builder>, node :: <object>,
                            ret-map :: <name-ret-map>, source :: <byte-string>)
 => (temp :: <object>)
  if (instance?(node, <ast-variable-ref>))
    // A bare name: Phase 0 only resolves params / locals in the env (lower.rs
    // lower_expr Ident → local-env read). A name not in the env (stdlib
    // constant, class ref, bare function-ref) is outside Phase 0 → #f.
    fb-lookup(b, token-source-text(varref-tok(node), source))
  elseif (instance?(node, <ast-integer-lit>))
    let v = lit-value(node);
    let t = fb-fresh-temp(b, "<integer>");
    let cval = concatenate("Integer(", concatenate(integer-to-string(v), ")"));
    fb-push(b, make(<dfm-comp>, kind: "const", dst: t, cval: cval,
                    op: #f, args: make(<stretchy-vector>), callee: #f));
    t
  elseif (instance?(node, <ast-boolean-lit>))
    let t = fb-fresh-temp(b, "<boolean>");
    let cval = if (lit-value(node)) "Bool(true)" else "Bool(false)" end;
    fb-push(b, make(<dfm-comp>, kind: "const", dst: t, cval: cval,
                    op: #f, args: make(<stretchy-vector>), callee: #f));
    t
  elseif (instance?(node, <ast-string-lit>))
    let t = fb-fresh-temp(b, "<string>");
    // NOTE: only ASCII strings with no quote/backslash/control chars match
    // Rust `{:?}` here (Phase-0 corpus has none — see plan risks).
    let raw = lit-value(node);
    let cval = concatenate("String(\"", concatenate(raw, "\")"));
    fb-push(b, make(<dfm-comp>, kind: "const", dst: t, cval: cval,
                    op: #f, args: make(<stretchy-vector>), callee: #f));
    t
  elseif (instance?(node, <ast-binary-op>))
    // Operands lower left-then-right — this ORDER fixes the operand temp ids.
    let l = lower-expr(b, binop-left(node), ret-map, source);
    let r = lower-expr(b, binop-right(node), ret-map, source);
    if (~ l | ~ r)
      #f
    else
      let lt = fb-temp-type(b, l);
      let rt = fb-temp-type(b, r);
      let op-text = token-source-text(binop-operator(node), source);
      let prim = select-binop(op-text, lt, rt);
      if (~ prim)
        #f
      else
        let dst = fb-fresh-temp(b, primop-result-label(prim));
        fb-push(b, make(<dfm-comp>, kind: "primop", dst: dst, cval: #f,
                        op: prim, args: pair-args(l, r), callee: #f));
        dst
      end
    end
  elseif (instance?(node, <ast-call>))
    let callee-node = call-fn(node);
    if (~ instance?(callee-node, <ast-variable-ref>))
      #f
    else
      let name = token-source-text(varref-tok(callee-node), source);
      // Args lower left-to-right BEFORE the dst is minted (dst id comes after
      // all arg ids, matching lower.rs fresh_temp(ret) ordering).
      let arg-nodes = call-args(node);
      let n = size(arg-nodes);
      let arg-temps = make(<stretchy-vector>);
      let i = 0;
      let ok? = #t;
      until (i >= n | ~ ok?)
        let an = arg-nodes[i];
        let av = if (instance?(an, <ast-pos-arg>)) pos-arg-value(an) else an end;
        let at = lower-expr(b, av, ret-map, source);
        if (~ at) ok? := #f; else add!(arg-temps, at); end;
        i := i + 1;
      end;
      if (~ ok?)
        #f
      else
        let ret-label = nrm-lookup(ret-map, name);
        let dst = fb-fresh-temp(b, ret-label);
        fb-push(b, make(<dfm-comp>, kind: "directcall", dst: dst, cval: #f,
                        op: #f, args: arg-temps, callee: name));
        dst
      end
    end
  else
    // Outside Phase-0 scope (local var refs, if/while, unary, floats, chars,
    // symbols, make/instance?/%-prims, …): 55a+ territory.
    #f
  end
end function;

// ─── lower-function — mirrors lower_function_inner (straight-line case) ─────
//
// Builds a <dfm-func> for one `define function` whose body is a single
// straight-line expression. Returns the <dfm-func>, or #f if outside scope.
// Order mirrored from lower.rs: params get fresh temps in declaration order
// (t0,t1,…) BEFORE the body; the body's single expression's temp is the Return
// value; return_type = declared label if present, else the final temp's type.

define function lower-function (defn :: <ast-body-definition>,
                                ret-map :: <name-ret-map>, source :: <byte-string>)
 => (func :: <object>)
  let name-tok = defn-method-name(defn);
  if (~ name-tok)
    #f
  else
    let name = token-source-text(name-tok, source);
    let b = make-fn-builder(name);
    // (1) Parameters -> entry temps, declaration order.
    let params = defn-params(defn);
    if (params)
      let reqs = params-required(params);
      let np = size(reqs);
      let pi = 0;
      until (pi >= np)
        let tn = reqs[pi];
        let ty = typed-name-type(tn);
        let type-name =
          if (ty & instance?(ty, <ast-variable-ref>))
            token-source-text(varref-tok(ty), source)
          else
            ""
          end;
        let t = fb-fresh-temp(b, type-name-to-label(type-name));
        add!(func-params(fb-func(b)), t);
        // Bind the param name so body var-refs resolve to its temp.
        fb-bind(b, token-source-text(typed-name-tok(tn), source), t);
        pi := pi + 1;
      end;
    end;
    // (2) Body — single straight-line expression (Phase-0 restriction).
    let body = defn-body(defn);
    let cs = body-constituents(body);
    if (size(cs) ~= 1)
      #f
    else
      let final-temp = lower-expr(b, cs[0], ret-map, source);
      if (~ final-temp)
        #f
      else
        // (3) return_type: declared wins, else the final temp's type.
        let declared = defn-declared-return-label(defn, source);
        let ret-label = if (declared) declared else fb-temp-type(b, final-temp) end;
        func-return-type(fb-func(b)) := ret-label;
        // (4) Return{value}.
        fb-terminate-return(b, final-temp);
        fb-func(b)
      end
    end
  end
end function;

// ─── format-dfm — mirrors nod-dfm/src/format.rs EXACTLY ────────────────────

// fmt_computation (format.rs), Phase-0 kinds. 4-space indent, newline-end.
define function fmt-computation (c :: <dfm-comp>, temps :: <stretchy-vector>)
 => (s :: <byte-string>)
  let kind = comp-kind(c);
  let dst-ty = temp-type-of(temps, comp-dst(c));
  let head = concatenate("    t",
               concatenate(integer-to-string(comp-dst(c)),
                 concatenate(": ", dst-ty)));
  if (kind = "const")
    concatenate(head, concatenate(" = Const ", concatenate(comp-cval(c), "\n")))
  elseif (kind = "primop")
    let line = concatenate(head, concatenate(" = PrimOp ", comp-op(c)));
    let args = comp-args(c);
    let n = size(args);
    let i = 0;
    until (i >= n)
      line := concatenate(line, concatenate(" t", integer-to-string(args[i])));
      i := i + 1;
    end;
    concatenate(line, "\n")
  else
    // directcall: ` = DirectCall callee(t0, t1)`; empty safepoint + not
    // no_alloc -> nothing appended.
    let line = concatenate(head,
                 concatenate(" = DirectCall ", concatenate(comp-callee(c), "(")));
    let args = comp-args(c);
    let n = size(args);
    let i = 0;
    until (i >= n)
      if (i > 0) line := concatenate(line, ", "); end;
      line := concatenate(line, concatenate("t", integer-to-string(args[i])));
      i := i + 1;
    end;
    concatenate(line, ")\n")
  end
end function;

// fmt_terminator (format.rs), Phase-0 = Return only.
define function fmt-terminator (blk :: <dfm-block>) => (s :: <byte-string>)
  let v = block-term-value(blk);
  if (v)
    concatenate("    Return t", concatenate(integer-to-string(v), "\n"))
  else
    "    Return\n"
  end
end function;

// fmt_function (format.rs).
define function fmt-function (f :: <dfm-func>) => (s :: <byte-string>)
  let temps  = func-temps(f);
  // Header: `fn <name> (t0: <type>, …) -> <ret>:`
  let out = concatenate("fn ", concatenate(func-name(f), " ("));
  let params = func-params(f);
  let np = size(params);
  let pi = 0;
  until (pi >= np)
    if (pi > 0) out := concatenate(out, ", "); end;
    let pid = params[pi];
    out := concatenate(out,
             concatenate("t", concatenate(integer-to-string(pid),
               concatenate(": ", temp-type-of(temps, pid)))));
    pi := pi + 1;
  end;
  out := concatenate(out,
           concatenate(") -> ", concatenate(func-return-type(f), ":\n")));
  // Blocks.
  let blocks = func-blocks(f);
  let nb = size(blocks);
  let bi = 0;
  until (bi >= nb)
    let blk = blocks[bi];
    out := concatenate(out, concatenate("  ", block-label(blk)));
    let bparams = block-params(blk);
    let nbp = size(bparams);
    if (nbp > 0)
      out := concatenate(out, "(");
      let bpi = 0;
      until (bpi >= nbp)
        if (bpi > 0) out := concatenate(out, ", "); end;
        let bpid = bparams[bpi];
        out := concatenate(out,
                 concatenate("t", concatenate(integer-to-string(bpid),
                   concatenate(": ", temp-type-of(temps, bpid)))));
        bpi := bpi + 1;
      end;
      out := concatenate(out, ")");
    end;
    out := concatenate(out, ":\n");
    let comps = block-comps(blk);
    let nc = size(comps);
    let ci = 0;
    until (ci >= nc)
      out := concatenate(out, fmt-computation(comps[ci], temps));
      ci := ci + 1;
    end;
    out := concatenate(out, fmt-terminator(blk));
    bi := bi + 1;
  end;
  out
end function;

// format_dfm_module (format.rs): functions joined by a '\n' separator (each
// function block already ends with '\n', so this yields a blank line between).
define function format-dfm-module (funcs :: <stretchy-vector>)
 => (s :: <byte-string>)
  let out = "";
  let n = size(funcs);
  let i = 0;
  until (i >= n)
    if (i > 0) out := concatenate(out, "\n"); end;
    out := concatenate(out, fmt-function(funcs[i]));
    i := i + 1;
  end;
  out
end function;

// ─── Top-level entry — lex -> parse -> lower -> format ─────────────────────
//
// Returns the dump-dfm text, or "" if ANY top-level item is outside Phase-0
// scope (so the gate keeps that fixture on the Rust path — Phase 0 must never
// emit a WRONG dump).

define function dylan-lower-emit (source :: <byte-string>)
 => (dfm-text :: <byte-string>)
  let tokens = lex(source);
  let ast    = parse-dylan-with-precedence(tokens, precedence-c-header?(source));
  let items  = body-constituents(ast);
  let ret-map = build-name-ret-map(items, source);
  let funcs  = make(<stretchy-vector>);
  let n = size(items);
  let i = 0;
  let all-ok? = #t;
  until (i >= n | ~ all-ok?)
    let item = items[i];
    if (instance?(item, <ast-body-definition>))
      let word = token-source-text(defn-word(item), source);
      if (word = "function")
        let f = lower-function(item, ret-map, source);
        if (f) add!(funcs, f); else all-ok? := #f; end;
      else
        all-ok? := #f;     // `define method` — outside Phase 0
      end;
    elseif (instance?(item, <ast-list-definition>)
              | instance?(item, <ast-class-definition>)
              | instance?(item, <ast-generic-definition>))
      all-ok? := #f;       // constant / variable / class / generic — 55a/55b
    else
      // Preamble (`Module:` / `Precedence:` lexed as ordinary forms) or a bare
      // top-level expression. The Dylan parser keeps the preamble as items
      // (the host translator strips it via scan_preamble); skip such items,
      // mirroring `collect-top-names`. No Phase-0 fixture has a bare top-level
      // expression, so skipping is safe here.
      #f;
    end;
    i := i + 1;
  end;
  if (all-ok?) format-dfm-module(funcs) else "" end
end function;
