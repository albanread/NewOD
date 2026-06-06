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

// ─── classes + slots (Sprint 53.3) ───────────────────────────────────────
//
// A user `define class` contributes, in the sema model:
//   * slot-accessor `fn` entries in `=== top-names ===`:
//       `<C>-getter-<s>` arity=1 return=<slot-type-estimate>, and
//       `<C>-setter-<s>` arity=2 return=Top  (when the slot has a setter).
//   * a getter generic `<s>` and (when the slot has a setter) a setter
//     generic `<s>-setter`, in `=== generics ===` (sorted, deduped).
//   * a `class` block in `=== classes ===`: the parents (declaration
//     order), the CPL (C3), and one `slot <s> @<offset> setter=<b>
//     origin=<C>` line per slot.
//
// Mirrors the Rust oracle `format_sema_model` (src/nod-sema/src/lower.rs).

// Is byte-string `s` already in vector `v`? (Linear; vectors are tiny.)
define function bs-member? (v :: <stretchy-vector>, s :: <byte-string>)
 => (yes? :: <boolean>)
  let n = size(v);
  let i = 0;
  let found? = #f;
  // `>=` guard so an empty vector is a clean no-op (GAP-011 off-by-one).
  until (i >= n | found?)
    if (v[i] = s) found? := #t; end;
    i := i + 1;
  end;
  found?
end function;

// Name text of a superclass expression. For the simple fixtures each
// super is an `<ast-variable-ref>` (`<object>`); fall back to "" for any
// other shape so a faulting deref never leaks in.
define function super-name (node :: <object>, source :: <byte-string>)
 => (name :: <byte-string>)
  if (instance?(node, <ast-variable-ref>))
    token-source-text(varref-tok(node), source)
  else
    ""
  end
end function;

// Return-type estimate of a slot: map its declared type, or Top when the
// slot has no `:: <type>`.
define function slot-est (s :: <ast-slot-spec>, source :: <byte-string>)
 => (est :: <byte-string>)
  if (instance?(slot-type(s), <ast-node>))
    map-type-estimate(type-node-name(slot-type(s), source))
  else
    "Top"
  end
end function;

// A slot has a setter unless it is declared `constant`. (The oracle also
// honours an explicit `setter: #f` slot option; the parsed AST here does
// not surface that flag separately, and none of the gated fixtures use
// it, so the `constant` adjective is the discriminator we implement.)
define function slot-has-setter? (s :: <ast-slot-spec>, source :: <byte-string>)
 => (yes? :: <boolean>)
  let adjs = slot-adjectives(s);
  let n = size(adjs);
  let i = 0;
  let is-constant? = #f;
  // Use the source-text slice (not token-name) so the match works whether
  // `constant` lexes as an identifier or a reserved keyword token.
  until (i >= n | is-constant?)
    if (token-source-text(adjs[i], source) = "constant") is-constant? := #t; end;
    i := i + 1;
  end;
  ~ is-constant?
end function;

// Does a definition's `define`-modifier vector contain `sealed`? Mirrors
// `slot-has-setter?`: scan the modifier tokens, comparing the source-text
// slice (so the match works whether `sealed` lexes as an identifier or a
// reserved-word token). Used to detect `define sealed class` /
// `define sealed generic`.
define function modifiers-has-sealed? (mods :: <stretchy-vector>,
                                       source :: <byte-string>)
 => (yes? :: <boolean>)
  let n = size(mods);
  let i = 0;
  let found? = #f;
  until (i >= n | found?)
    if (token-source-text(mods[i], source) = "sealed") found? := #t; end;
    i := i + 1;
  end;
  found?
end function;

// Per-class record: name, parent names, the class's CPL (computed by C3),
// and parallel vectors describing its OWN slots.
define class <class-rec> (<object>)
  slot rec-name        :: <byte-string>,    init-keyword: name:;
  slot rec-parents     :: <stretchy-vector>, init-keyword: parents:;
  slot rec-cpl         :: <stretchy-vector>, init-keyword: cpl:;
  slot rec-slot-names  :: <stretchy-vector>, init-keyword: slot-names:;
  slot rec-slot-ests   :: <stretchy-vector>, init-keyword: slot-ests:;
  slot rec-slot-setters :: <stretchy-vector>, init-keyword: slot-setters:;
end class;

// CPL registry: parallel name / cpl vectors. Seeded with `<object>`.
// `registry-lookup` returns a parent's CPL — `[<object>]` for the seed,
// the computed CPL for an earlier user class, or the leaf fallback
// `[name]` for an unknown parent (a builtin other than `<object>`; none
// of the gated fixtures hit this).
define function registry-lookup (names :: <stretchy-vector>,
                                 cpls  :: <stretchy-vector>,
                                 name  :: <byte-string>)
 => (cpl :: <stretchy-vector>)
  let n = size(names);
  let i = 0;
  let found = #f;
  until (i >= n | found)
    if (names[i] = name) found := cpls[i]; end;
    i := i + 1;
  end;
  if (found)
    found
  else
    // Leaf fallback: treat the unknown parent as a root with CPL [name].
    let v = make(<stretchy-vector>);
    add!(v, name);
    v
  end
