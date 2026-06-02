Module: dylan-lexer
Precedence: c

// Sprint 52.4 — substitution + hygiene parity unit driver.
//
// For a fixed set of (define-macro, call-site) cases: collect the def,
// match rule 0's pattern against the call, then `substitute-hyg` the
// template with the bindings under a PINNED hygiene nonce ("42"), and
// print the expansion text:
//
//   EXPAND <name> = <substituted + hygiene-renamed text>
//   NOMATCH                                  (if the pattern didn't match)
//
// The Rust gate `tests/nod-tests/tests/macro_expand.rs` runs the same
// cases through `nod_macro::substitute` (nonce 42) and asserts the
// whitespace-normalised expansions are byte-identical. Cases cover:
//   * substitution only, no binders (unless),
//   * a `let`-introduced binder (renamed everywhere),
//   * a `method (…)` param binder,
//   * the real stdlib `for-each` (the `%fip-state` binder renamed, the
//     `?var`/`?coll`/`?body` pattern vars NOT renamed).

define function run-expand (nm :: <byte-string>,
                            def-src :: <byte-string>,
                            call-src :: <byte-string>) => ()
  let defs = collect-macro-defs(def-src);
  if (size(defs) = 0)
    format-out("EXPAND %s = NODEF\n", nm);
  else
    let rule       = macro-def-rules(defs[0])[0];
    let pattern    = macro-rule-pattern(rule);
    let template   = macro-rule-template(rule);
    let pvars      = collect-pattern-var-names(pattern);
    let call-toks  = lex-source-to-toks(call-src);
    let call-frags = tokens-to-fragments(call-toks);
    let b = match-pattern(pattern, call-frags);
    if (~ b)
      format-out("EXPAND %s = NOMATCH\n", nm);
    else
      let text = substitute-hyg(template, b, pvars, "42");
      format-out("EXPAND %s = %s\n", nm, text);
    end;
  end;
end function run-expand;

define function expand-main () => ()
  run-expand("unless",
             "define macro unless { unless ?cond:expression ?body:body end } => { if (~ ?cond) ?body else #f end } end macro;",
             "unless x (foo) end");
  run-expand("let-binder",
             "define macro lt { lt ?e:expression end } => { let tmp = ?e ; tmp end } end macro;",
             "lt (foo) end");
  run-expand("method-param",
             "define macro mm { mm ?e:expression end } => { method (q) q end } end macro;",
             "mm (z) end");
  run-expand("for-each",
             "define macro for-each { for-each (?var:name in ?coll:expression) ?body:body end } => { begin let %fip-state = %fip-init(?coll); until (%fip-finished?(%fip-state)) let ?var = %fip-current-element(%fip-state); ?body; %fip-advance!(%fip-state) end end } end macro;",
             "for-each (i in xs) (work) end");
end function expand-main;
