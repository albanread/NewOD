Module: dylan-lexer
Precedence: c

// Sprint 53.2 — Dylan-side sema recording walk: the top-level name table.
//
// Walks the parsed AST (the `<ast-*>` tree from dylan-parser.dylan) and
// emits the `=== top-names ===` section of the sema model, byte-matching
// the Rust `format_sema_model` for class-free fixtures: top-level
// `define function`/`method` names with arity + return-type estimate,
// and `define constant`/`variable` names. Auto-generated slot-accessor
// names come from class processing (Sprint 53.3); this covers user
// definitions only.

// ─── return-type estimate mapping (must match TypeEstimate Debug names) ──

define function map-type-estimate (type-name :: <byte-string>) => (est :: <byte-string>)
  if (type-name = "<integer>")           "Integer"
  elseif (type-name = "<single-float>")  "SingleFloat"
  elseif (type-name = "<double-float>")  "DoubleFloat"
  elseif (type-name = "<character>")     "Character"
  elseif (type-name = "<boolean>")       "Boolean"
  elseif (type-name = "<byte-string>")   "String"
  elseif (type-name = "<string>")        "String"
  else                                   "Top"
  end
end function;

// Extract a type expression's name token text (a bare `<integer>` return
// type is stored AS the typed-name's token; `x :: <integer>` puts the
// type in a variable-ref node).
define function type-node-name (node :: <object>, source :: <byte-string>)
 => (name :: <byte-string>)
  if (instance?(node, <ast-variable-ref>))
    token-source-text(varref-tok(node), source)
  else
    ""
  end
end function;

// The return-type estimate for a body-definition.
define function defn-return-estimate (defn :: <ast-body-definition>,
                                      source :: <byte-string>)
 => (est :: <byte-string>)
  let rspec = defn-return(defn);
  if (~ rspec)
    "Top"
  else
    let vals = ret-values(rspec);
    if (size(vals) = 0)
      "Top"
    else
      let tn = vals[0];                  // <ast-typed-name>
      let ty = typed-name-type(tn);
      let type-name =
        if (ty)
          type-node-name(ty, source)     // `x :: <type>` form
        else
          token-source-text(typed-name-tok(tn), source)   // bare `<type>`
        end;
      map-type-estimate(type-name)
    end
  end
end function;

// Required-parameter count = arity.
define function defn-arity (defn :: <ast-body-definition>) => (n :: <integer>)
  let params = defn-params(defn);
  if (params)
    size(params-required(params))
  else
    0
  end
end function;

// ─── a sortable top-level function entry ─────────────────────────────────

define class <top-fn> (<object>)
  slot top-fn-name  :: <byte-string>, init-keyword: name:;
  slot top-fn-line  :: <byte-string>, init-keyword: line:;
end class;

// Lexicographic `a <= b` on byte-strings (byte-wise; the runtime doesn't
// guarantee `<=` on <byte-string>).
define function bs-le? (a :: <byte-string>, b :: <byte-string>) => (yes? :: <boolean>)
  let na = size(a);
  let nb = size(b);
  let m = if (na < nb) na else nb end;
  let i = 0;
  let result = #f;
  let decided = #f;
  until (i = m | decided)
    let ca = %byte-string-element(a, i);
    let cb = %byte-string-element(b, i);
    if (ca < cb)      result := #t; decided := #t;
    elseif (ca > cb)  result := #f; decided := #t;
    else              i := i + 1;
    end;
  end;
  if (decided) result else na <= nb end
end function;

// Insertion-sort a vector of <byte-string> by value (ascending).
define function sort-strings! (v :: <stretchy-vector>) => ()
  let n = size(v);
  let i = 1;
  // `i` starts at 1, so guard with `>=` not `=`: an empty vector (n = 0)
  // would otherwise step straight past n and index v[1] out of bounds
  // (factorial.dylan has no constants/variables, hitting exactly this).
  until (i >= n)
    let x = v[i];
    let j = i;
    until (j = 0 | bs-le?(v[j - 1], x))
      v[j] := v[j - 1];
      j := j - 1;
    end;
    v[j] := x;
    i := i + 1;
  end;
end function;