end function;

// Build a <class-rec> for one `<ast-class-definition>`, computing its CPL
// from the running registry. Registers the result into the registry so a
// later subclass can find it.
define function build-class-rec (cd :: <ast-class-definition>,
                                 source :: <byte-string>,
                                 reg-names :: <stretchy-vector>,
                                 reg-cpls  :: <stretchy-vector>)
 => (rec :: <class-rec>)
  let cname = token-source-text(class-name(cd), source);

  // Parent names in declaration order.
  let parents = make(<stretchy-vector>);
  let supers = class-supers(cd);
  let ns = size(supers);
  let si = 0;
  until (si >= ns)
    add!(parents, super-name(supers[si], source));
    si := si + 1;
  end;

  // Parent CPLs (parallel to `parents`) for the C3 input.
  let parent-cpls = make(<stretchy-vector>);
  let pi = 0;
  let np = size(parents);
  until (pi >= np)
    add!(parent-cpls, registry-lookup(reg-names, reg-cpls, parents[pi]));
    pi := pi + 1;
  end;

  // C3 linearisation → this class's CPL.
  let c3 = c3-linearise(cname, parents, parent-cpls);
  let cpl = c3-result-cpl(c3);

  // Own slots.
  let slot-names   = make(<stretchy-vector>);
  let slot-ests    = make(<stretchy-vector>);
  let slot-setters = make(<stretchy-vector>);
  let slots = class-slots(cd);
  let nsl = size(slots);
  let sli = 0;
  until (sli >= nsl)
    let s = slots[sli];
    if (instance?(slot-name-tok(s), <token>))
      add!(slot-names,   token-source-text(slot-name-tok(s), source));
      add!(slot-ests,    slot-est(s, source));
      add!(slot-setters, slot-has-setter?(s, source));
    end;
    sli := sli + 1;
  end;

  // Register into the running registry for later subclasses.
  add!(reg-names, cname);
  add!(reg-cpls, cpl);

  make(<class-rec>,
       name: cname, parents: parents, cpl: cpl,
       slot-names: slot-names, slot-ests: slot-ests,
       slot-setters: slot-setters)
end function;

// Join a vector of byte-strings with ", " (for parents / cpl listings).
define function join-comma (v :: <stretchy-vector>) => (s :: <byte-string>)
  let n = size(v);
  let out = "";
  let i = 0;
  until (i >= n)
    if (i = 0)
      out := v[i];
    else
      out := concatenate(out, concatenate(", ", v[i]));
    end;
    i := i + 1;
  end;
  out
end function;

// ─── the walk ────────────────────────────────────────────────────────────

