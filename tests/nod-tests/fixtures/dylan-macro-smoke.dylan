Module: dylan-macro-smoke

// Sprint 50a — Dylan-side macro engine smoke test.
//
// First step on the "retire nod-macro" track of the year-3
// self-hosting plan. The Rust nod-macro crate (~1900 lines) implements
// pattern matching + template substitution over `Fragment`s — a
// token-grouping structure that sits between raw tokens and parsed
// AST. This fixture mirrors enough of that to expand ONE rule (the
// stdlib `unless` macro) and print the resulting source text so a
// Rust-side test can byte-compare against nod-macro's output.
//
// Sprint 50a does NOT:
//   * Parse `define macro` source — the unless rule is hand-built
//     here. Source parsing is Sprint 50b.
//   * Walk an AST applying expansions — Sprint 50c wires that.
//   * Implement hygiene rename — deferred until Sprint 50c/d.
//   * Wire up to `<token>` from dylan-lexer.dylan — uses a local
//     minimal `<tok>` to keep the smoke standalone.

// ─── Minimal token + fragment shape ───────────────────────────────────────

// A token is (kind, text). Real `<token>` from dylan-lexer.dylan has
// the same shape plus spans; we omit spans for the smoke. Token kinds
// used in this smoke: #"ident", #"kw-end", #"punct".

define class <tok> (<object>)
  slot tok-kind :: <symbol>,      init-keyword: kind:;
  slot tok-text :: <byte-string>, init-keyword: text:;
end class;

define function make-tok (k :: <symbol>, t :: <byte-string>) => (x :: <tok>)
  make(<tok>, kind: k, text: t)
end function;

// A Fragment is either a single token or a grouped sequence
// `( … )`, `[ … ]`, `{ … }`, etc. The macro engine matches at this
// level — call-site fragments against pattern elements.

define class <fragment> (<object>)
end class;

define class <token-fragment> (<fragment>)
  slot tfrag-tok :: <tok>, init-keyword: tok:;
end class;

define class <group-fragment> (<fragment>)
  slot gfrag-kind :: <symbol>,           init-keyword: kind:;   // #"paren", #"bracket", #"brace"
  slot gfrag-body :: <stretchy-vector>,  init-keyword: body:;
end class;

define function make-token-frag (t :: <tok>) => (f :: <token-fragment>)
  make(<token-fragment>, tok: t)
end function;

define function make-group-frag (kind :: <symbol>, body :: <stretchy-vector>)
 => (f :: <group-fragment>)
  make(<group-fragment>, kind: kind, body: body)
end function;

// ─── Pattern + template elements ──────────────────────────────────────────
//
// PatternElem variants (matching Rust nod-macro):
//   <pat-literal>  — a fixed token the call must reproduce
//   <pat-variable> — `?name:kind`, binds one or more call fragments
//   <pat-group>    — `( … )` etc, recursively patterned

define class <pattern-elem> (<object>)
end class;

define class <pat-literal> (<pattern-elem>)
  slot pat-lit-tok :: <tok>, init-keyword: tok:;
end class;

define class <pat-variable> (<pattern-elem>)
  slot pat-var-name :: <byte-string>, init-keyword: name:;
  slot pat-var-kind :: <symbol>,       init-keyword: kind:;
    // #"expression" | #"body" — Sprint 50a subset.
end class;

define class <pat-group> (<pattern-elem>)
  slot pat-grp-kind :: <symbol>,          init-keyword: kind:;
  slot pat-grp-body :: <stretchy-vector>, init-keyword: body:;
end class;

// TemplateElem variants. `<tpl-substitution>` carries the binding
// name to splice; everything else is emitted verbatim.

define class <template-elem> (<object>)
end class;

define class <tpl-literal> (<template-elem>)
  slot tpl-lit-tok :: <tok>, init-keyword: tok:;
end class;

define class <tpl-substitution> (<template-elem>)
  slot tpl-sub-name :: <byte-string>, init-keyword: name:;
end class;

