Module: dylan-lexer

// dylan-lex-shim.dylan — Sprint 51b.
//
// Bridges the Dylan-side lexer (`lex(source)` in `dylan-lexer.dylan`)
// to a Rust-aligned token stream — the same kind ordinals
// `nod_reader::token::TokenKind` uses, with the same trivia-filtering
// + preamble-skip behaviour `nod_reader::lex` applies. The wire
// contract is locked in `docs/DYLAN_TOKEN_WIRE.md`.
//
// First-cut transport (Sprint 51b v1): text on stdout, one line per
// emitted token, `KIND SPAN_LO SPAN_HI\n`. The nod-driver adapter
// spawns this EXE, pipes the source as argv[1] (file path), reads
// the stream, and reconstructs `Vec<Token>`. Once the JIT side-load
// path lands (51b-followup), this same classifier wraps into a
// `c-callable` that emits the binary 16-byte records §1 specifies;
// the classifier itself never changes.
//
// Build:
//   nod-driver build --project dylan-lex-shim.prj
//
// The two-file build (this + dylan-lexer.dylan) reuses every existing
// helper — `<token>` hierarchy, `lex()`, `%read-file`, `%argv1`, etc.
// — so adding new kinds is just a method on the generic.

// ─── token-rust-kind — generic classifier (Rust ordinals) ─────────────────
//
// Discriminants below MUST stay aligned with the `#[repr(u8)]` order of
// `TokenKind` in `src/nod-reader/src/token.rs`. The mapping table in
// `docs/DYLAN_TOKEN_WIRE.md` §3 is the human-readable reference; this
// file is the executable form.

define method token-rust-kind (t :: <token>, source :: <byte-string>)
 => (kind :: <integer>)
  // Default for unrecognised classes — surfaces as Invalid (64) so the
  // oracle test fails loudly rather than silently producing wrong codes.
  64
end method;