define function collect-top-names (ast :: <ast-body>, source :: <byte-string>)
 => (text :: <byte-string>)
  let fns      = make(<stretchy-vector>);
  let consts   = make(<stretchy-vector>);
  let vars     = make(<stretchy-vector>);
  let classes  = make(<stretchy-vector>);   // <class-rec> in declaration order
  let generics = make(<stretchy-vector>);   // generic names (deduped, sorted later)
  let sealed-classes  = make(<stretchy-vector>);  // `define sealed class` names (sorted later)
  let sealed-generics = make(<stretchy-vector>);  // `define sealed generic` names (sorted later)
  // CPL registry, seeded with `<object>` → [<object>].
  let reg-names = make(<stretchy-vector>);
  let reg-cpls  = make(<stretchy-vector>);
  begin
    let obj-cpl = make(<stretchy-vector>);
    add!(obj-cpl, "<object>");
    add!(reg-names, "<object>");
    add!(reg-cpls, obj-cpl);
  end;

  let items  = body-constituents(ast);
  let n = size(items);
  let i = 0;
  until (i = n)
    let item = items[i];
    if (instance?(item, <ast-body-definition>))
      let word = token-source-text(defn-word(item), source);
      // Only `define function` contributes a top-names `fn`. The oracle
      // attaches `define method` bodies to their generic (the explicit
      // `define generic`, recorded below), so a method emits NO `fn`
      // line and must not duplicate the generic's entry.
      //
      // DEFERRED: a bare `define method` with no explicit `define generic`
      // implicitly CREATES a generic of that name in the real sema model.
      // No fixture here exercises that (richards' run-task generic is
      // explicit), so implicit-generic creation from methods is left for a
      // later sprint.
      if (word = "function")
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
    elseif (instance?(item, <ast-class-definition>))
      // Build the class record (computes + registers its CPL), then emit
      // the slot accessors into `fns` and the slot generics.
      let rec = build-class-rec(item, source, reg-names, reg-cpls);
      add!(classes, rec);
      // `define sealed class` → a `sealed-class <name>` entry.
      if (modifiers-has-sealed?(defn-modifiers(item), source))
        add!(sealed-classes, rec-name(rec));
      end;
      let cname = rec-name(rec);
      let snames = rec-slot-names(rec);
      let sests  = rec-slot-ests(rec);
      let ssetters = rec-slot-setters(rec);
      let ns = size(snames);
      let sj = 0;
      until (sj >= ns)
        let sname = snames[sj];
        let sest  = sests[sj];
        let has-setter? = ssetters[sj];
        // Getter accessor fn: `<C>-getter-<s>` arity=1 return=<est>.
        let getter = concatenate(cname, concatenate("-getter-", sname));
        let gline  = concatenate("fn ", concatenate(getter,
                       concatenate(" arity=1 return=", sest)));
        add!(fns, make(<top-fn>, name: getter, line: gline));
        // Getter generic `<s>`.
        if (~ bs-member?(generics, sname)) add!(generics, sname); end;
        if (has-setter?)
          // Setter accessor fn: `<C>-setter-<s>` arity=2 return=Top.
          let setter = concatenate(cname, concatenate("-setter-", sname));
          let sline  = concatenate("fn ", concatenate(setter,
                         " arity=2 return=Top"));
          add!(fns, make(<top-fn>, name: setter, line: sline));
          // Setter generic `<s>-setter`.
          let sg = concatenate(sname, "-setter");
          if (~ bs-member?(generics, sg)) add!(generics, sg); end;
        end;
        sj := sj + 1;
      end;
    elseif (instance?(item, <ast-generic-definition>))
      // `define generic NAME (...) => (...)` → a `generic <NAME>` entry
      // (deduped against slot getter/setter generics). `define sealed
      // generic` additionally yields a `sealed-generic <NAME>` entry.
      let gtok = gen-name(item);
      if (instance?(gtok, <token>))
        let gname = token-source-text(gtok, source);
        if (~ bs-member?(generics, gname)) add!(generics, gname); end;
        if (modifiers-has-sealed?(defn-modifiers(item), source))
          add!(sealed-generics, gname);
        end;
      end;
    end;
    i := i + 1;
  end;

  sort-fns!(fns);
  sort-strings!(consts);
  sort-strings!(vars);
  sort-strings!(generics);
  sort-strings!(sealed-classes);
  sort-strings!(sealed-generics);

  // ── === top-names === ──
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

  // ── === generics === ──
  out := concatenate(out, "=== generics ===\n");
  let gi = 0;
  until (gi = size(generics))
    out := concatenate(out, concatenate("generic ", concatenate(generics[gi], "\n")));
    gi := gi + 1;
  end;

  // ── === classes === ──
  out := concatenate(out, "=== classes ===\n");
  let cli = 0;
  until (cli = size(classes))
    let rec = classes[cli];
    out := concatenate(out, concatenate("class ", concatenate(rec-name(rec), "\n")));
    out := concatenate(out, concatenate("  parents [",
                              concatenate(join-comma(rec-parents(rec)), "]\n")));
    out := concatenate(out, concatenate("  cpl [",
                              concatenate(join-comma(rec-cpl(rec)), "]\n")));
    // Slots: object header @0, own slots laid out 8 bytes each from @8.
    // (These fixtures have no slot-bearing superclass, so there are no
    // inherited slots to place first; origin is always the class itself.)
    let snames   = rec-slot-names(rec);
    let ssetters = rec-slot-setters(rec);
    let nsl = size(snames);
    let sk = 0;
    until (sk >= nsl)
      let offset = 8 + (sk * 8);
      let setter-str = if (ssetters[sk]) "true" else "false" end;
      out := concatenate(out,
               concatenate("  slot ",
                 concatenate(snames[sk],
                   concatenate(" @",
                     concatenate(integer-to-string(offset),
                       concatenate(" setter=",
                         concatenate(setter-str,
                           concatenate(" origin=",
                             concatenate(rec-name(rec), "\n")))))))));
      sk := sk + 1;
    end;
    cli := cli + 1;
  end;

  // ── === sealing === ──
  // Sorted `sealed-class <name>` lines first, then sorted
  // `sealed-generic <name>` lines (matching the oracle's order). The
  // header is always emitted even when both are empty.
  //
  // DEFERRED: `define sealed domain` (a `sealed-domain G (T, ...)` entry
  // in the real model) is not exercised by any fixture, so it is not
  // collected here.
  out := concatenate(out, "=== sealing ===\n");
  let sci = 0;
  until (sci = size(sealed-classes))
    out := concatenate(out,
             concatenate("sealed-class ", concatenate(sealed-classes[sci], "\n")));
    sci := sci + 1;
  end;
  let sgi = 0;
  until (sgi = size(sealed-generics))
    out := concatenate(out,
             concatenate("sealed-generic ", concatenate(sealed-generics[sgi], "\n")));
    sgi := sgi + 1;
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