define class <tpl-group> (<template-elem>)
  slot tpl-grp-kind :: <symbol>,          init-keyword: kind:;
  slot tpl-grp-body :: <stretchy-vector>, init-keyword: body:;
end class;

// ─── Bindings (linear list-of-pairs for now) ──────────────────────────────
//
// A bindings table maps a pattern-variable name (<byte-string>) to a
// captured sequence of fragments (<stretchy-vector>). The Rust
// implementation uses a HashMap; for Sprint 50a's tiny tables (≤4
// entries) a linear scan is faster than the hash overhead.

define class <binding> (<object>)
  slot binding-name  :: <byte-string>,    init-keyword: name:;
  slot binding-frags :: <stretchy-vector>, init-keyword: frags:;
end class;

define function make-bindings () => (b :: <stretchy-vector>)
  make(<stretchy-vector>)
end function;

define function bindings-add! (b :: <stretchy-vector>, name :: <byte-string>,
                               frags :: <stretchy-vector>) => ()
  add!(b, make(<binding>, name: name, frags: frags));
end function;

define function bindings-get (b :: <stretchy-vector>, name :: <byte-string>)
 => (frags :: <object>)
  // Returns the <stretchy-vector> of captured fragments, or #f on miss.
  let n = size(b);
  let i = 0;
  let found = #f;
  until (i = n | found)
    let entry = b[i];
    if (binding-name(entry) = name)
      found := binding-frags(entry);
    else
      i := i + 1;
    end;
  end;
  found
end function;

// ─── Pattern matching ─────────────────────────────────────────────────────
//
// match-pattern takes a pattern (stretchy-vector of <pattern-elem>)
// and a call site's fragments (stretchy-vector of <fragment>) and
// returns either a bindings table or #f on mismatch.
//
// Sprint 50a supports:
//   * <pat-literal>  — token-kind + text equality
//   * <pat-variable> with kind #"expression" — binds exactly one frag
//   * <pat-variable> with kind #"body"       — binds 0+ frags up to
//                                              the first match of the
//                                              NEXT literal in pattern,
//                                              or to end-of-call if
//                                              pattern has no trailer.
//                                              Depth-aware on `end`.
//   * <pat-group>    — recursive match on body
//
// Greedy, left-to-right, no backtracking. Same approach as Rust
// nod-macro::match_pattern at Sprint-17 level.

define function tok-frag? (f :: <fragment>) => (yes? :: <boolean>)
  instance?(f, <token-fragment>)
end function;

define function group-frag? (f :: <fragment>) => (yes? :: <boolean>)
  instance?(f, <group-fragment>)
end function;

// Predicate: does this call-site fragment match a literal-pattern's
// (kind, text)? Only token fragments can match literals.
define function frag-matches-literal? (f :: <fragment>, lit :: <tok>)
 => (yes? :: <boolean>)
  if (tok-frag?(f))
    let tf = f;
    let t = tfrag-tok(tf);
    tok-kind(t) = tok-kind(lit) & tok-text(t) = tok-text(lit)
  else
    #f
  end
end function;

// Recognise call-site idents that open an end-terminated body form.
// Used by the body-matcher's depth-aware scan. List mirrors the Rust
// engine's tok_text_eq cluster.
define function opens-end-form? (text :: <byte-string>) => (yes? :: <boolean>)
  text = "if" | text = "unless" | text = "while" | text = "until"
    | text = "for" | text = "block" | text = "select" | text = "case"
    | text = "cond" | text = "begin" | text = "method" | text = "when"
    | text = "with-cleanup"
end function;