// Insertion-sort <top-fn> entries by name.
define function sort-fns! (v :: <stretchy-vector>) => ()
  let n = size(v);
  let i = 1;
  // See sort-strings!: guard with `>=` so an empty vector (n = 0) is a no-op
  // instead of indexing v[1] out of bounds.
  until (i >= n)
    let x = v[i];
    let j = i;
    until (j = 0 | bs-le?(top-fn-name(v[j - 1]), top-fn-name(x)))
      v[j] := v[j - 1];
      j := j - 1;
    end;
    v[j] := x;
    i := i + 1;
  end;
end function;

// Best-effort name of a `define constant`/`variable` binding: the
// left-hand binder of the first constituent (`name = init`, or a bare
// `name`). Refined when the corpus needs more shapes.
define function list-defn-name (defn :: <ast-list-definition>,
                                source :: <byte-string>) => (name :: <byte-string>)
  let lst = defn-list(defn);
  let cs = body-constituents(lst);
  if (size(cs) = 0)
    ""
  else
    let first = cs[0];
    if (instance?(first, <ast-binary-op>))
      let lhs = binop-left(first);
      if (instance?(lhs, <ast-variable-ref>))
        token-source-text(varref-tok(lhs), source)
      elseif (instance?(lhs, <ast-typed-name>))
        token-source-text(typed-name-tok(lhs), source)
      else
        ""
      end
    elseif (instance?(first, <ast-variable-ref>))
      token-source-text(varref-tok(first), source)
    elseif (instance?(first, <ast-typed-name>))
      token-source-text(typed-name-tok(first), source)
    else
      ""
    end
  end
end function;

// ─── the walk ────────────────────────────────────────────────────────────

define function collect-top-names (ast :: <ast-body>, source :: <byte-string>)
 => (text :: <byte-string>)
  let fns    = make(<stretchy-vector>);
  let consts = make(<stretchy-vector>);
  let vars   = make(<stretchy-vector>);
  let items  = body-constituents(ast);
  let n = size(items);
  let i = 0;
  until (i = n)
    let item = items[i];
    if (instance?(item, <ast-body-definition>))
      let word = token-source-text(defn-word(item), source);
      if (word = "function" | word = "method")
        let name-tok = defn-method-name(item);
        if (name-tok)
          let name  = token-source-text(name-tok, source);
          let arity = defn-arity(item);
          let est   = defn-return-estimate(item, source);
          let line  = concatenate("fn ", concatenate(name,
                        concatenate(" arity=", concatenate(integer-to-string(arity),
                          concatenate(" return=", est)))));
          add!(fns, make(<top-fn>, name: name, line: line));
        end;
      end;
    elseif (instance?(item, <ast-list-definition>))
      let word = token-source-text(defn-word(item), source);
      let name = list-defn-name(item, source);
      if (word = "constant")    add!(consts, name);
      elseif (word = "variable") add!(vars, name);
      end;
    end;
    i := i + 1;
  end;

  sort-fns!(fns);
  sort-strings!(consts);
  sort-strings!(vars);

  let out = "=== top-names ===\n";
  let fi = 0;
  until (fi = size(fns))
    out := concatenate(out, concatenate(top-fn-line(fns[fi]), "\n"));
    fi := fi + 1;
  end;
  let ci = 0;
  until (ci = size(consts))
    out := concatenate(out, concatenate("constant ", concatenate(consts[ci], "\n")));
    ci := ci + 1;
  end;
  let vi = 0;
  until (vi = size(vars))
    out := concatenate(out, concatenate("variable ", concatenate(vars[vi], "\n")));
    vi := vi + 1;
  end;
  out
end function;

// ─── driver entry ──────────────────────────────────────────────────────

define function sema-main () => ()
  let path = %argv1();
  if (empty?(path))
    format-out("dylan-sema: missing input path\n");
  else
    let source = load-source-via-rope(path);
    if (empty?(source))
      format-out("dylan-sema: could not read %s\n", path);
    else
      let tokens = lex(source);
      // parse-dylan uses the default (flat, DRM) precedence — correct for
      // headerless fixtures. `Precedence: c` files would need the shim's
      // precedence-c-header? flag (not bundled here); the 53.2 gate uses
      // flat-precedence class-free fixtures.
      let ast = parse-dylan(tokens);
      format-out("%s", collect-top-names(ast, source));
    end;
  end;
end function sema-main;
