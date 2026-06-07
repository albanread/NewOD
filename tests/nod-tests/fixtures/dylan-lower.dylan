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
// A terminator (ir.rs Terminator). kind ∈ {"return","if","jump"}:
//   return: value = <integer> temp or #f.
//   if:     value = cond temp; a = then-label; b = else-label.
//   jump:   a = target-label; args = <stretchy-vector> of temp ids.
// Held as a separate object so <dfm-block>'s `make` stays within the 8-keyword
// limit and every slot is supplied (avoiding the slot-default GAP).
define class <dfm-term> (<object>)
  slot term-kind  :: <byte-string>, init-keyword: kind:;
  slot term-value :: <object>,      init-keyword: value:;
  slot term-a     :: <object>,      init-keyword: a:;
  slot term-b     :: <object>,      init-keyword: b:;
  slot term-args  :: <object>,      init-keyword: args:;
end class;

define function make-return-term (value :: <object>) => (t :: <dfm-term>)
  make(<dfm-term>, kind: "return", value: value, a: #f, b: #f, args: #f)
end function;

define class <dfm-block> (<object>)
  slot block-id     :: <integer>,         init-keyword: id:;
  slot block-label  :: <byte-string>,     init-keyword: label:;
  slot block-params :: <stretchy-vector>, init-keyword: params:;
  slot block-comps  :: <stretchy-vector>, init-keyword: comps:;
  slot block-term   :: <dfm-term>,        init-keyword: term:;
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
                   term: make-return-term(#f));
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
  block-term(blk) := make-return-term(value);
end function;

// Function::temp_type — rendered type label of a temp id (Top fallback).
define function fb-temp-type (b :: <fn-builder>, id :: <integer>)
 => (ty :: <byte-string>)
  temp-type-of(func-temps(fb-func(b)), id)
end function;

// new_block — allocate the next block id, append a block labelled
// `<prefix><id>` (matching the Rust new_block labels: "then1", "else2",
// "join3"), default Return{None} terminator. Returns the block's index in
// func-blocks (== its id, since blocks are appended in id order).
define function fb-new-block (b :: <fn-builder>, prefix :: <byte-string>)
 => (index :: <integer>)
  let id = fb-next-block(b);
  fb-next-block(b) := id + 1;
  let blk = make(<dfm-block>,
                 id: id, label: concatenate(prefix, integer-to-string(id)),
                 params: make(<stretchy-vector>),
                 comps:  make(<stretchy-vector>),
                 term: make-return-term(#f));
  let blocks = func-blocks(fb-func(b));
  let index = size(blocks);
  add!(blocks, blk);
  index
end function;

// switch_to — make `index` the current block.
define function fb-switch-to (b :: <fn-builder>, index :: <integer>) => ()
  fb-current(b) := index;
end function;

// Block label by index.
define function fb-block-label (b :: <fn-builder>, index :: <integer>)
 => (label :: <byte-string>)
  block-label(func-blocks(fb-func(b))[index])
end function;

// add_block_param — append a fresh temp (typed `ty`) as a parameter of block
// `index`; returns the temp id (the merged value at a join).
define function fb-add-block-param (b :: <fn-builder>, index :: <integer>,
                                    ty :: <byte-string>) => (temp :: <integer>)
  let t = fb-fresh-temp(b, ty);
  add!(block-params(func-blocks(fb-func(b))[index]), t);
  t
end function;

// terminate the current block with `If <cnd> then-label else-label`.
define function fb-terminate-if (b :: <fn-builder>, cnd :: <integer>,
                                 then-lbl :: <byte-string>, else-lbl :: <byte-string>) => ()
  let blk = func-blocks(fb-func(b))[fb-current(b)];
  block-term(blk) := make(<dfm-term>, kind: "if", value: cnd,
                          a: then-lbl, b: else-lbl, args: #f);
end function;

// terminate the current block with `Jump target(args…)`.
define function fb-terminate-jump (b :: <fn-builder>, target :: <byte-string>,
                                   args :: <stretchy-vector>) => ()
  let blk = func-blocks(fb-func(b))[fb-current(b)];
  block-term(blk) := make(<dfm-term>, kind: "jump", value: #f,
                          a: target, b: #f, args: args);
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

define function singleton-vec (a :: <integer>) => (v :: <stretchy-vector>)
  let v = make(<stretchy-vector>);
  add!(v, a);
  v
end function;

// Shallow copy of a stretchy-vector (for env snapshot / restore at a merge).
define function copy-vec (v :: <stretchy-vector>) => (c :: <stretchy-vector>)
  let c = make(<stretchy-vector>);
  let n = size(v);
  let i = 0;
  until (i >= n)
    add!(c, v[i]);
    i := i + 1;
  end;
  c
end function;

// Is a type label GC-typed (needs a GC root / block-param threading across a
// merge)? Immediate-scalar values (fixnum / boolean / character) are NOT;
// everything else (strings, classes, floats(boxed), Top) conservatively is.
// Used to gate `if`: env-merge threading of GC-typed bindings is a later 55a
// step, so an `if` whose enclosing env holds a GC-typed binding bails to Rust.
define function gc-typed-label? (label :: <byte-string>) => (yes? :: <boolean>)
  ~ (label = "<integer>" | label = "<boolean>" | label = "<character>")
end function;

// Lattice join of two type labels for a merge param (TypeEstimate::join):
// equal → that type; otherwise → Top. (Two distinct user classes both render
// "<class>" via name(), so this is approximate for classes — a 55b concern;
// no class values flow through `if` yet.)
define function join-type-label (a :: <byte-string>, b :: <byte-string>)
 => (label :: <byte-string>)
  if (a = b) a else "<top>" end
end function;

// Const Bool(false) — the value of an `if` with no `else` arm.
define function emit-false-const (b :: <fn-builder>) => (temp :: <integer>)
  let t = fb-fresh-temp(b, "<boolean>");
  fb-push(b, make(<dfm-comp>, kind: "const", dst: t, cval: "Bool(false)",
                  op: #f, args: make(<stretchy-vector>), callee: #f));
  t
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
    let raw = lit-value(node);
    let cval = concatenate("String(\"", concatenate(escape-string-debug(raw), "\")"));
    fb-push(b, make(<dfm-comp>, kind: "const", dst: t, cval: cval,
                    op: #f, args: make(<stretchy-vector>), callee: #f));
    t
  elseif (instance?(node, <ast-binary-op>))
    let op-text = token-source-text(binop-operator(node), source);
    if (op-text = ":=")
      // `lhs := rhs` — plain-local SSA rebind (lower_assign). Lower the RHS and,
      // if the LHS is a simple env-bound name, rebind name->rhs-temp; the
      // assignment value IS the rhs temp and NO computation is emitted for the
      // assignment itself. A non-simple / unbound LHS is outside scope -> #f.
      lower-assign(b, node, ret-map, source)
    elseif (op-text = "|" | op-text = "&")
      // `|` / `&` short-circuit — a diamond, NOT a PrimOp (lower_short_circuit).
      lower-short-circuit(b, node, op-text, ret-map, source)
    else
      // Strict binop. Operands lower left-then-right — this ORDER fixes the
      // operand temp ids.
      let l = lower-expr(b, binop-left(node), ret-map, source);
      let r = lower-expr(b, binop-right(node), ret-map, source);
      if (~ l | ~ r)
        #f
      else
        let lt = fb-temp-type(b, l);
        let rt = fb-temp-type(b, r);
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
  elseif (instance?(node, <ast-statement>))
    // Control-flow statements in expression position. 55a: `if`, `while`,
    // `until`. Others (begin / case / method-literal) are later → #f.
    let word = token-source-text(stmt-word(node), source);
    if (word = "if")
      lower-if-expr(b, node, ret-map, source)
    elseif (word = "while")
      lower-loop(b, node, #f, ret-map, source)
    elseif (word = "until")
      lower-loop(b, node, #t, ret-map, source)
    else
      #f
    end
  else
    // Outside the current subset (unary, floats, chars, symbols,
    // make/instance?/%-prims, begin/loops, …): later → #f.
    #f
  end
end function;

// ─── lower-let — mirrors a Statement::Let with a single binder ─────────────
//
// `let binder = init` (an <ast-local-decl> whose ldecl-list is the binder
// binop). Lowers the init expression and binds the binder name to its temp
// (a non-captured let in Rust lowering is just a name->value-temp binding — no
// extra computation; cell promotion for captured lets is 55c). Returns the
// init temp, or #f if outside the Phase-0/55a subset (multi-binder destructure,
// `let x` with no init, or an unsupported init).
define function lower-let (b :: <fn-builder>, decl :: <ast-local-decl>,
                           ret-map :: <name-ret-map>, source :: <byte-string>)
 => (temp :: <object>)
  let list = ldecl-list(decl);
  let cs = body-constituents(list);
  if (size(cs) ~= 1)
    #f                                  // `let (a, b) = …` multi-binder — 55a+
  else
    let node = cs[0];
    if (~ instance?(node, <ast-binary-op>))
      #f                                // `let x` with no initialiser — bail
    else
      let lhs = binop-left(node);
      let name =
        if (instance?(lhs, <ast-variable-ref>))
          token-source-text(varref-tok(lhs), source)
        elseif (instance?(lhs, <ast-typed-name>))
          token-source-text(typed-name-tok(lhs), source)
        else
          #f
        end;
      if (~ name)
        #f
      else
        let t = lower-expr(b, binop-right(node), ret-map, source);
        if (~ t)
          #f
        else
          fb-bind(b, name, t);
          t
        end
      end
    end
  end
end function;

// Lower one body constituent (a `let` decl or an expression). Returns its
// value temp, or #f if unsupported.
define function lower-body-stmt (b :: <fn-builder>, item :: <object>,
                                 ret-map :: <name-ret-map>, source :: <byte-string>)
 => (temp :: <object>)
  if (instance?(item, <ast-local-decl>))
    lower-let(b, item, ret-map, source)
  else
    lower-expr(b, item, ret-map, source)
  end
end function;

// Lower a range of body constituents [start, end) in order; the last *value* is
// returned. #f if any is unsupported, or the range is empty.
//
// A body statement's result is classified (lower_function_inner's last_temp
// update): #f → bail; an <integer> temp → a value (updates `last`); anything
// else truthy (the void marker `#t` from a loop, which produces no value) is a
// void statement — it is lowered for effect but does NOT update `last`.
define function lower-stmt-range (b :: <fn-builder>, cs :: <stretchy-vector>,
                                  start :: <integer>, ret-map :: <name-ret-map>,
                                  source :: <byte-string>)
 => (temp :: <object>)
  let n = size(cs);
  let i = start;
  let last = #f;
  let ok? = #t;
  until (i >= n | ~ ok?)
    let t = lower-body-stmt(b, cs[i], ret-map, source);
    if (~ t)
      ok? := #f;
    elseif (instance?(t, <integer>))
      last := t;
    end;
    i := i + 1;
  end;
  if (~ ok?) #f else last end
end function;

// Does the current env hold any GC-typed binding? (See gc-typed-label?.)
define function env-has-gc-typed? (b :: <fn-builder>) => (yes? :: <boolean>)
  let temps = fb-env-temps(b);
  let n = size(temps);
  let i = 0;
  let found = #f;
  until (i >= n | found)
    if (gc-typed-label?(fb-temp-type(b, temps[i]))) found := #t; end;
    i := i + 1;
  end;
  found
end function;

// ─── lower-short-circuit — mirrors lower_short_circuit (`|` / `&`) ──────────
//
// `a | b` / `a & b` lower to an sc_edge / sc_rhs / sc_join diamond. The LHS is
// evaluated in the current block; on the short-circuit outcome control jumps to
// sc_edge carrying the LHS value, otherwise to sc_rhs which evaluates the RHS
// and jumps with its value; sc_join's block-param is the result.
//   `|`: LHS true  → sc_edge (value = LHS); false → sc_rhs.  (If lhs edge rhs)
//   `&`: LHS true  → sc_rhs;  false → sc_edge (value = LHS). (If lhs rhs edge)
// Same env-merge guard as `if` (bail to Rust on GC-typed env).
define function lower-short-circuit (b :: <fn-builder>, node :: <ast-binary-op>,
                                     op :: <byte-string>, ret-map :: <name-ret-map>,
                                     source :: <byte-string>)
 => (temp :: <object>)
  if (env-has-gc-typed?(b))
    #f
  else
    let lhs = lower-expr(b, binop-left(node), ret-map, source);
    if (~ lhs)
      #f
    else
      let lhs-ty = fb-temp-type(b, lhs);
      let edge-idx = fb-new-block(b, "sc_edge");
      let rhs-idx = fb-new-block(b, "sc_rhs");
      let join-idx = fb-new-block(b, "sc_join");
      let edge-lbl = fb-block-label(b, edge-idx);
      let rhs-lbl = fb-block-label(b, rhs-idx);
      let join-lbl = fb-block-label(b, join-idx);
      if (op = "|")
        fb-terminate-if(b, lhs, edge-lbl, rhs-lbl);
      else
        fb-terminate-if(b, lhs, rhs-lbl, edge-lbl);
      end;
      // sc_edge: short-circuit — carry the LHS value.
      fb-switch-to(b, edge-idx);
      fb-terminate-jump(b, join-lbl, singleton-vec(lhs));
      // sc_rhs: evaluate the RHS.
      fb-switch-to(b, rhs-idx);
      let rhs = lower-expr(b, binop-right(node), ret-map, source);
      if (~ rhs)
        #f
      else
        let rhs-ty = fb-temp-type(b, rhs);
        fb-terminate-jump(b, join-lbl, singleton-vec(rhs));
        fb-switch-to(b, join-idx);
        fb-add-block-param(b, join-idx, join-type-label(lhs-ty, rhs-ty))
      end
    end
  end
end function;

// ─── lower-if-expr — mirrors lower_if (the value-merge, non-mutating case) ──
//
// `if (cond) then-body [else else-body] end` → a 3-block diamond
// (then/else/join) with the merged value as the single join block-param — the
// shape Rust's lower_if produces when no arm assigns a variable and the
// enclosing env holds no GC-typed binding (so nothing else threads through the
// join). Block ids/labels and temp ids reproduce the Rust emission order:
// cond temps (entry) → then-body temps → else-body temps → join param.
//
// Bails (#f, → Rust path) on: any GC-typed env binding (env-merge threading is
// a later 55a step), `elseif` chains, or any unsupported arm expression
// (e.g. an arm that assigns — `:=` isn't lowered yet, so it bails naturally).
define function lower-if-expr (b :: <fn-builder>, stmt :: <ast-statement>,
                               ret-map :: <name-ret-map>, source :: <byte-string>)
 => (temp :: <object>)
  let scs = body-constituents(stmt-body(stmt));
  if (size(scs) < 1)
    #f
  else
    // Resolve the else arm: no clauses → none; one `else` clause → its body;
    // elseif / multiple → bail.
    let clauses = stmt-clauses(stmt);
    let else-cs = #f;
    let bail? = #f;
    if (instance?(clauses, <stretchy-vector>))
      if (size(clauses) = 1)
        let cl = clauses[0];
        if (token-source-text(clause-word(cl), source) = "else")
          else-cs := body-constituents(clause-body(cl));
        else
          bail? := #t;
        end;
      elseif (size(clauses) > 1)
        bail? := #t;
      end;
    end;
    if (bail?)
      #f
    else
      // Merge set = vars assigned in either arm ∪ GC-typed env names, sorted.
      // That order = join-param order = jump-arg order, value param FIRST.
      let merge = make(<stretchy-vector>);
      let ti = 1;
      let tn = size(scs);
      until (ti >= tn)
        collect-assigned(b, scs[ti], source, merge);
        ti := ti + 1;
      end;
      if (instance?(else-cs, <stretchy-vector>))
        let ei = 0;
        let en = size(else-cs);
        until (ei >= en)
          collect-assigned(b, else-cs[ei], source, merge);
          ei := ei + 1;
        end;
      end;
      let enames = fb-env-names(b);
      let etemps = fb-env-temps(b);
      let ne = size(enames);
      let gi = 0;
      until (gi >= ne)
        if (gc-typed-label?(fb-temp-type(b, etemps[gi])))
          set-add!(merge, enames[gi]);
        end;
        gi := gi + 1;
      end;
      lower-sort-strings!(merge);
      let nm = size(merge);
      // Condition (lowered in the current block).
      let cnd = lower-expr(b, scs[0], ret-map, source);
      if (~ cnd)
        #f
      else
        let then-idx = fb-new-block(b, "then");
        let else-idx = fb-new-block(b, "else");
        fb-terminate-if(b, cnd, fb-block-label(b, then-idx), fb-block-label(b, else-idx));
        // Snapshot env so the else arm starts from the pre-if bindings.
        let snap-names = copy-vec(fb-env-names(b));
        let snap-temps = copy-vec(fb-env-temps(b));
        // then arm
        fb-switch-to(b, then-idx);
        let then-val = lower-stmt-range(b, scs, 1, ret-map, source);
        if (~ then-val)
          #f
        else
          let then-ty = fb-temp-type(b, then-val);
          let then-end = fb-current(b);            // arm may have branched
          let then-merge = make(<stretchy-vector>);
          let mi = 0;
          until (mi >= nm) add!(then-merge, fb-lookup(b, merge[mi])); mi := mi + 1; end;
          // Restore env for the else arm.
          fb-env-names(b) := copy-vec(snap-names);
          fb-env-temps(b) := copy-vec(snap-temps);
          fb-switch-to(b, else-idx);
          let else-val =
            if (instance?(else-cs, <stretchy-vector>))
              lower-stmt-range(b, else-cs, 0, ret-map, source)
            else
              emit-false-const(b)
            end;
          if (~ else-val)
            #f
          else
            let else-ty = fb-temp-type(b, else-val);
            let else-end = fb-current(b);
            let else-merge = make(<stretchy-vector>);
            let mj = 0;
            until (mj >= nm) add!(else-merge, fb-lookup(b, merge[mj])); mj := mj + 1; end;
            // Join created AFTER both arms (GAP-010): its id follows any blocks
            // a nested-control-flow arm created.
            let join-idx = fb-new-block(b, "join");
            let join-lbl = fb-block-label(b, join-idx);
            // then-end → join(then-val, then-merge…)
            let then-args = make(<stretchy-vector>);
            add!(then-args, then-val);
            let ai = 0;
            until (ai >= nm) add!(then-args, then-merge[ai]); ai := ai + 1; end;
            fb-switch-to(b, then-end);
            fb-terminate-jump(b, join-lbl, then-args);
            // else-end → join(else-val, else-merge…)
            let else-args = make(<stretchy-vector>);
            add!(else-args, else-val);
            let aj = 0;
            until (aj >= nm) add!(else-args, else-merge[aj]); aj := aj + 1; end;
            fb-switch-to(b, else-end);
            fb-terminate-jump(b, join-lbl, else-args);
            // Join params: VALUE first, then merge vars (sorted). Then rebind
            // env to pre-if + the merge vars' new join params.
            fb-switch-to(b, join-idx);
            let value-param =
              fb-add-block-param(b, join-idx, join-type-label(then-ty, else-ty));
            fb-env-names(b) := copy-vec(snap-names);
            fb-env-temps(b) := copy-vec(snap-temps);
            let pk = 0;
            until (pk >= nm)
              let pty = join-type-label(fb-temp-type(b, then-merge[pk]),
                                        fb-temp-type(b, else-merge[pk]));
              fb-bind(b, merge[pk], fb-add-block-param(b, join-idx, pty));
              pk := pk + 1;
            end;
            value-param
          end
        end
      end
    end
  end
end function;

// ─── lower-assign — mirrors lower_assign (plain-local SSA-rebind case) ──────
//
// `lhs := rhs`. Lower the RHS to a temp; if the LHS is a bare name currently
// bound in env (a plain local / param), REBIND name->rhs-temp and return the
// rhs temp — emitting NO computation for the assignment itself (an SSA rebind
// is just an env update; the value of `:=` is the RHS). A non-simple LHS, or a
// name not in env (module variable / cell-promoted local — later sprints),
// bails to the Rust path (#f). The 55a subset has no GC-typed locals, so the
// cell/closure/module-variable branches of lower_assign never apply here.
define function lower-assign (b :: <fn-builder>, node :: <ast-binary-op>,
                              ret-map :: <name-ret-map>, source :: <byte-string>)
 => (temp :: <object>)
  let lhs = binop-left(node);
  if (~ instance?(lhs, <ast-variable-ref>))
    #f
  else
    let name = token-source-text(varref-tok(lhs), source);
    if (~ fb-lookup(b, name))
      #f                                  // unbound name — module var / later
    else
      let t = lower-expr(b, binop-right(node), ret-map, source);
      if (~ t)
        #f
      else
        fb-bind(b, name, t);              // SSA rebind; most-recent wins
        t
      end
    end
  end
end function;

// ── string sort (local; mirrors bs-le? / sort-strings! in dylan-sema.dylan) ──
// Byte-wise lexical compare a <= b. Shorter-but-equal-prefix sorts first.
define function lower-bs-le? (a :: <byte-string>, b :: <byte-string>)
 => (yes? :: <boolean>)
  let na = size(a);
  let nb = size(b);
  let n = if (na < nb) na else nb end;
  let i = 0;
  let result = #f;
  let decided = #f;
  until (i >= n | decided)
    let ca = %byte-string-element(a, i);
    let cb = %byte-string-element(b, i);
    if (ca < cb)      result := #t; decided := #t;
    elseif (ca > cb)  result := #f; decided := #t;
    end;
    i := i + 1;
  end;
  if (decided) result else na <= nb end
end function;

// In-place insertion sort of a stretchy-vector of <byte-string> (ascending).
define function lower-sort-strings! (v :: <stretchy-vector>) => ()
  let n = size(v);
  let i = 1;
  until (i >= n)
    let x = v[i];
    let j = i;
    until (j = 0 | lower-bs-le?(v[j - 1], x))
      v[j] := v[j - 1];
      j := j - 1;
    end;
    v[j] := x;
    i := i + 1;
  end;
end function;

// Add `name` to set-vector `v` if not already present (HashSet::insert).
define function set-add! (v :: <stretchy-vector>, name :: <byte-string>) => ()
  let n = size(v);
  let i = 0;
  let found = #f;
  until (i >= n | found)
    if (v[i] = name) found := #t; end;
    i := i + 1;
  end;
  if (~ found) add!(v, name); end;
end function;

// ── carried-set walks — mirror collect_used_bound_names_* / collect_assigned_*
//
// Both walk the loop's cond + body collecting env-bound names into a set-vector
// `out`. They recurse over binops, calls (callee + positional-arg values),
// control statements (if/while/until: stmt-body constituents + clause bodies),
// and nested `let` initialisers — the node shapes that appear in the 55a subset.

// Names that are READ (and currently bound in env). An <ast-variable-ref> whose
// name is in env is a use. (`x := …` also reaches here via the binop LHS, which
// is harmless: the assigned name is carried regardless.)
define function collect-used (b :: <fn-builder>, node :: <object>,
                              source :: <byte-string>, out :: <stretchy-vector>) => ()
  if (instance?(node, <ast-variable-ref>))
    let name = token-source-text(varref-tok(node), source);
    if (fb-lookup(b, name)) set-add!(out, name); end;
  elseif (instance?(node, <ast-binary-op>))
    collect-used(b, binop-left(node), source, out);
    collect-used(b, binop-right(node), source, out);
  elseif (instance?(node, <ast-call>))
    collect-used(b, call-fn(node), source, out);
    let args = call-args(node);
    let n = size(args);
    let i = 0;
    until (i >= n)
      let an = args[i];
      let av = if (instance?(an, <ast-pos-arg>)) pos-arg-value(an) else an end;
      collect-used(b, av, source, out);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-local-decl>))
    collect-used-in-body(b, ldecl-list(node), source, out);
  elseif (instance?(node, <ast-statement>))
    collect-used-in-body(b, stmt-body(node), source, out);
    let clauses = stmt-clauses(node);
    if (instance?(clauses, <stretchy-vector>))
      let n = size(clauses);
      let i = 0;
      until (i >= n)
        collect-used-in-body(b, clause-body(clauses[i]), source, out);
        i := i + 1;
      end;
    end;
  end;
end function;

define function collect-used-in-body (b :: <fn-builder>, body :: <object>,
                                      source :: <byte-string>, out :: <stretchy-vector>) => ()
  let cs = body-constituents(body);
  let n = size(cs);
  let i = 0;
  until (i >= n)
    collect-used(b, cs[i], source, out);
    i := i + 1;
  end;
end function;

// Names ASSIGNED via `:=` to a bound env name (collect_assigned_in_*). For an
// assignment binop the LHS env-name is added and only the RHS is recursed;
// other binops recurse both sides. `let` shadowing a bound outer name marks
// that name assigned (Sprint 18 rule), matching the Rust binder-shadow arm.
define function collect-assigned (b :: <fn-builder>, node :: <object>,
                                  source :: <byte-string>, out :: <stretchy-vector>) => ()
  if (instance?(node, <ast-binary-op>))
    let op-text = token-source-text(binop-operator(node), source);
    if (op-text = ":=")
      let lhs = binop-left(node);
      if (instance?(lhs, <ast-variable-ref>))
        let name = token-source-text(varref-tok(lhs), source);
        if (fb-lookup(b, name)) set-add!(out, name); end;
      end;
      collect-assigned(b, binop-right(node), source, out);
    else
      collect-assigned(b, binop-left(node), source, out);
      collect-assigned(b, binop-right(node), source, out);
    end;
  elseif (instance?(node, <ast-call>))
    collect-assigned(b, call-fn(node), source, out);
    let args = call-args(node);
    let n = size(args);
    let i = 0;
    until (i >= n)
      let an = args[i];
      let av = if (instance?(an, <ast-pos-arg>)) pos-arg-value(an) else an end;
      collect-assigned(b, av, source, out);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-local-decl>))
    collect-assigned-in-body(b, ldecl-list(node), source, out);
  elseif (instance?(node, <ast-statement>))
    collect-assigned-in-body(b, stmt-body(node), source, out);
    let clauses = stmt-clauses(node);
    if (instance?(clauses, <stretchy-vector>))
      let n = size(clauses);
      let i = 0;
      until (i >= n)
        collect-assigned-in-body(b, clause-body(clauses[i]), source, out);
        i := i + 1;
      end;
    end;
  end;
end function;

define function collect-assigned-in-body (b :: <fn-builder>, body :: <object>,
                                          source :: <byte-string>, out :: <stretchy-vector>) => ()
  let cs = body-constituents(body);
  let n = size(cs);
  let i = 0;
  until (i >= n)
    collect-assigned(b, cs[i], source, out);
    i := i + 1;
  end;
end function;

// ─── lower-loop — mirrors lower_while_like (while + until) ──────────────────
//
// `while (cond) body… end` / `until (cond) body… end`. stmt-body = [cond,
// body…]. Builds the loop_header / loop_body / loop_exit CFG with the carried
// (phi) set threaded through the header block-params.
//
//   loop_header(phi…):  cond_t = <cond>;  If cond_t <then> <else>
//      while: then=body  else=exit ;  until: then=exit  else=body
//      (ONLY the branch labels swap — the cond primop is NOT negated.)
//   loop_body:          <body stmts>;  Jump loop_header(carried env temps…)
//   loop_exit:          continue (the loop's value is void).
//
// Carried set = names assigned via `:=` in the body, OR used in cond/body, OR
// GC-typed in env — sorted lexically. That single order governs header-param
// order, the entry-jump args, and the back-edge args. Returns the void marker
// (#t) on success, or #f if any sub-lowering bails (-> Rust path).
//
// Block creation order is load-bearing for the byte-match: header FIRST (id H);
// header params consume temp ids BEFORE the cond is lowered; body/exit are
// created AFTER lowering the cond (so any sc_* blocks from a short-circuit cond
// precede them — GAP-009).
define function lower-loop (b :: <fn-builder>, stmt :: <ast-statement>,
                            invert? :: <object>, ret-map :: <name-ret-map>,
                            source :: <byte-string>)
 => (temp :: <object>)
  let scs = body-constituents(stmt-body(stmt));
  if (size(scs) < 1)
    #f                                   // no condition — malformed
  else
    let cond-node = scs[0];
    // (1) loop_header FIRST (id H).
    let header-idx = fb-new-block(b, "loop_header");
    // (2) Carried set: assigned ∪ used ∪ GC-typed env names, then sort.
    let carried = make(<stretchy-vector>);
    collect-assigned-in-body(b, stmt-body(stmt), source, carried);  // cond + body
    let used = make(<stretchy-vector>);
    collect-used(b, cond-node, source, used);
    let bi = 1;
    let nb = size(scs);
    until (bi >= nb)
      collect-used(b, scs[bi], source, used);
      bi := bi + 1;
    end;
    // Add GC-typed env names + used names to the carried set (assigned already
    // in). (No 55a fixture has GC-typed locals, but mirror the Rust rule.)
    let enames = fb-env-names(b);
    let etemps = fb-env-temps(b);
    let ne = size(enames);
    let ei = 0;
    until (ei >= ne)
      if (gc-typed-label?(fb-temp-type(b, etemps[ei])))
        set-add!(carried, enames[ei]);
      end;
      ei := ei + 1;
    end;
    let nu = size(used);
    let ui = 0;
    until (ui >= nu)
      // Only carry names still bound in env (used is already env-filtered).
      set-add!(carried, used[ui]);
      ui := ui + 1;
    end;
    lower-sort-strings!(carried);
    // (3) Capture each carried name's CURRENT (pre-loop) env temp in sorted
    // order, and add a header block-param per carried name (consuming temp ids
    // BEFORE the cond is lowered). Bail if any carried name is somehow unbound.
    let nc = size(carried);
    let pre-temps = make(<stretchy-vector>);
    let phis = make(<stretchy-vector>);
    let ok? = #t;
    let ci = 0;
    until (ci >= nc | ~ ok?)
      let outer = fb-lookup(b, carried[ci]);
      if (~ outer)
        ok? := #f;
      else
        add!(pre-temps, outer);
        let phi = fb-add-block-param(b, header-idx, fb-temp-type(b, outer));
        add!(phis, phi);
      end;
      ci := ci + 1;
    end;
    if (~ ok?)
      #f
    else
      let header-lbl = fb-block-label(b, header-idx);
      // (4) Entry-side jump → header with pre-loop temps (sorted order).
      fb-terminate-jump(b, header-lbl, pre-temps);
      // (5) Rebind env name->phi so header/body read the loop phis.
      let ri = 0;
      until (ri >= nc)
        fb-bind(b, carried[ri], phis[ri]);
        ri := ri + 1;
      end;
      // (6) header: lower cond, then create body/exit, then branch.
      fb-switch-to(b, header-idx);
      let cond-t = lower-expr(b, cond-node, ret-map, source);
      if (~ cond-t)
        #f
      else
        let body-idx = fb-new-block(b, "loop_body");
        let exit-idx = fb-new-block(b, "loop_exit");
        let body-lbl = fb-block-label(b, body-idx);
        let exit-lbl = fb-block-label(b, exit-idx);
        if (invert?)
          fb-terminate-if(b, cond-t, exit-lbl, body-lbl);   // until polarity
        else
          fb-terminate-if(b, cond-t, body-lbl, exit-lbl);   // while polarity
        end;
        // (7) loop_body: lower body stmts (`:=` rebinds env), then back-edge.
        fb-switch-to(b, body-idx);
        let body-ok? = #t;
        let si = 1;
        until (si >= nb | ~ body-ok?)
          let t = lower-body-stmt(b, scs[si], ret-map, source);
          if (~ t) body-ok? := #f; end;   // <integer>/void both fine (discarded)
          si := si + 1;
        end;
        if (~ body-ok?)
          #f
        else
          // Back-edge args: env[name] for each carried name, in sorted order.
          let back-args = make(<stretchy-vector>);
          let qi = 0;
          until (qi >= nc)
            add!(back-args, fb-lookup(b, carried[qi]));
            qi := qi + 1;
          end;
          fb-terminate-jump(b, header-lbl, back-args);
          // (8) Restore env name->phi (post-loop reads see the header phi),
          // then continue at exit. The loop's own value is void.
          let xi = 0;
          until (xi >= nc)
            fb-bind(b, carried[xi], phis[xi]);
            xi := xi + 1;
          end;
          fb-switch-to(b, exit-idx);
          #t                              // void marker — loop produces no value
        end
      end
    end
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
    // (2) Body — a sequence of straight-line statements (let bindings +
    // expressions). Each lowers in order; the LAST statement's value is the
    // return value (lower_function_inner's last_temp). Any unsupported
    // statement bails the whole function (-> Rust path).
    let body = defn-body(defn);
    let cs = body-constituents(body);
    let nc = size(cs);
    let ci = 0;
    let last-temp = #f;
    let ok? = #t;
    until (ci >= nc | ~ ok?)
      let t = lower-body-stmt(b, cs[ci], ret-map, source);
      // #f → bail; <integer> temp → the running return value; void marker (a
      // loop) → lowered for effect, does NOT become the return value (so
      // `until(...)...end; result` returns `result`, not the loop).
      if (~ t)
        ok? := #f;
      elseif (instance?(t, <integer>))
        last-temp := t;
      end;
      ci := ci + 1;
    end;
    if (~ ok? | ~ last-temp)
      #f
    else
      // (3) return_type: declared wins, else the final temp's type.
      let declared = defn-declared-return-label(defn, source);
      let ret-label = if (declared) declared else fb-temp-type(b, last-temp) end;
      func-return-type(fb-func(b)) := ret-label;
      // (4) Return{value}.
      fb-terminate-return(b, last-temp);
      fb-func(b)
    end
  end
end function;

// ─── format-dfm — mirrors nod-dfm/src/format.rs EXACTLY ────────────────────

// Render one byte as a 1-char <byte-string>.
define function byte-to-string-1 (c :: <integer>) => (s :: <byte-string>)
  let s = %byte-string-allocate(1);
  %byte-string-element-setter(c, s, 0);
  s
end function;

// Lowercase hex of a byte value (no leading zero), for `\u{..}` escapes.
define function byte-hex (c :: <integer>) => (s :: <byte-string>)
  let digits = "0123456789abcdef";
  let hi = c - (c / 16) * 16;        // low nibble
  let lo-s = byte-to-string-1(%byte-string-element(digits, hi));
  let high = c / 16;
  if (high = 0)
    lo-s
  else
    concatenate(byte-to-string-1(%byte-string-element(digits, high)), lo-s)
  end
end function;

// Escape a string the way Rust's `{:?}` (str Debug / escape_debug) does, so
// `String(<...>)` in the DFM dump matches `format.rs` byte-for-byte: `"` and
// `\` are backslash-escaped, `\n` / `\t` / `\r` use their letter escapes,
// printable ASCII passes through, and any other byte becomes `\u{<hex>}`.
define function escape-string-debug (s :: <byte-string>) => (out :: <byte-string>)
  let out = "";
  let n = size(s);
  let i = 0;
  until (i >= n)
    let c = %byte-string-element(s, i);
    let piece =
      if (c = 34)                   "\\\""        // "
      elseif (c = 92)               "\\\\"        // backslash
      elseif (c = 10)               "\\n"
      elseif (c = 9)                "\\t"
      elseif (c = 13)               "\\r"
      elseif (c >= 32 & c <= 126)   byte-to-string-1(c)
      else                          concatenate("\\u{", concatenate(byte-hex(c), "}"))
      end;
    out := concatenate(out, piece);
    i := i + 1;
  end;
  out
end function;

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

// fmt_terminator (format.rs): Return / If / Jump.
define function fmt-terminator (blk :: <dfm-block>) => (s :: <byte-string>)
  let tm = block-term(blk);
  let kind = term-kind(tm);
  if (kind = "return")
    let v = term-value(tm);
    if (v)
      concatenate("    Return t", concatenate(integer-to-string(v), "\n"))
    else
      "    Return\n"
    end
  elseif (kind = "if")
    // `    If t<cond> <then-label> <else-label>`
    concatenate("    If t",
      concatenate(integer-to-string(term-value(tm)),
        concatenate(" ", concatenate(term-a(tm),
          concatenate(" ", concatenate(term-b(tm), "\n"))))))
  else
    // `    Jump <target>(t.., t..)`
    let line = concatenate("    Jump ", concatenate(term-a(tm), "("));
    let args = term-args(tm);
    let m = size(args);
    let j = 0;
    until (j >= m)
      if (j > 0) line := concatenate(line, ", "); end;
      line := concatenate(line, concatenate("t", integer-to-string(args[j])));
      j := j + 1;
    end;
    concatenate(line, ")\n")
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