// Scan `call[ci..]` for the first position whose fragment matches
// `lit`, tracking nesting so a nested `if … end` doesn't claim the
// outer `unless`'s terminator. Returns the absolute index or #f.
define function find-body-end (call :: <stretchy-vector>, ci :: <integer>,
                               lit :: <tok>) => (pos :: <object>)
  let n = size(call);
  let depth = 0;
  let i = ci;
  let found = #f;
  let kw-end-lit = tok-kind(lit) = #"kw-end";
  until (i = n | found)
    let f = call[i];
    if (tok-frag?(f))
      let t = tfrag-tok(f);
      if (kw-end-lit & tok-kind(t) = #"ident" & opens-end-form?(tok-text(t)))
        depth := depth + 1;
      elseif (frag-matches-literal?(f, lit))
        if (depth = 0)
          found := i;
        else
          depth := depth - 1;
        end;
      end;
    end;
    if (~ found) i := i + 1; end;
  end;
  found
end function;

// Count trailing literal/group pattern elements — used as the body's
// stop-point when the next pattern element isn't a literal.
define function count-trailing-literals (pattern :: <stretchy-vector>,
                                         start :: <integer>) => (n :: <integer>)
  let m = size(pattern);
  let n = 0;
  let i = m - 1;
  let stop? = #f;
  until (i < start | stop?)
    let p = pattern[i];
    if (instance?(p, <pat-literal>) | instance?(p, <pat-group>))
      n := n + 1;
      i := i - 1;
    else
      stop? := #t;
    end;
  end;
  n
end function;

define function match-pattern (pattern :: <stretchy-vector>,
                               call    :: <stretchy-vector>)
 => (b :: <object>)
  let b      = make-bindings();
  let pi     = 0;
  let ci     = 0;
  let pn     = size(pattern);
  let cn     = size(call);
  let fail?  = #f;
  until (pi = pn | fail?)
    let p = pattern[pi];
    if (instance?(p, <pat-literal>))
      if (ci >= cn)
        fail? := #t;
      else
        let f = call[ci];
        if (frag-matches-literal?(f, pat-lit-tok(p)))
          ci := ci + 1;
          pi := pi + 1;
        else
          fail? := #t;
        end;
      end;
    elseif (instance?(p, <pat-variable>))
      let kind = pat-var-kind(p);
      if (kind = #"expression")
        if (ci >= cn)
          fail? := #t;
        else
          let frags = make(<stretchy-vector>);
          add!(frags, call[ci]);
          bindings-add!(b, pat-var-name(p), frags);
          ci := ci + 1;
          pi := pi + 1;
        end;
      elseif (kind = #"body")
        // Determine body's end position: scan to the next literal in
        // pattern, or fall back to len(call) - count_trailing_literals.
        // Statement-form (not let-binding an if-expression) to dodge
        // the GAP-011-family LLVM SSA-dominance issue on heap-typed
        // join values.
        let body-end = cn - count-trailing-literals(pattern, pi + 1);
        if (pi + 1 < pn & instance?(pattern[pi + 1], <pat-literal>))
          let next-lit = pat-lit-tok(pattern[pi + 1]);
          let scanned  = find-body-end(call, ci, next-lit);
          if (scanned) body-end := scanned; end;
        end;
        let frags = make(<stretchy-vector>);
        let j = ci;
        until (j = body-end)
          add!(frags, call[j]);
          j := j + 1;
        end;
        bindings-add!(b, pat-var-name(p), frags);
        ci := body-end;
        pi := pi + 1;
      else
        // Unsupported pattern kind for Sprint 50a.
        fail? := #t;
      end;
    elseif (instance?(p, <pat-group>))
      if (ci >= cn)
        fail? := #t;
      else
        let f = call[ci];
        if (~ group-frag?(f))
          fail? := #t;
        else
          let g = f;
          if (gfrag-kind(g) ~= pat-grp-kind(p))
            fail? := #t;
          else
            let sub = match-pattern(pat-grp-body(p), gfrag-body(g));
            if (~ sub)
              fail? := #t;
            else
              // Merge sub-bindings into b.
              let m = size(sub);
              let k = 0;
              until (k = m)
                let e = sub[k];
                add!(b, e);
                k := k + 1;
              end;
              ci := ci + 1;
              pi := pi + 1;
            end;
          end;
        end;
      end;
    else
      fail? := #t;
    end;
  end;
  if (fail? | ci ~= cn)
    #f
  else
    b
  end
end function;

// ─── Template substitution → text ─────────────────────────────────────────
//
// The Rust `substitute` emits a text buffer; the caller re-lexes and
// re-parses. We mirror that: walk the template, accumulating into a
// <stretchy-vector> of <byte-string> chunks, then concatenate via the
// stdlib's reduce + concatenate.
//
// Spacing policy: insert a single space between any two adjacent
// chunks unless the surroundings are tight (open paren before, close
// paren / comma / semicolon after). Same heuristic the Rust engine
// uses to keep emitted text readable.

define function emit-tok (out :: <stretchy-vector>, t :: <tok>) => ()
  add!(out, tok-text(t));
end function;

define function emit-frag (out :: <stretchy-vector>, f :: <fragment>) => ()
  if (tok-frag?(f))
    emit-tok(out, tfrag-tok(f));
  else
    let g = f;
    let k = gfrag-kind(g);
    // Statement-form open/close pick: heap-typed `let X = if ... end`
    // hits the GAP-011-family LLVM SSA-dominance issue (deferred fix,
    // see Sprint 49d retro). Statement-form sidesteps it.
    let open  = "{";
    let close = "}";
    if (k = #"paren")
      open := "("; close := ")";
    elseif (k = #"bracket")
      open := "["; close := "]";
    end;
    add!(out, open);
    let body = gfrag-body(g);
    let n = size(body);
    let i = 0;
    until (i = n)
      emit-frag(out, body[i]);
      i := i + 1;
    end;
    add!(out, close);
  end;
end function;

define function emit-template (template :: <stretchy-vector>,
                               bindings :: <stretchy-vector>,
                               out      :: <stretchy-vector>) => ()
  let n = size(template);
  let i = 0;
  until (i = n)
    let e = template[i];
    if (instance?(e, <tpl-literal>))
      emit-tok(out, tpl-lit-tok(e));
    elseif (instance?(e, <tpl-substitution>))
      let frags = bindings-get(bindings, tpl-sub-name(e));
      if (frags)
        let m = size(frags);
        let j = 0;
        until (j = m)
          emit-frag(out, frags[j]);
          j := j + 1;
        end;
      end;
    elseif (instance?(e, <tpl-group>))
      let k = tpl-grp-kind(e);
      let open  = "{";
      let close = "}";
      if (k = #"paren")
        open := "("; close := ")";
      elseif (k = #"bracket")
        open := "["; close := "]";
      end;
      add!(out, open);
      emit-template(tpl-grp-body(e), bindings, out);
      add!(out, close);
    end;
    i := i + 1;
  end;
end function;

// Join chunks with single spaces. A more sophisticated pass would
// respect cluster boundaries (no space between an ident and its
// opening paren); Sprint 50b will refine this.
define function join-chunks (chunks :: <stretchy-vector>) => (s :: <byte-string>)
  let n = size(chunks);
  let result = "";
  if (n > 0)
    result := chunks[0];
    let i = 1;
    until (i = n)
      result := concatenate(result, " ");
      result := concatenate(result, chunks[i]);
      i := i + 1;
    end;
  end;
  result
end function;

define function substitute (template :: <stretchy-vector>,
                            bindings :: <stretchy-vector>)
 => (s :: <byte-string>)
  let out = make(<stretchy-vector>);
  emit-template(template, bindings, out);
  join-chunks(out)
end function;

// ─── Sprint 50b — parse `define macro` body fragments → <macro-def> ──────
//
// The Rust nod-macro grammar for a definition body is:
//   macro-body : rule (';' rule)*
//   rule       : '{' pattern '}' '=>' '{' template '}'
//   pattern    : pattern-elem*
//   template   : template-elem*
//   pat-elem   : literal | '?' name ':' kind | group   (group recursive)
//   tpl-elem   : literal | '?' name             | group   (group recursive)
//
// In tokenised form the lexer glues `name:` into a single
// `#"keyword-name"` token. So the common physical shape for
// `?cond:expression` is three tokens: `?`, `cond:`, `expression`.
// Sprint 50b accepts that form (mirrors nod-macro's parse_pattern_var_head
// common arm). The explicit-spaces form `? cond : expression` is rare
// and deferred to 50c when we plug the real lexer in.

// Sprint 50b: a rule wraps one (pattern, template) pair so a single
// def can carry multiple. Sprint 50a's match/substitute happily took
// the two halves separately; the wrapper is just an organisational
// convenience for the def-level parser.
define class <macro-rule> (<object>)
  slot macro-rule-pattern  :: <stretchy-vector>, init-keyword: pattern:;
  slot macro-rule-template :: <stretchy-vector>, init-keyword: template:;
end class;

define class <macro-def> (<object>)
  slot macro-def-name  :: <byte-string>,    init-keyword: name:;
  slot macro-def-rules :: <stretchy-vector>, init-keyword: rules:;
end class;

// Predicate: is `f` a single-token fragment whose token has `kind` and `text`?
define function tok-is? (f :: <fragment>, kind :: <symbol>, text :: <byte-string>)
 => (yes? :: <boolean>)
  if (tok-frag?(f))
    let t = tfrag-tok(f);
    tok-kind(t) = kind & tok-text(t) = text
  else
    #f
  end
end function;

// Strip a trailing `:` from `s` (used to unglue the keyword-name's name).
define function strip-trailing-colon (s :: <byte-string>) => (r :: <byte-string>)
  let n = size(s);
  if (n > 0 & %byte-string-element(s, n - 1) = 58)
    copy-sequence(s, 0, n - 1)
  else
    s
  end
end function;

// Parse one pattern-elem from `body[i]`, return (elem, consumed-count).
define function parse-pattern-elem (body :: <stretchy-vector>, i :: <integer>)
 => (elem :: <pattern-elem>, consumed :: <integer>)
  let f = body[i];
  let result :: <pattern-elem> = make(<pat-literal>, tok: make-tok(#"ident", "?"));
  let consumed = 1;
  if (group-frag?(f))
    let g = f;
    let inner-pattern = parse-pattern-body(gfrag-body(g));
    result := make(<pat-group>, kind: gfrag-kind(g), body: inner-pattern);
  elseif (tok-is?(f, #"punct", "?"))
    // Expect: ?  keyword-name(name:)  ident(kind)
    let name-frag = body[i + 1];
    let kind-frag = body[i + 2];
    let name-tok  = tfrag-tok(name-frag);
    let kind-tok  = tfrag-tok(kind-frag);
    let name      = strip-trailing-colon(tok-text(name-tok));
    let kind-text = tok-text(kind-tok);
    let kind-sym  = #"expression";
    if (kind-text = "body")       kind-sym := #"body";
    elseif (kind-text = "expression") kind-sym := #"expression";
    end;
    result := make(<pat-variable>, name: name, kind: kind-sym);
    consumed := 3;
  else
    result := make(<pat-literal>, tok: tfrag-tok(f));
  end;
  values(result, consumed)
end function;

define function parse-pattern-body (body :: <stretchy-vector>)
 => (pat :: <stretchy-vector>)
  let out = make(<stretchy-vector>);
  let n = size(body);
  let i = 0;
  until (i = n)
    let (elem, consumed) = parse-pattern-elem(body, i);
    add!(out, elem);
    i := i + consumed;
  end;
  out
end function;

// Parse one template-elem. Templates only have `?name` (no kind).
define function parse-template-elem (body :: <stretchy-vector>, i :: <integer>)
 => (elem :: <template-elem>, consumed :: <integer>)
  let f = body[i];
  let result :: <template-elem> = make(<tpl-literal>, tok: make-tok(#"ident", "?"));
  let consumed = 1;
  if (group-frag?(f))
    let g = f;
    let inner-tpl = parse-template-body(gfrag-body(g));
    result := make(<tpl-group>, kind: gfrag-kind(g), body: inner-tpl);
  elseif (tok-is?(f, #"punct", "?"))
    let name-frag = body[i + 1];
    let name-tok  = tfrag-tok(name-frag);
    result := make(<tpl-substitution>, name: tok-text(name-tok));
    consumed := 2;
  else
    result := make(<tpl-literal>, tok: tfrag-tok(f));
  end;
  values(result, consumed)
end function;

define function parse-template-body (body :: <stretchy-vector>)
 => (tpl :: <stretchy-vector>)
  let out = make(<stretchy-vector>);
  let n = size(body);
  let i = 0;
  until (i = n)
    let (elem, consumed) = parse-template-elem(body, i);
    add!(out, elem);
    i := i + consumed;
  end;
  out
end function;

// Parse one rule starting at `frags[i]`: expects `{ pattern } => { template }`.
// Returns (rule, next-i).
define function parse-rule (frags :: <stretchy-vector>, start :: <integer>)
 => (rule :: <macro-rule>, next :: <integer>)
  let pat-group  = frags[start];
  let arrow-frag = frags[start + 1];
  let tpl-group  = frags[start + 2];
  let pattern  = parse-pattern-body(gfrag-body(pat-group));
  let template = parse-template-body(gfrag-body(tpl-group));
  let rule = make(<macro-rule>, pattern: pattern, template: template);
  values(rule, start + 3)
end function;

// Parse a complete `define macro NAME` body: 1+ rules separated by `;`.
define function parse-macro-def (name :: <byte-string>, body :: <stretchy-vector>)
 => (def :: <macro-def>)
  let rules = make(<stretchy-vector>);
  let n = size(body);
  let i = 0;
  until (i >= n)
    // Skip a leading `;` between rules.
    if (i < n & tok-is?(body[i], #"punct", ";"))
      i := i + 1;
    else
      let (rule, next) = parse-rule(body, i);
      add!(rules, rule);
      i := next;
    end;
  end;
  make(<macro-def>, name: name, rules: rules)
end function;

// ─── Hand-built unless rule + call-site smoke ────────────────────────────
//
// The stdlib `unless` macro is:
//
//   define macro unless
//     { unless ?cond:expression ?body:body end }
//       => { if (~ ?cond) ?body else #f end }
//   end macro;
//
// We hand-build that rule's pattern and template, then build a call
// site `unless x (foo) end` as fragments, match, and substitute. The
// expected output text is the if-expansion.

define function build-unless-rule ()
 => (pattern :: <stretchy-vector>, template :: <stretchy-vector>)
  // Pattern: unless ?cond:expression ?body:body end
  let pat = make(<stretchy-vector>);
  add!(pat, make(<pat-literal>,
                 tok: make-tok(#"ident", "unless")));
  add!(pat, make(<pat-variable>,
                 name: "cond", kind: #"expression"));
  add!(pat, make(<pat-variable>,
                 name: "body", kind: #"body"));
  add!(pat, make(<pat-literal>,
                 tok: make-tok(#"kw-end", "end")));
  // Template: if (~ ?cond) ?body else #f end
  let tpl = make(<stretchy-vector>);
  add!(tpl, make(<tpl-literal>, tok: make-tok(#"ident", "if")));
  let paren-body = make(<stretchy-vector>);
  add!(paren-body, make(<tpl-literal>, tok: make-tok(#"punct", "~")));
  add!(paren-body, make(<tpl-substitution>, name: "cond"));
  add!(tpl, make(<tpl-group>, kind: #"paren", body: paren-body));
  add!(tpl, make(<tpl-substitution>, name: "body"));
  add!(tpl, make(<tpl-literal>, tok: make-tok(#"ident", "else")));
  add!(tpl, make(<tpl-literal>, tok: make-tok(#"ident", "#f")));
  add!(tpl, make(<tpl-literal>, tok: make-tok(#"kw-end", "end")));
  values(pat, tpl)
end function;

// Build call site: unless x (foo) end
define function build-call-site ()
 => (frags :: <stretchy-vector>)
  let frags = make(<stretchy-vector>);
  add!(frags, make-token-frag(make-tok(#"ident", "unless")));
  add!(frags, make-token-frag(make-tok(#"ident", "x")));
  let paren = make(<stretchy-vector>);
  add!(paren, make-token-frag(make-tok(#"ident", "foo")));
  add!(frags, make-group-frag(#"paren", paren));
  add!(frags, make-token-frag(make-tok(#"kw-end", "end")));
  frags
end function;

// Build the fragment stream the lexer would produce for the BODY of
// `define macro unless`:
//
//   { unless ?cond:expression ?body:body end }
//     => { if (~ ?cond) ?body else #f end }
//
// Two brace groups separated by a `=>` token. Lexer convention:
// `name:` is a single `#"keyword-name"` token (so `cond:` is one token,
// not two — see Sprint 50b note above).
define function build-unless-def-body ()
 => (frags :: <stretchy-vector>)
  let frags = make(<stretchy-vector>);
  // Pattern brace: { unless ?cond:expression ?body:body end }
  let pat = make(<stretchy-vector>);
  add!(pat, make-token-frag(make-tok(#"ident", "unless")));
  add!(pat, make-token-frag(make-tok(#"punct", "?")));
  add!(pat, make-token-frag(make-tok(#"keyword-name", "cond:")));
  add!(pat, make-token-frag(make-tok(#"ident", "expression")));
  add!(pat, make-token-frag(make-tok(#"punct", "?")));
  add!(pat, make-token-frag(make-tok(#"keyword-name", "body:")));
  add!(pat, make-token-frag(make-tok(#"ident", "body")));
  add!(pat, make-token-frag(make-tok(#"kw-end", "end")));
  add!(frags, make-group-frag(#"brace", pat));
  // Arrow
  add!(frags, make-token-frag(make-tok(#"punct", "=>")));
  // Template brace: { if (~ ?cond) ?body else #f end }
  let tpl = make(<stretchy-vector>);
  add!(tpl, make-token-frag(make-tok(#"ident", "if")));
  let paren = make(<stretchy-vector>);
  add!(paren, make-token-frag(make-tok(#"punct", "~")));
  add!(paren, make-token-frag(make-tok(#"punct", "?")));
  add!(paren, make-token-frag(make-tok(#"ident", "cond")));
  add!(tpl, make-group-frag(#"paren", paren));
  add!(tpl, make-token-frag(make-tok(#"punct", "?")));
  add!(tpl, make-token-frag(make-tok(#"ident", "body")));
  add!(tpl, make-token-frag(make-tok(#"ident", "else")));
  add!(tpl, make-token-frag(make-tok(#"ident", "#f")));
  add!(tpl, make-token-frag(make-tok(#"kw-end", "end")));
  add!(frags, make-group-frag(#"brace", tpl));
  frags
end function;

define function run-match-substitute (pattern :: <stretchy-vector>,
                                      template :: <stretchy-vector>,
                                      call :: <stretchy-vector>) => ()
  let b = match-pattern(pattern, call);
  if (~ b)
    format-out("FAIL: unless pattern did not match\n");
  else
    format-out("MATCH: ok\n");
    let cond-frags = bindings-get(b, "cond");
    let body-frags = bindings-get(b, "body");
    if (cond-frags & size(cond-frags) = 1)
      format-out("BIND cond: 1 frag\n");
    else
      format-out("FAIL: cond binding shape\n");
    end;
    if (body-frags & size(body-frags) = 1)
      format-out("BIND body: 1 frag\n");
    else
      let body-size = -1;
      if (body-frags) body-size := size(body-frags); end;
      format-out("FAIL: body binding shape (size=%d)\n", body-size);
    end;
    let text = substitute(template, b);
    format-out("EXPAND: %s\n", text);
  end;
end function;

define function main () => ()
  let call = build-call-site();
  // Phase A — Sprint 50a — hand-built rule.
  format-out("PHASE: hand-built\n");
  let (pattern, template) = build-unless-rule();
  run-match-substitute(pattern, template, call);
  // Phase B — Sprint 50b — parse `define macro unless`'s body into the
  // same rule, then run the same match + substitute on the same call
  // site. The output should be byte-for-byte identical.
  format-out("PHASE: parsed-def\n");
  let body = build-unless-def-body();
  let def = parse-macro-def("unless", body);
  format-out("PARSE-DEF: ok, rules=%d\n", size(macro-def-rules(def)));
  let rule = macro-def-rules(def)[0];
  run-match-substitute(macro-rule-pattern(rule), macro-rule-template(rule), call);
end function;
