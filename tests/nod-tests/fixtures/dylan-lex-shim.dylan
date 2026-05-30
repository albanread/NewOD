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