// — Keyword tokens. Only the three hard-reserveds map to dedicated Rust
//   kinds; every other Dylan-classified keyword falls through to Ident (0)
//   because §2.1 of `specs/01-lexer.md` says they're not lexer reserveds.
//   `#next`/`#rest`/`#key`/`#all-keys` enter `<keyword-token>` too — they
//   map to HashNext/HashRest/HashKey/HashAllKeys.
define method token-rust-kind (t :: <keyword-token>, source :: <byte-string>)
 => (kind :: <integer>)
  let kw = keyword-token-keyword(t);
  if (kw = #"define")        1   // KwDefine
  elseif (kw = #"end")       2   // KwEnd
  elseif (kw = #"otherwise") 3   // KwOtherwise
  elseif (kw = #"hash-next")     14  // HashNext
  elseif (kw = #"hash-rest")     11  // HashRest
  elseif (kw = #"hash-key")      12  // HashKey
  elseif (kw = #"hash-all-keys") 13  // HashAllKeys
  else                       0   // Ident — all other Dylan-classified keywords
  end
end method;

define method token-rust-kind (t :: <identifier-token>, source :: <byte-string>)
 => (kind :: <integer>)
  0   // Ident
end method;

define method token-rust-kind (t :: <escaped-ident-token>, source :: <byte-string>)
 => (kind :: <integer>)
  4   // EscapedIdent
end method;

define method token-rust-kind (t :: <keyword-name-token>, source :: <byte-string>)
 => (kind :: <integer>)
  21  // KeywordColon
end method;

// — Hash-prefixed literals.
define method token-rust-kind (t :: <boolean-literal-token>, source :: <byte-string>)
 => (kind :: <integer>)
  if (boolean-literal-token-value(t)) 5 else 6 end   // HashTrue / HashFalse
end method;

define method token-rust-kind (t :: <literal-sequence-open>, source :: <byte-string>)
 => (kind :: <integer>)
  7   // HashLParen — `#(`
end method;

define method token-rust-kind (t :: <literal-vector-open>, source :: <byte-string>)
 => (kind :: <integer>)
  8   // HashLBracket — `#[`
end method;

define method token-rust-kind (t :: <symbol-literal-token>, source :: <byte-string>)
 => (kind :: <integer>)
  16  // Symbol — covers both `#"foo"` and `#:foo`
end method;

define method token-rust-kind (t :: <nil-literal-token>, source :: <byte-string>)
 => (kind :: <integer>)
  // `#nil` doesn't have a dedicated Rust kind. The Rust lexer doesn't
  // recognise it — the closest analogue is Invalid (64), but the
  // Dylan-side parser does accept `#nil` as a literal. For now we emit
  // it as HashHash (10) which is also the unsupported-`##` slot; the
  // oracle test will flag this if it diverges.
  10  // HashHash
end method;

// — Numerics.
define method token-rust-kind (t :: <integer-token>, source :: <byte-string>)
 => (kind :: <integer>)
  let r = integer-token-radix(t);
  if (r = 2)       18  // IntegerBin
  elseif (r = 8)   19  // IntegerOct
  elseif (r = 16)  20  // IntegerHex
  else             22  // Integer (decimal)
  end
end method;

define method token-rust-kind (t :: <float-token>, source :: <byte-string>)
 => (kind :: <integer>)
  23  // Float
end method;

define method token-rust-kind (t :: <ratio-token>, source :: <byte-string>)
 => (kind :: <integer>)
  24  // Ratio
end method;

// — Strings + chars. v1 lumps all three Rust subkinds (String, StringMulti,
//   StringRaw) into String (25) since the Dylan lexer doesn't expose the
//   subkind on the token class. Sprint-51b-followup: peek the source bytes
//   at the span start to distinguish `"`, `"""`, `r"`.
define method token-rust-kind (t :: <string-literal-token>, source :: <byte-string>)
 => (kind :: <integer>)
  25  // String
end method;

define method token-rust-kind (t :: <character-literal-token>, source :: <byte-string>)
 => (kind :: <integer>)
  28  // Char
end method;

// — Punctuation: discriminate on form symbol. The Dylan-side `#"assign"`
//   maps to Rust's ColonEqual; everything else uses the literal name from
//   the spec's table.
define method token-rust-kind (t :: <punctuation-token>, source :: <byte-string>)
 => (kind :: <integer>)
  let f = punctuation-token-form(t);
  if (f = #"lparen")             29
  elseif (f = #"rparen")         30
  elseif (f = #"lbracket")       31
  elseif (f = #"rbracket")       32
  elseif (f = #"lbrace")         33
  elseif (f = #"rbrace")         34
  elseif (f = #"comma")          35
  elseif (f = #"semicolon")      36
  elseif (f = #"dot")            37
  elseif (f = #"ellipsis")       38
  elseif (f = #"colon")          39
  elseif (f = #"colon-colon")    40
  elseif (f = #"assign")         41  // `:=` — Dylan-side symbol predates the spec
  elseif (f = #"equal")          42
  elseif (f = #"equal-equal")    43
  elseif (f = #"arrow")          44
  elseif (f = #"tilde")          45
  elseif (f = #"tilde-equal")    46
  elseif (f = #"tilde-equal-equal") 47
  elseif (f = #"plus")           48
  elseif (f = #"minus")          49
  elseif (f = #"star")           50
  elseif (f = #"slash")          51
  elseif (f = #"caret")          52
  elseif (f = #"amp")            53
  elseif (f = #"bar")            54
  elseif (f = #"less")           55
  elseif (f = #"greater")        56
  elseif (f = #"less-equal")     57
  elseif (f = #"greater-equal")  58
  elseif (f = #"query")          59
  elseif (f = #"query-query")    60
  elseif (f = #"query-equal")    61
  elseif (f = #"query-at")       62
  elseif (f = #"hash-hash")      10  // HashHash
  elseif (f = #"hash-lbrace")     9  // HashLBrace
  else                           64  // Invalid — unrecognised form
  end
end method;

// — Trivia + end-markers.
define method token-rust-kind (t :: <comment-token>, source :: <byte-string>)
 => (kind :: <integer>)
  // Trivia; filtered out before emission. Returning Invalid here would be
  // misleading; the value should never reach the wire. Use Eof (63) as a
  // sentinel so a misuse causes the oracle test to diverge clearly.
  63
end method;

define method token-rust-kind (t :: <whitespace-token>, source :: <byte-string>)
 => (kind :: <integer>)
  63  // Same rationale as `<comment-token>`.
end method;

define method token-rust-kind (t :: <error-token>, source :: <byte-string>)
 => (kind :: <integer>)
  64  // Invalid
end method;

define method token-rust-kind (t :: <eof-token>, source :: <byte-string>)
 => (kind :: <integer>)
  63  // Eof
end method;

// ─── token-emit? — trivia + comments are dropped before printing ──────────

define method token-emit? (t :: <token>) => (yes? :: <boolean>)
  // Default — keep every concrete class not explicitly overridden below.
  #t
end method;

define method token-emit? (t :: <whitespace-token>) => (yes? :: <boolean>)
  #f
end method;

define method token-emit? (t :: <comment-token>) => (yes? :: <boolean>)
  #f
end method;

// ─── preamble-end — port of nod_reader::scan_preamble ─────────────────────
//
// Find the byte offset where the Dylan source's `Key: value` preamble
// ends, i.e. the byte just after the terminating blank line. Returns 0
// if the source does not begin with a preamble.
//
// Heuristic (matches the Rust path's effective behaviour for every
// well-formed `.dylan` file in the corpus):
//   1. Source begins with [A-Za-z_] (header key start).
//   2. The first line contains a colon before its LF.
//   3. Find `"\n\n"` (or `"\r\n\r\n"`); the preamble ends one byte past
//      the second LF.
//   4. If no blank line exists in the source, the whole file is
//      conservatively treated as preamble-free (return 0).

define function preamble-end (source :: <byte-string>) => (cursor :: <integer>)
  let n = size(source);
  if (n = 0)
    0
  else
    let b0 = %byte-string-element(source, 0);
    // Header key must start with a letter or underscore.
    let key-start? = (b0 >= 65 & b0 <= 90)   // A-Z
                       | (b0 >= 97 & b0 <= 122)  // a-z
                       | (b0 = 95);              // _
    if (~ key-start?)
      0
    else
      // Find the first LF and verify a colon precedes it.
      let i = 0;
      let line-end = -1;
      let saw-colon? = #f;
      until (i = n | line-end >= 0)
        let b = %byte-string-element(source, i);
        if (b = 10) line-end := i;
        elseif (b = 58) saw-colon? := #t;
        end;
        i := i + 1;
      end;
      if (line-end < 0 | ~ saw-colon?)
        0
      else
        // Walk forward looking for blank line.
        let j = line-end + 1;
        let result = 0;
        let done = #f;
        until (done)
          if (j >= n)
            done := #t;
          else
            let b = %byte-string-element(source, j);
            if (b = 10)
              result := j + 1;
              done := #t;
            elseif (b = 13 & j + 1 < n
                      & %byte-string-element(source, j + 1) = 10)
              // CRLF blank line — skip both bytes.
              result := j + 2;
              done := #t;
            elseif (b = 32 | b = 9)
              // Leading whitespace on the line — continuation of previous
              // header value. Skip to next LF and continue.
              until (j >= n | %byte-string-element(source, j) = 10)
                j := j + 1;
              end;
              if (j < n) j := j + 1; end;
            else
              // Non-blank line; skip past its LF and continue.
              until (j >= n | %byte-string-element(source, j) = 10)
                j := j + 1;
              end;
              if (j < n) j := j + 1; end;
            end;
          end;
        end;
        result
      end
    end
  end
end function;

// ─── emit-tokens — print kind + span for each emit-eligible token ─────────

define function emit-tokens (tokens, source :: <byte-string>) => ()
  let pre = preamble-end(source);
  let n = %stretchy-vector-size(tokens);
  let i = 0;
  until (i = n)
    let t = %stretchy-vector-element(tokens, i);
    let lo = span-start(token-span(t));
    if (token-emit?(t) & lo >= pre)
      let hi = span-end(token-span(t));
      let kind = token-rust-kind(t, source);
      format-out("%d %d %d\n", kind, lo, hi);
    end;
    i := i + 1;
  end;
end function;

// ─── dylan-lex-collect — in-process JIT side-load entry ──────────────────
//
// Sprint 51b Phase B entry. Same classification + filtering as
// `emit-tokens` but instead of writing to stdout it accumulates into a
// `<stretchy-vector>` of integers — three per emitted token:
// `kind, lo, hi`. The host (`src/nod-driver/src/dylan_lex_jit.rs`) walks
// the vector pulling out triples and reconstructs `Vec<Token>`.
//
// Why three flat ints rather than a `<list>` of triples? Stretchy-
// vectors of immediate integers are the cheapest readback shape:
// `nod_stretchy_vector_size` + `nod_stretchy_vector_element` already
// exist in the runtime ABI, and immediate-tagged integers unbox in
// O(1) on the Rust side. A list-of-triples would force three pair
// allocations per token plus a third-level structure walk.

define function dylan-lex-collect (source :: <byte-string>)
 => (records :: <object>)
  let pre = preamble-end(source);
  let tokens = lex(source);
  let records = %make-stretchy-vector(64);
  let n = %stretchy-vector-size(tokens);
  let i = 0;
  until (i = n)
    let t = %stretchy-vector-element(tokens, i);
    let lo = span-start(token-span(t));
    if (token-emit?(t) & lo >= pre)
      let hi = span-end(token-span(t));
      let kind = token-rust-kind(t, source);
      %stretchy-vector-push(records, kind);
      %stretchy-vector-push(records, lo);
      %stretchy-vector-push(records, hi);
    end;
    i := i + 1;
  end;
  records
end function;

// ─── dylan-parse-collect — Sprint 51c verify-mode entry ──────────────────
//
// Lex + parse `source` end-to-end on the Dylan side, then return the
// number of `<ast-error-node>`s in the top-level body. The host runs
// the Rust parser AND this one; agreement on the "did this source
// parse" verdict (count == 0 vs. count > 0) gates the build under
// `--verify-parse`. A nonzero divergence means one of the two parsers
// disagrees with the corpus, and we surface it loudly.
//
// Why count only top-level errors: the existing parser's error
// recovery emits `<ast-error-node>` at the constituent level when it
// bails on a definition / statement; nested errors propagate up.
// That's enough to answer the binary question "did the Dylan parser
// accept this file" — which is the contract this entry is making.
//
// Sprint 51d (deferred): a tree-shaped wire format that lets the
// Rust side actually consume the AST instead of just spot-checking
// it. This entry stays useful as the verify path even once
// replacement mode lands.

define function count-top-level-errors (body :: <ast-body>) => (n :: <integer>)
  let constituents = body-constituents(body);
  let size = %stretchy-vector-size(constituents);
  let count = 0;
  let i = 0;
  until (i = size)
    let c = %stretchy-vector-element(constituents, i);
    if (instance?(c, <ast-error-node>))
      count := count + 1;
    end;
    i := i + 1;
  end;
  count
end function;

define function dylan-parse-collect (source :: <byte-string>)
 => (error-count :: <integer>)
  let tokens = lex(source);
  let ast = parse-dylan(tokens);
  count-top-level-errors(ast)
end function;

// ─── dylan-parse-emit — Sprint 51d AST wire emitter ──────────────────────
//
// Per docs/DYLAN_AST_WIRE.md, pre-order walk of the parser's output
// emitting 4-int records into a flat stretchy-vector:
//   (kind, span_lo, span_hi, subtree_size)
//
// Sprint 51d v1 handles: Body, DefineFunction, Call, VariableRef,
// StringLit, IntegerLit, BinaryOp. Anything else lowers to Error
// (kind 7) with the span covering the unrecognised constituent.
// The host falls back to the Rust parser for the whole file on Error.

define constant $ast-kind-body            = 0;
define constant $ast-kind-define-function = 1;
define constant $ast-kind-call            = 2;
define constant $ast-kind-variable-ref    = 3;
define constant $ast-kind-string-lit      = 4;
define constant $ast-kind-integer-lit     = 5;
define constant $ast-kind-binary-op       = 6;
define constant $ast-kind-error           = 7;

// Emit one record (kind, lo, hi, subtree_size). subtree_size patched
// later — initial push is 1 (just self).
define function emit-record (out :: <stretchy-vector>,
                             kind :: <integer>,
                             lo :: <integer>,
                             hi :: <integer>)
 => (record-index :: <integer>)
  let idx = %stretchy-vector-size(out);
  %stretchy-vector-push(out, kind);
  %stretchy-vector-push(out, lo);
  %stretchy-vector-push(out, hi);
  %stretchy-vector-push(out, 1);   // subtree_size placeholder
  idx
end function;

// After children are emitted, patch the subtree_size = (current_size
// - record_index) / 4.
define function patch-subtree-size (out :: <stretchy-vector>,
                                    record-index :: <integer>)
 => ()
  let total-ints = %stretchy-vector-size(out);
  let subtree-records = (total-ints - record-index) / 4;
  %stretchy-vector-element-setter(subtree-records, out, record-index + 3);
end function;

define function span-of (node :: <ast-node>) => (lo :: <integer>, hi :: <integer>)
  let tok = node-token(node);
  if (instance?(tok, <token>))
    let s = token-span(tok);
    values(span-start(s), span-end(s))
  else
    values(0, 0)
  end
end function;

// Sprint 51e — span backfill. Container nodes (<ast-body>, <ast-call>,
// <ast-binary-op>) carry no leading <token>, so `span-of` returns
// (0,0) for them. After a container's children have been emitted, this
// recovers the container's span as the union of its descendants'
// spans. The walk is bottom-up: each child's own `emit-node` already
// backfilled it before we patch the parent, so descendant spans are
// final by the time we read them here. Only fires when the node's own
// span is empty — a real token-derived span is never overwritten.
define function backfill-span-from-children (out :: <stretchy-vector>,
                                             idx :: <integer>) => ()
  let cur-lo = %stretchy-vector-element(out, idx + 1);
  let cur-hi = %stretchy-vector-element(out, idx + 2);
  if (cur-lo = 0 & cur-hi = 0)
    let total = %stretchy-vector-size(out);
    let min-lo = 0;
    let max-hi = 0;
    let seen = #f;
    let i = idx + 4;
    until (i >= total)
      let lo = %stretchy-vector-element(out, i + 1);
      let hi = %stretchy-vector-element(out, i + 2);
      // A real span always has hi > lo >= 0, so hi > 0 ⟺ spanned;
      // (0,0) is the unspanned marker. Positive condition avoids an
      // empty `if` branch (the lowerer rejects empty `begin` blocks).
      if (hi > 0)
        if (seen = #f | lo < min-lo) min-lo := lo end;
        if (hi > max-hi) max-hi := hi end;
        seen := #t;
      end;
      i := i + 4;
    end;
    if (seen)
      %stretchy-vector-element-setter(min-lo, out, idx + 1);
      %stretchy-vector-element-setter(max-hi, out, idx + 2);
    end;
  end;
end function;

// Forward declared via define generic semantics — each method below
// emits one record (plus children) and returns nothing. The caller is
// responsible for patching the parent's subtree size if it cares.

define method emit-node (node :: <ast-node>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(node);
  emit-record(out, $ast-kind-error, lo, hi);
end method;

define method emit-node (b :: <ast-body>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(b);
  let idx = emit-record(out, $ast-kind-body, lo, hi);
  let constituents = body-constituents(b);
  let n = %stretchy-vector-size(constituents);
  let i = 0;
  until (i = n)
    let c = %stretchy-vector-element(constituents, i);
    emit-node(c, source, out);
    i := i + 1;
  end;
  backfill-span-from-children(out, idx);
  patch-subtree-size(out, idx);
end method;

define method emit-node (d :: <ast-body-definition>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  // v1 only handles `define function`. Anything else is Error.
  let word-tok = defn-word(d);
  let word-name = token-name(word-tok);
  if (word-name = "function")
    let word-span = token-span(word-tok);
    let lo = span-start(word-span);
    let hi = span-end(word-span);
    // Stretch the span to cover the whole definition (best-effort:
    // use the body's end if present).
    let body = defn-body(d);
    let (body-lo, body-hi) = span-of(body);
    let outer-hi = if (body-hi > hi) body-hi else hi end;
    let body-lo-unused = body-lo; // discard explicitly
    let idx = emit-record(out, $ast-kind-define-function, lo, outer-hi);
    emit-node(body, source, out);
    patch-subtree-size(out, idx);
  else
    let (lo, hi) = span-of(d);
    emit-record(out, $ast-kind-error, lo, hi);
  end
end method;

define method emit-node (c :: <ast-call>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(c);
  let idx = emit-record(out, $ast-kind-call, lo, hi);
  // First child: callee.
  emit-node(call-fn(c), source, out);
  // Remaining children: each arg.
  let args = call-args(c);
  let n = %stretchy-vector-size(args);
  let i = 0;
  until (i = n)
    let a = %stretchy-vector-element(args, i);
    emit-node(a, source, out);
    i := i + 1;
  end;
  backfill-span-from-children(out, idx);
  patch-subtree-size(out, idx);
end method;

define method emit-node (v :: <ast-variable-ref>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let tok = varref-tok(v);
  let s = token-span(tok);
  let lo = span-start(s);
  let hi = span-end(s);
  emit-record(out, $ast-kind-variable-ref, lo, hi);
end method;

define method emit-node (s :: <ast-string-lit>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(s);
  emit-record(out, $ast-kind-string-lit, lo, hi);
end method;

define method emit-node (i :: <ast-integer-lit>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(i);
  emit-record(out, $ast-kind-integer-lit, lo, hi);
end method;

define method emit-node (b :: <ast-binary-op>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(b);
  let idx = emit-record(out, $ast-kind-binary-op, lo, hi);
  emit-node(binop-left(b), source, out);
  emit-node(binop-right(b), source, out);
  backfill-span-from-children(out, idx);
  patch-subtree-size(out, idx);
end method;

define method emit-node (e :: <ast-error-node>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  let (lo, hi) = span-of(e);
  emit-record(out, $ast-kind-error, lo, hi);
end method;

// <ast-pos-arg> is the parser's wrapper for "this is a positional
// call argument." Wire-format-wise it's transparent — we don't emit
// a record for it, just recurse into the wrapped value. That keeps
// the host's tree free of a wrapper kind that wouldn't translate to
// anything in `ast::Expr`.
define method emit-node (p :: <ast-pos-arg>, source :: <byte-string>,
                         out :: <stretchy-vector>) => ()
  emit-node(pos-arg-value(p), source, out);
end method;

define function dylan-parse-emit (source :: <byte-string>)
 => (records :: <object>)
  let tokens = lex(source);
  let ast = parse-dylan(tokens);
  let out = %make-stretchy-vector(64);
  emit-node(ast, source, out);
  out
end function;

// ─── main — read argv[1] as a path, lex, emit ────────────────────────────

define function shim-main () => ()
  let path = %argv1();
  if (empty?(path))
    format-out("dylan-lex-shim: missing input path\n");
  else
    let source = load-source-via-rope(path);
    if (empty?(source))
      format-out("dylan-lex-shim: could not read %s\n", path);
    else
      let tokens = lex(source);
      emit-tokens(tokens, source);
    end;
  end;
end function shim-main;
