Module: dylan-lexer

// Sprint 45a — Dylan lexer in Dylan, scaffolding phase.
//
// What lives here:
//   * `<span>` — start/end byte offsets into the source buffer.
//   * `<token>` and its concrete subclasses — the full hierarchy from
//     §2.2 of `docs/SPRINT_45_DYLAN_LEXER.md`. Each token is a class,
//     not an enum tag; everything dispatches on token class via generic
//     methods (`print-token`, `colour-of`, `token-source-text`, …) so
//     consumer code never writes a giant `select (kind)`.
//   * `dump-tokens(tokens, source) => <byte-string>` — the canonical
//     textual representation, locked in as the oracle-test contract
//     for sprint 45d.
//   * `lex(source) => <stretchy-vector>` — STUB for sprint 45a; returns
//     a one-element vector holding a single `<eof-token>` at offset 0.
//     Sprint 45b fills out the real implementation.
//   * A tiny `main` stub used by the `nod-driver dump-dylan-tokens`
//     subcommand. Reads argv[1], lexes it, prints the dump.
//
// The file knows NOTHING about the IDE. Sprint 45e is the IDE-side
// consumer that imports from this file via `colour-of` and the token
// hierarchy.

// ─── <span> — byte-offset range into a source buffer ──────────────────────

define class <span> (<object>)
  slot span-start :: <integer>, init-keyword: start:;
  slot span-end   :: <integer>, init-keyword: end:;
end class;

// `copy-sequence` on `<byte-string>` is positional (`s, start, stop`),
// not keyword — Sprint 42a's stdlib hasn't grown the keyword surface yet.
define method span-text (span :: <span>, source :: <byte-string>)
 => (text :: <byte-string>)
  copy-sequence(source, span-start(span), span-end(span))
end method;

define method span-contains? (span :: <span>, offset :: <integer>)
 => (yes? :: <boolean>)
  offset >= span-start(span) & offset < span-end(span)
end method;

// ─── <token> — abstract base ──────────────────────────────────────────────
//
// Every concrete token carries a `<span>` plus whatever extra slots its
// class needs. The hierarchy is FLAT in the sense that consumers never
// special-case it via subclass instanceof checks; they call the generic
// methods declared at the bottom of this section.

define class <token> (<object>)
  slot token-span :: <span>, init-keyword: span:;
end class;

// Concrete tokens. Slot lists mirror §2.2 of the design doc.

define class <keyword-token> (<token>)
  slot keyword-token-keyword :: <symbol>, init-keyword: keyword:;
end class;

define class <identifier-token> (<token>)
  slot identifier-token-name :: <byte-string>, init-keyword: name:;
end class;

define class <keyword-name-token> (<token>)
  slot keyword-name-token-name :: <byte-string>, init-keyword: name:;
end class;

// `<number-token>` is the abstract intermediate; never instantiated.
define class <number-token> (<token>) end class;

define class <integer-token> (<number-token>)
  slot integer-token-value :: <integer>, init-keyword: value:;
  slot integer-token-radix :: <integer>, init-keyword: radix:;
end class;

define class <float-token> (<number-token>)
  // Sprint 45a stores the raw text; sprint 45b can add a decoded
  // value slot once we have <float> / <double-float> in the runtime.
  slot float-token-raw-text :: <byte-string>, init-keyword: raw-text:;
end class;

define class <string-literal-token> (<token>)
  slot string-literal-token-raw-text :: <byte-string>, init-keyword: raw-text:;
  slot string-literal-token-decoded  :: <byte-string>, init-keyword: decoded:;
end class;

define class <character-literal-token> (<token>)
  slot character-literal-token-codepoint :: <integer>, init-keyword: codepoint:;
end class;

define class <symbol-literal-token> (<token>)
  slot symbol-literal-token-name :: <byte-string>, init-keyword: name:;
end class;

define class <boolean-literal-token> (<token>)
  slot boolean-literal-token-value :: <boolean>, init-keyword: value:;
end class;

define class <nil-literal-token> (<token>) end class;

define class <literal-vector-open>   (<token>) end class;
define class <literal-sequence-open> (<token>) end class;

define class <punctuation-token> (<token>)
  slot punctuation-token-form :: <symbol>, init-keyword: form:;
end class;

// Comments carry their text plus a flag distinguishing `//` (line)
// from `/* */` (block). Sprint 45a uses the flag only for `dump-tokens`
// kind discrimination (COMMENT_LINE vs COMMENT_BLOCK); 45e tunes the
// colouring per kind.
define class <comment-token> (<token>)
  slot comment-token-text     :: <byte-string>, init-keyword: text:;
  slot comment-token-is-block? :: <boolean>,    init-keyword: is-block?:;
end class;

define class <whitespace-token> (<token>) end class;

define class <error-token> (<token>)
  slot error-token-message :: <byte-string>, init-keyword: message:;
end class;

define class <eof-token> (<token>) end class;

// ─── colour-of — RGB integer per token class ──────────────────────────────
//
// Constants for now; Sprint 45e tunes them for the IDE palette. Encoded
// as a 24-bit RGB integer (red << 16 | green << 8 | blue) so consumers
// can mask out channels with `/` and `mod` arithmetic. Whitespace
// colours to white (invisible against the white background); the IDE
// special-cases it anyway.

// RGB colours expressed as decimal — the Sprint 02 lexer hasn't
// taught the front-end the `16#RRGGBB` literal form yet (that's a
// Sprint 45b/c follow-up since our own lexer learns the same syntax
// then). Each comment notes the hex equivalent so the values are
// easy to cross-check against an editor palette.

define method colour-of (t :: <keyword-token>) => (rgb :: <integer>)
  255                              // 0x0000FF — blue
end method;

define method colour-of (t :: <identifier-token>) => (rgb :: <integer>)
  0                                // 0x000000 — black
end method;

define method colour-of (t :: <keyword-name-token>) => (rgb :: <integer>)
  128                              // 0x000080 — navy
end method;

define method colour-of (t :: <integer-token>) => (rgb :: <integer>)
  8388736                          // 0x800080 — purple
end method;

define method colour-of (t :: <float-token>) => (rgb :: <integer>)
  8388736                          // 0x800080 — purple
end method;

define method colour-of (t :: <string-literal-token>) => (rgb :: <integer>)
  16711680                         // 0xFF0000 — red
end method;

define method colour-of (t :: <character-literal-token>) => (rgb :: <integer>)
  16711680                         // 0xFF0000 — red
end method;

define method colour-of (t :: <symbol-literal-token>) => (rgb :: <integer>)
  16711680                         // 0xFF0000 — red
end method;

define method colour-of (t :: <boolean-literal-token>) => (rgb :: <integer>)
  8388736                          // 0x800080 — purple
end method;

define method colour-of (t :: <nil-literal-token>) => (rgb :: <integer>)
  8388736                          // 0x800080 — purple
end method;

define method colour-of (t :: <literal-vector-open>) => (rgb :: <integer>)
  8421504                          // 0x808080 — grey
end method;

define method colour-of (t :: <literal-sequence-open>) => (rgb :: <integer>)
  8421504                          // 0x808080 — grey
end method;

define method colour-of (t :: <punctuation-token>) => (rgb :: <integer>)
  0                                // 0x000000 — black
end method;

define method colour-of (t :: <comment-token>) => (rgb :: <integer>)
  32768                            // 0x008000 — green
end method;

define method colour-of (t :: <whitespace-token>) => (rgb :: <integer>)
  16777215                         // 0xFFFFFF — white (invisible)
end method;

define method colour-of (t :: <error-token>) => (rgb :: <integer>)
  16711680                         // 0xFF0000 — red
end method;

define method colour-of (t :: <eof-token>) => (rgb :: <integer>)
  0                                // 0x000000 — black
end method;

// ─── token-kind-name — uppercase tag for dump-tokens ──────────────────────
//
// The canonical dump format uses an uppercase kind tag without the
// angle-brackets or `-token` suffix. Lives here as a generic so adding
// a new token class only takes one method, never a giant `select`.

define method token-kind-name (t :: <keyword-token>) => (s :: <byte-string>)
  "KEYWORD"
end method;

define method token-kind-name (t :: <identifier-token>) => (s :: <byte-string>)
  "IDENTIFIER"
end method;

define method token-kind-name (t :: <keyword-name-token>) => (s :: <byte-string>)
  "KEYWORD_NAME"
end method;

define method token-kind-name (t :: <integer-token>) => (s :: <byte-string>)
  "INTEGER"
end method;

define method token-kind-name (t :: <float-token>) => (s :: <byte-string>)
  "FLOAT"
end method;

define method token-kind-name (t :: <string-literal-token>) => (s :: <byte-string>)
  "STRING"
end method;

define method token-kind-name (t :: <character-literal-token>) => (s :: <byte-string>)
  "CHAR"
end method;

define method token-kind-name (t :: <symbol-literal-token>) => (s :: <byte-string>)
  "SYMBOL"
end method;

define method token-kind-name (t :: <boolean-literal-token>) => (s :: <byte-string>)
  "BOOLEAN"
end method;

define method token-kind-name (t :: <nil-literal-token>) => (s :: <byte-string>)
  "NIL"
end method;

define method token-kind-name (t :: <literal-vector-open>) => (s :: <byte-string>)
  "LIT_VEC_OPEN"
end method;

define method token-kind-name (t :: <literal-sequence-open>) => (s :: <byte-string>)
  "LIT_SEQ_OPEN"
end method;

define method token-kind-name (t :: <punctuation-token>) => (s :: <byte-string>)
  "PUNCT"
end method;

// Comments distinguish line vs block via the `is-block?` slot so
// dump consumers see a stable two-token vocabulary.
define method token-kind-name (t :: <comment-token>) => (s :: <byte-string>)
  if (comment-token-is-block?(t)) "COMMENT_BLOCK" else "COMMENT_LINE" end
end method;

define method token-kind-name (t :: <whitespace-token>) => (s :: <byte-string>)
  "WS"
end method;

define method token-kind-name (t :: <error-token>) => (s :: <byte-string>)
  "ERROR"
end method;

define method token-kind-name (t :: <eof-token>) => (s :: <byte-string>)
  "EOF"
end method;

// ─── token-source-text — span-text wrapper ────────────────────────────────
//
// Generic so future token classes can override (e.g. a synthesised
// `<error-token>` whose message isn't a substring of `source`). The
// default just slices the span out of the source buffer.

define method token-source-text (t :: <token>, source :: <byte-string>)
 => (text :: <byte-string>)
  span-text(token-span(t), source)
end method;

// ─── print-token — write one canonical dump line for a token ──────────────
//
// One canonical line per token; fields separated by EXACTLY two spaces:
//
//   <start-line>:<start-col>-<end-line>:<end-col>  <KIND>  <escaped-text>
//
// EOF tokens stop after the KIND tag (no source text to show). The
// trailing newline is added by `dump-tokens`, not here.
//
// Stream-based: writes directly into the caller's `<string-stream>`
// accumulator rather than returning a freshly-allocated byte-string per
// token (the GAP-001-pre shape was O(N²) on the whole-buffer dump).
// GAP-001 (`a689fcd`) lit up the stream surface; this method is the
// first real consumer.

// GAP-007 workaround: this method ignores the `stream` parameter and
// writes to the module-variable `*dump-stream*` instead. The variable
// lives in a cell-backed slot registered as a GC root, so it survives
// the many allocations that happen inside `nod-int-to-string`,
// `write-string`, and `write-escaped-source-text`. (The function-arg
// form clobbers around the 92nd iteration of `dump-tokens`.) Callers
// MUST set `*dump-stream*` to a fresh string-stream before calling.
define method print-token
    (t :: <token>, source :: <byte-string>, stream :: <string-stream>)
 => ()
  let span = token-span(t);
  let start-packed = offset-to-line-col-packed(source, span-start(span));
  let end-packed   = offset-to-line-col-packed(source, span-end(span));
  write-line-col-to-dump-stream(unpack-line(start-packed), unpack-col(start-packed));
  write-byte(*dump-stream*, 45);  // '-'
  write-line-col-to-dump-stream(unpack-line(end-packed),   unpack-col(end-packed));
  write-string(*dump-stream*, "  ");
  write-string(*dump-stream*, token-kind-name(t));
  if (~instance?(t, <eof-token>))
    write-string(*dump-stream*, "  ");
    write-escaped-source-text-to-dump-stream(token-source-text(t, source));
  end;
end method;

// ─── write-line-col — small helper: `<line>:<col>` into a stream ─────────

define function write-line-col
    (stream :: <string-stream>, line :: <integer>, col :: <integer>) => ()
  write-string(stream, nod-int-to-string(line));
  write-byte(stream, 58);  // ':'
  write-string(stream, nod-int-to-string(col));
end function;

// GAP-007 workaround variant: writes to `*dump-stream*` so the stream
// reference lives in a GC-root cell, not a function-arg slot that can
// go stale across the int-to-string allocation.
define function write-line-col-to-dump-stream
    (line :: <integer>, col :: <integer>) => ()
  write-string(*dump-stream*, nod-int-to-string(line));
  write-byte(*dump-stream*, 58);  // ':'
  write-string(*dump-stream*, nod-int-to-string(col));
end function;

// ─── nod-int-to-string — local digit formatter ────────────────────────────
//
// The line-numbers gutter in `ide_syntax.dylan` already has an
// `integer-to-string` — we copy that body here (under a `nod-`
// prefix to avoid clashing with any future stdlib generic) so the
// lexer file stays self-contained. Sprint 45c will lift this to a
// stdlib helper alongside the character predicates.

define function nod-int-to-string (n :: <integer>) => (s :: <byte-string>)
  if (n = 0)
    "0"
  else
    let m = n;
    let digits = 0;
    until (m = 0)
      digits := digits + 1;
      m := m / 10;
    end;
    let s = %byte-string-allocate(digits);
    let m = n;
    let i = digits - 1;
    let done = #f;
    until (done)
      if (i < 0)
        done := #t;
      else
        let d = m - (m / 10) * 10;
        %byte-string-element-setter(48 + d, s, i);
        m := m / 10;
        i := i - 1;
      end;
    end;
    s
  end
end function;

// ─── offset-to-line-col ───────────────────────────────────────────────────
//
// Walk the source bytes up to `offset` counting line breaks. Lines and
// columns are 1-indexed. Newline = byte 10; carriage returns inside
// `\r\n` count as column-bumps only (the LF advances the line) — for
// Sprint 45a's hello.dylan the input is LF-only so the simple form is
// enough. Sprint 45b will revisit if/when we hit CRLF fixtures.
//
// Returns `line * 1_000_000 + col`. The Sprint 06 sema kernel doesn't
// lower `values(a, b)` / multi-binder `let (a, b) =` yet (see
// nod-sema/src/lib.rs §"Out of scope"), so we pack the two integers
// into one. Callers that just want the line use `/ 1_000_000`; column
// is `mod 1_000_000`. The cap is enforced implicitly: any column
// >= 1_000_000 would collide, but no source file we'll ever lex has
// a single line that long.

// Packing scale: line * $line-col-shift + col. GAP-002 is fixed —
// `define constant` names now resolve from inside function bodies,
// so we use the named constant at all three sites. (Until GAP-003
// lands proper multi-value return, the packing trick stays.)

define constant $line-col-shift = 1000000;

define function offset-to-line-col-packed
    (source :: <byte-string>, offset :: <integer>) => (packed :: <integer>)
  let n = %byte-string-size(source);
  let stop = if (offset > n) n elseif (offset < 0) 0 else offset end;
  let line = 1;
  let col = 1;
  let i = 0;
  // Shaped to mirror `count-newlines-in` in ide_rope.dylan: the `else`
  // arm of every assignment-flavoured `if` returns `#f` so the loop
  // body's join point sees no SSA disagreement (the Sprint 42-pre
  // fix to `lower_if` only catches cases where one arm assigns and
  // the other arm has a meaningful value).
  until (i = stop)
    let b = %byte-string-element(source, i);
    if (b = 10)
      line := line + 1;
      col := 1;
    else
      col := col + 1;
    end;
    i := i + 1;
  end;
  line * $line-col-shift + col
end function;

define function unpack-line (packed :: <integer>) => (line :: <integer>)
  packed / $line-col-shift
end function;

define function unpack-col (packed :: <integer>) => (col :: <integer>)
  packed - (packed / $line-col-shift) * $line-col-shift
end function;

// ─── write-escaped-source-text — escape control bytes into a stream ──────
//
// Replace control bytes and quote/backslash with their canonical dump
// escapes, writing directly into the caller's stream:
//   * byte 10  (LF)  → `\n`   (two characters: backslash + 'n')
//   * byte 9   (TAB) → `\t`
//   * byte 92  (`\`) → `\\`
//   * byte 34  (`"`) → `\"`
//   * byte 32  (` `) → `\s`   (so whitespace runs are visible)
//   * other bytes pass through unchanged — Sprint 45a doesn't bother
//     with hex escapes; the corpus we care about is LF-only.
//
// Pre-GAP-001 this allocated a fresh byte-string per byte (concatenate-
// as-you-go to dodge Sprint 42-pre's `lower_if` SSA-join bug). The
// stream-flavour writes single bytes via `write-byte` and 2-byte
// escapes via `write-string` of a literal — no `acc := concatenate(...)`
// chain, no O(N²) blow-up, no SSA-join trip-wire.

define function write-escaped-source-text
    (stream :: <string-stream>, s :: <byte-string>) => ()
  let n = %byte-string-size(s);
  let i = 0;
  until (i = n)
    let b = %byte-string-element(s, i);
    if (b = 10)
      write-string(stream, "\\n");
    elseif (b = 9)
      write-string(stream, "\\t");
    elseif (b = 92)
      write-string(stream, "\\\\");
    elseif (b = 34)
      write-string(stream, "\\\"");
    elseif (b = 32)
      write-string(stream, "\\s");
    else
      write-byte(stream, b);
    end;
    i := i + 1;
  end;
end function;

// GAP-007 workaround variant: writes to `*dump-stream*` directly.
define function write-escaped-source-text-to-dump-stream
    (s :: <byte-string>) => ()
  let n = %byte-string-size(s);
  let i = 0;
  until (i = n)
    let b = %byte-string-element(s, i);
    if (b = 10)
      write-string(*dump-stream*, "\\n");
    elseif (b = 9)
      write-string(*dump-stream*, "\\t");
    elseif (b = 92)
      write-string(*dump-stream*, "\\\\");
    elseif (b = 34)
      write-string(*dump-stream*, "\\\"");
    elseif (b = 32)
      write-string(*dump-stream*, "\\s");
    else
      write-byte(*dump-stream*, b);
    end;
    i := i + 1;
  end;
end function;

// ─── dump-tokens ──────────────────────────────────────────────────────────
//
// Build the whole-buffer dump. Allocates ONE `<string-stream>` accumulator,
// walks the token vector calling `print-token` on each (which writes the
// canonical line into the stream), then materialises the stream as a
// `<byte-string>` once at the end.
//
// Pre-GAP-001 this was the O(N²) site — every token allocated a fresh
// dump-line byte-string, every iteration concatenated it onto a growing
// accumulator (allocating a fresh result). With the stream surface, the
// only allocations are (a) the stream's own stretchy-vector growth and
// (b) the final `as-byte-string` materialisation.

// Per-token line build — returns the canonical dump line for ONE
// token as a freshly-allocated byte-string, with no trailing newline.
// Allocates a fresh stream into the *dump-stream* module variable so
// the GC's root-tracking of the variable cell keeps the stream live
// across the many allocations that happen inside `print-token` itself.
// See GAP-007.
define function print-token-to-string
    (t :: <token>, source :: <byte-string>) => (line :: <byte-string>)
  *dump-stream* := make-string-stream();
  print-token(t, source, *dump-stream*);
  as-byte-string(*dump-stream*)
end function;

// Dump the token vector. Concatenate per-token lines using
// `acc := concatenate(acc, …)` — the IDE syntax fixture's
// `build-line-numbers-block` uses the same shape and works through
// thousands of tokens. See GAP-007 for the stream-local clobber that
// pushed us off the stream-streaming form.
//
// GAP-007 workaround: reads from the `*tokens*` module variable rather
// than the `tokens` parameter so the vector stays reachable through
// the heavy per-iteration allocation in `print-token-to-string`. The
// caller MUST set `*tokens*` before calling (the `lex` function does
// this on every invocation).
define function dump-tokens
    (tokens, source :: <byte-string>) => (text :: <byte-string>)
  *tokens* := tokens;
  let n = %stretchy-vector-size(*tokens*);
  let acc = "";
  let i = 0;
  until (i = n)
    let t = %stretchy-vector-element(*tokens*, i);
    let line = print-token-to-string(t, source);
    acc := concatenate(acc, line);
    acc := concatenate(acc, "\n");
    i := i + 1;
  end;
  acc
end function;

// ─── lex — Sprint 45b real implementation ─────────────────────────────────
//
// Strategy:
//   * Module-level `*src*` + `*pos*` variables hold the immutable source
//     buffer and the moving byte cursor. GAP-004 (define variable) is
//     fixed, so the cursor can be a true mutable scalar.
//   * `lex(source)` resets the variables, then loops calling
//     `next-token()` until an `<eof-token>` is appended.
//   * Each scanner consumes ≥ 1 byte even on malformed input (the
//     `<error-token>` recovery path) so the loop is guaranteed to
//     terminate after at most `size(source) + 1` iterations.
//   * Lossless: whitespace and comments come back as first-class
//     tokens, one per run. The parser will use `non-trivia-tokens` to
//     skip them (a sprint-46 helper).
//
// Open questions from §9 of SPRINT_45_DYLAN_LEXER.md, settled here:
//   * Negative integers lex as `-` + digits (two tokens). Parser folds.
//   * `/* … */` block comments DO NOT nest. First `*/` closes them.
//   * `<error-token>` always carries an explanatory `error-message` and
//     advances pos by at least 1 byte.
//
// Things deliberately NOT covered in 45b (queued for follow-ups):
//   * Float literals (`3.14`, `1.0e-3`). The token class exists but
//     `lex` does not produce it yet; SPRINT_45_DYLAN_LEXER.md §3 marks
//     floats as nice-to-have.
//   * Header preambles (`Module: foo`, `Author: bar`). The Rust lexer
//     skips them before scanning; we lex them as ordinary identifiers
//     plus a trailing `:` (`<keyword-name-token>`). The 45d oracle
//     will document any disagreement.
//   * Triple-quoted strings, raw-string `#r"..."`, ratio numerics,
//     hex `\<HHHH>` char escapes, leading-dot floats — all deferred to
//     follow-up sprints with their own tests.

define variable *src* :: <byte-string> = "";
define variable *pos* :: <integer> = 0;
// GAP-007 workaround: also stash the in-progress token vector as a
// module variable so it lives in a `<cell>` slot (registered as a GC
// root). The function-local form clobbers around the 1000th token under
// heavy allocation pressure.
define variable *tokens* :: <object> = #f;
// Same GAP-007 workaround on the dump side. The dump-token stream
// is the only one we ever materialise in this file; stashing it in
// a module-variable cell keeps it reachable across the many
// allocations inside `print-token`.
define variable *dump-stream* :: <object> = #f;

// ─── tiny cursor helpers ──────────────────────────────────────────────────

define function at-end? () => (yes? :: <boolean>)
  *pos* >= %byte-string-size(*src*)
end function;

// `peek-at(off)` returns the byte at `*pos* + off` or -1 when past end.
// Using -1 as the EOF sentinel keeps every classification predicate
// pure-integer; no token-stream code ever pattern-matches on it.

define function peek-at (off :: <integer>) => (b :: <integer>)
  let i = *pos* + off;
  if (i >= 0 & i < %byte-string-size(*src*))
    %byte-string-element(*src*, i)
  else
    -1
  end
end function;

define function current-byte () => (b :: <integer>)
  peek-at(0)
end function;

define function advance (n :: <integer>) => ()
  *pos* := *pos* + n;
end function;

// ─── character classification ─────────────────────────────────────────────
//
// Dylan identifier alphabet (mirrors `is_ident_start` /
// `is_ident_continue` in `src/nod-reader/src/lexer.rs`). For 45c these
// lift into stdlib character predicates; the inline form is fine for
// 45b and lets the lexer stay self-contained.

define function is-ascii-digit? (b :: <integer>) => (yes? :: <boolean>)
  b >= 48 & b <= 57           // '0'..'9'
end function;

define function is-ascii-alpha? (b :: <integer>) => (yes? :: <boolean>)
  (b >= 65 & b <= 90) | (b >= 97 & b <= 122)   // A..Z | a..z
end function;

define function is-bin-digit? (b :: <integer>) => (yes? :: <boolean>)
  b = 48 | b = 49             // '0' | '1'
end function;

define function is-oct-digit? (b :: <integer>) => (yes? :: <boolean>)
  b >= 48 & b <= 55           // '0'..'7'
end function;

define function is-hex-digit? (b :: <integer>) => (yes? :: <boolean>)
  is-ascii-digit?(b)
    | (b >= 65 & b <= 70)     // 'A'..'F'
    | (b >= 97 & b <= 102)    // 'a'..'f'
end function;

// Dylan's "name-start" alphabet: letters plus the punctuation graphics
// allowed at the head of an identifier. Note `-` is NOT in the start
// set (so `-7` lexes as `-` + `7`).
define function is-name-start? (b :: <integer>) => (yes? :: <boolean>)
  is-ascii-alpha?(b)
    | b = 95   // '_'
    | b = 33   // '!'
    | b = 36   // '$'
    | b = 37   // '%'
    | b = 38   // '&'
    | b = 42   // '*'
    | b = 60   // '<'
    | b = 62   // '>'
    | b = 94   // '^'
    | b = 124  // '|'
    | b = 126  // '~'
end function;

// Name-continuation also accepts digits, `?`, `-`, `+`, `=`, `/`.
define function is-name-cont? (b :: <integer>) => (yes? :: <boolean>)
  is-name-start?(b)
    | is-ascii-digit?(b)
    | b = 45   // '-'
    | b = 43   // '+'
    | b = 61   // '='
    | b = 47   // '/'
    | b = 63   // '?'
end function;

// Whitespace bytes treated as a single run. Newline (10) is included;
// the line/col packing in `offset-to-line-col-packed` separately tracks
// line breaks.
define function is-whitespace-byte? (b :: <integer>) => (yes? :: <boolean>)
  b = 32 | b = 9 | b = 10 | b = 13 | b = 12  // ' ' \t \n \r \f
end function;

// ─── identifier classification: keyword vs ordinary ───────────────────────
//
// Dylan has a fairly long keyword list. Rather than allocating a hash
// table at lex-time we just compare against the literal strings via the
// stdlib `=` method on `<byte-string>` (Sprint 42a). One comparison per
// candidate keyword; for the typical token-stream this is a few hundred
// nanoseconds total per identifier. If profiling ever flags this hot,
// a perfect-hash table is a follow-up sprint.

define function classify-keyword (name :: <byte-string>)
 => (kw :: <object>)   // either a <symbol> on match or #f on miss
  if (name = "define") #"define"
  elseif (name = "end") #"end"
  elseif (name = "let") #"let"
  elseif (name = "local") #"local"
  elseif (name = "if") #"if"
  elseif (name = "else") #"else"
  elseif (name = "elseif") #"elseif"
  elseif (name = "then") #"then"
  elseif (name = "begin") #"begin"
  elseif (name = "method") #"method"
  elseif (name = "function") #"function"
  elseif (name = "class") #"class"
  elseif (name = "module") #"module"
  elseif (name = "library") #"library"
  elseif (name = "use") #"use"
  elseif (name = "export") #"export"
  elseif (name = "import") #"import"
  elseif (name = "constant") #"constant"
  elseif (name = "variable") #"variable"
  elseif (name = "slot") #"slot"
  elseif (name = "make") #"make"
  elseif (name = "instance?") #"instance?"
  elseif (name = "singleton") #"singleton"
  elseif (name = "inherited") #"inherited"
  elseif (name = "next") #"next"
  elseif (name = "signal") #"signal"
  elseif (name = "condition") #"condition"
  elseif (name = "block") #"block"
  elseif (name = "cleanup") #"cleanup"
  elseif (name = "exception") #"exception"
  elseif (name = "select") #"select"
  elseif (name = "case") #"case"
  elseif (name = "cond") #"cond"
  elseif (name = "unless") #"unless"
  elseif (name = "while") #"while"
  elseif (name = "until") #"until"
  elseif (name = "for") #"for"
  elseif (name = "from") #"from"
  elseif (name = "to") #"to"
  elseif (name = "by") #"by"
  elseif (name = "in") #"in"
  elseif (name = "handler") #"handler"
  elseif (name = "generic") #"generic"
  elseif (name = "domain") #"domain"
  elseif (name = "sealed") #"sealed"
  elseif (name = "open") #"open"
  elseif (name = "abstract") #"abstract"
  elseif (name = "concrete") #"concrete"
  elseif (name = "primary") #"primary"
  elseif (name = "free") #"free"
  elseif (name = "virtual") #"virtual"
  elseif (name = "each-subclass") #"each-subclass"
  elseif (name = "required-init-keyword") #"required-init-keyword"
  elseif (name = "init-keyword") #"init-keyword"
  elseif (name = "init-value") #"init-value"
  elseif (name = "init-function") #"init-function"
  elseif (name = "setter") #"setter"
  elseif (name = "getter") #"getter"
  elseif (name = "type") #"type"
  elseif (name = "subclass") #"subclass"
  elseif (name = "super") #"super"
  elseif (name = "next-method") #"next-method"
  else
    #f
  end
end function;

// ─── span construction + small wrappers ───────────────────────────────────

define function span-here (lo :: <integer>) => (s :: <span>)
  make(<span>, start: lo, end: *pos*)
end function;

// Materialise the bytes between `lo` and `*pos*` as a fresh
// `<byte-string>`. Used by scanners that capture token text (idents,
// numbers, comments).
define function slice-from (lo :: <integer>) => (s :: <byte-string>)
  copy-sequence(*src*, lo, *pos*)
end function;

// ─── individual scanners ──────────────────────────────────────────────────
//
// Every scanner is called with `*pos*` pointing at the first byte of the
// token. Each one advances `*pos*` to the byte after the last consumed
// byte and returns a fully-built token.

// Run of whitespace bytes — one token per maximal run.
define function scan-whitespace (lo :: <integer>) => (t :: <whitespace-token>)
  until (at-end?() | ~ is-whitespace-byte?(current-byte()))
    advance(1);
  end;
  make(<whitespace-token>, span: span-here(lo))
end function;

// `// …` to end of line. Newline byte is NOT consumed (it becomes a
// whitespace token on the next iteration).
define function scan-line-comment (lo :: <integer>) => (t :: <comment-token>)
  until (at-end?() | current-byte() = 10)
    advance(1);
  end;
  make(<comment-token>,
       span: span-here(lo),
       text: slice-from(lo),
       is-block?: #f)
end function;

// `/* … */` — does NOT nest. The first `*/` closes the comment. EOF
// inside an unterminated block comment produces an `<error-token>` so
// callers can flag it visually.
define function scan-block-comment (lo :: <integer>) => (t :: <token>)
  advance(2);  // consume the opening "/*"
  let closed = #f;
  until (at-end?() | closed)
    if (current-byte() = 42 & peek-at(1) = 47)  // '*' '/'
      advance(2);
      closed := #t;
    else
      advance(1);
    end;
  end;
  if (closed)
    make(<comment-token>,
         span: span-here(lo),
         text: slice-from(lo),
         is-block?: #t)
  else
    make(<error-token>,
         span: span-here(lo),
         message: "unterminated block comment")
  end
end function;

// String literal `"…"` with escapes `\n \t \\ \" \r`. Returns either a
// `<string-literal-token>` (raw + decoded text) or an `<error-token>`
// for unterminated/invalid forms.
define function scan-string (lo :: <integer>) => (t :: <token>)
  advance(1);  // consume opening quote
  // Build the decoded value into a stretchy-vector of bytes; the raw
  // text comes from a slice of the source. Two allocations per string,
  // which is fine for an editor-shaped workload.
  let decoded-bytes = %make-stretchy-vector(16);
  let done = #f;
  let result = #f;
  until (done)
    if (at-end?())
      result := make(<error-token>,
                     span: span-here(lo),
                     message: "unterminated string literal");
      done := #t;
    else
      let b = current-byte();
      if (b = 34)  // closing '"'
        advance(1);
        let n = %stretchy-vector-size(decoded-bytes);
        let decoded = %byte-string-allocate(n);
        let i = 0;
        until (i = n)
          %byte-string-element-setter(%stretchy-vector-element(decoded-bytes, i),
                                      decoded, i);
          i := i + 1;
        end;
        result := make(<string-literal-token>,
                       span: span-here(lo),
                       raw-text: slice-from(lo),
                       decoded: decoded);
        done := #t;
      elseif (b = 10)  // bare newline — unterminated
        result := make(<error-token>,
                       span: span-here(lo),
                       message: "newline inside string literal");
        done := #t;
      elseif (b = 92)  // backslash escape
        advance(1);
        if (at-end?())
          result := make(<error-token>,
                         span: span-here(lo),
                         message: "trailing backslash in string literal");
          done := #t;
        else
          let esc = current-byte();
          let decoded-byte =
            if (esc = 110) 10        // \n
            elseif (esc = 116) 9     // \t
            elseif (esc = 114) 13    // \r
            elseif (esc = 92) 92     // \\
            elseif (esc = 34) 34     // \"
            elseif (esc = 39) 39     // \'
            elseif (esc = 48) 0      // \0
            else
              esc                    // unknown escape — pass-through
            end;
          %stretchy-vector-push(decoded-bytes, decoded-byte);
          advance(1);
        end;
      else
        %stretchy-vector-push(decoded-bytes, b);
        advance(1);
      end;
    end;
  end;
  result
end function;

// Character literal `'a'` or `'\n'` — same escape vocabulary as strings
// but exactly one codepoint. Sprint 45b ASCII-only; Unicode characters
// in the source produce an error token (Dylan source IS UTF-8 but
// `<character>` design waits for a later sprint).
define function scan-character (lo :: <integer>) => (t :: <token>)
  advance(1);  // consume opening quote
  if (at-end?())
    make(<error-token>,
         span: span-here(lo),
         message: "unterminated character literal")
  else
    let codepoint = -1;
    let b = current-byte();
    if (b = 39)  // empty '' — invalid
      advance(1);
      make(<error-token>,
           span: span-here(lo),
           message: "empty character literal")
    elseif (b = 92)  // escape
      advance(1);
      if (at-end?())
        make(<error-token>,
             span: span-here(lo),
             message: "trailing backslash in character literal")
      else
        let esc = current-byte();
        codepoint :=
          if (esc = 110) 10
          elseif (esc = 116) 9
          elseif (esc = 114) 13
          elseif (esc = 92) 92
          elseif (esc = 34) 34
          elseif (esc = 39) 39
          elseif (esc = 48) 0
          else esc
          end;
        advance(1);
        scan-character-close(lo, codepoint)
      end
    else
      codepoint := b;
      advance(1);
      scan-character-close(lo, codepoint)
    end
  end
end function;

// After the character body has been consumed, check for the closing
// quote and emit either a character-literal or an error token. Kept
// separate so both the escaped and bare branches share the logic.
define function scan-character-close
    (lo :: <integer>, codepoint :: <integer>) => (t :: <token>)
  if (at-end?() | current-byte() ~= 39)
    make(<error-token>,
         span: span-here(lo),
         message: "expected closing quote in character literal")
  else
    advance(1);
    make(<character-literal-token>,
         span: span-here(lo),
         codepoint: codepoint)
  end
end function;

// Decimal integer literal. Caller has verified the first byte is a
// digit. NB: negative numbers are lexed as `-` + digits — this scanner
// never sees a leading sign.
define function scan-integer (lo :: <integer>) => (t :: <integer-token>)
  let value = 0;
  until (at-end?() | ~ is-ascii-digit?(current-byte()))
    value := value * 10 + (current-byte() - 48);
    advance(1);
  end;
  make(<integer-token>,
       span: span-here(lo),
       value: value,
       radix: 10)
end function;

// Radix-prefixed integer. Caller has consumed `#` and the letter (`b`,
// `o`, or `x`); `radix` plus the matching digit predicate are passed
// in. Empty digit run produces an error token.
define function scan-radix-integer
    (lo :: <integer>, radix :: <integer>) => (t :: <token>)
  let value = 0;
  let any-digit? = #f;
  let done = #f;
  until (done)
    if (at-end?())
      done := #t;
    else
      let b = current-byte();
      let digit-value =
        if (is-ascii-digit?(b)) b - 48
        elseif (b >= 97 & b <= 102) b - 87   // a..f → 10..15
        elseif (b >= 65 & b <= 70) b - 55    // A..F → 10..15
        else -1
        end;
      if (digit-value < 0 | digit-value >= radix)
        done := #t;
      else
        value := value * radix + digit-value;
        any-digit? := #t;
        advance(1);
      end;
    end;
  end;
  if (any-digit?)
    make(<integer-token>,
         span: span-here(lo),
         value: value,
         radix: radix)
  else
    make(<error-token>,
         span: span-here(lo),
         message: "radix literal with no digits")
  end
end function;

// Identifier (or identifier-shaped keyword). Trailing `:` (NOT part of
// `::` / `:=`) folds in as a `<keyword-name-token>`. Recognised
// keyword bodies map to `<keyword-token>` via `classify-keyword`.
define function scan-identifier (lo :: <integer>) => (t :: <token>)
  until (at-end?() | ~ is-name-cont?(current-byte()))
    advance(1);
  end;
  // Check for trailing keyword-name colon: a `:` that is not part of
  // `::` (type ann) or `:=` (assignment). Peek both bytes.
  if (~ at-end?() & current-byte() = 58
        & peek-at(1) ~= 58 & peek-at(1) ~= 61)
    advance(1);
    let name = copy-sequence(*src*, lo, *pos* - 1);
    make(<keyword-name-token>,
         span: span-here(lo),
         name: name)
  else
    let name = slice-from(lo);
    let kw = classify-keyword(name);
    if (kw)
      make(<keyword-token>,
           span: span-here(lo),
           keyword: kw)
    else
      make(<identifier-token>,
           span: span-here(lo),
           name: name)
    end
  end
end function;

// Hash-prefixed forms — `#t`, `#f`, `#(`, `#[`, `#"…"`, `#x…`, `#b…`,
// `#o…`. The caller has NOT yet consumed the `#`. Falls through to
// `<error-token>` for unrecognised follow-up bytes.
define function scan-hash (lo :: <integer>) => (t :: <token>)
  advance(1);  // consume '#'
  if (at-end?())
    make(<error-token>,
         span: span-here(lo),
         message: "lone `#` at end of input")
  else
    let b = current-byte();
    if (b = 116 | b = 84)  // 't' | 'T'
      advance(1);
      make(<boolean-literal-token>, span: span-here(lo), value: #t)
    elseif (b = 102 | b = 70)  // 'f' | 'F'
      advance(1);
      make(<boolean-literal-token>, span: span-here(lo), value: #f)
    elseif (b = 40)  // '('
      advance(1);
      make(<literal-vector-open>, span: span-here(lo))
    elseif (b = 91)  // '['
      advance(1);
      make(<literal-sequence-open>, span: span-here(lo))
    elseif (b = 120 | b = 88)  // 'x' | 'X'
      advance(1);
      scan-radix-integer(lo, 16)
    elseif (b = 98 | b = 66)   // 'b' | 'B'
      advance(1);
      scan-radix-integer(lo, 2)
    elseif (b = 111 | b = 79)  // 'o' | 'O'
      advance(1);
      scan-radix-integer(lo, 8)
    elseif (b = 34)  // '"' — symbol literal #"foo"
      scan-hash-symbol(lo)
    else
      // Unrecognised: consume one byte so we make progress.
      advance(1);
      make(<error-token>,
           span: span-here(lo),
           message: "unrecognised `#` form")
    end
  end
end function;

// Body of `#"foo"`. The `#` is already consumed; the `"` is at *pos*.
// Uses the same escape vocabulary as string literals.
define function scan-hash-symbol (lo :: <integer>) => (t :: <token>)
  advance(1);  // consume opening '"'
  let name-bytes = %make-stretchy-vector(8);
  let done = #f;
  let result = #f;
  until (done)
    if (at-end?())
      result := make(<error-token>,
                     span: span-here(lo),
                     message: "unterminated symbol literal");
      done := #t;
    else
      let b = current-byte();
      if (b = 34)
        advance(1);
        let n = %stretchy-vector-size(name-bytes);
        let name = %byte-string-allocate(n);
        let i = 0;
        until (i = n)
          %byte-string-element-setter(%stretchy-vector-element(name-bytes, i),
                                      name, i);
          i := i + 1;
        end;
        result := make(<symbol-literal-token>,
                       span: span-here(lo),
                       name: name);
        done := #t;
      elseif (b = 10)
        result := make(<error-token>,
                       span: span-here(lo),
                       message: "newline inside symbol literal");
        done := #t;
      elseif (b = 92)
        advance(1);
        if (~ at-end?())
          %stretchy-vector-push(name-bytes, current-byte());
          advance(1);
        end;
      else
        %stretchy-vector-push(name-bytes, b);
        advance(1);
      end;
    end;
  end;
  result
end function;

// Punctuation dispatch — single-byte operators plus the multi-char
// combinations `==`, `=>`, `::`, `:=`, `...`, `??`, `?=`. The form
// slot uses canonical short symbols so the parser can dispatch with a
// single `select` on the punctuation symbol later.
define function scan-punctuation (lo :: <integer>) => (t :: <token>)
  let b = current-byte();
  if (b = 40)        // '('
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"lparen")
  elseif (b = 41)    // ')'
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"rparen")
  elseif (b = 91)    // '['
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"lbracket")
  elseif (b = 93)    // ']'
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"rbracket")
  elseif (b = 123)   // '{'
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"lbrace")
  elseif (b = 125)   // '}'
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"rbrace")
  elseif (b = 59)    // ';'
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"semicolon")
  elseif (b = 44)    // ','
    advance(1);
    make(<punctuation-token>, span: span-here(lo), form: #"comma")
  elseif (b = 46)    // '.'  -- check for "..."
    if (peek-at(1) = 46 & peek-at(2) = 46)
      advance(3);
      make(<punctuation-token>, span: span-here(lo), form: #"ellipsis")
    else
      advance(1);
      make(<punctuation-token>, span: span-here(lo), form: #"dot")
    end
  elseif (b = 58)    // ':' -- check for "::" then ":="
    if (peek-at(1) = 58)
      advance(2);
      make(<punctuation-token>, span: span-here(lo), form: #"colon-colon")
    elseif (peek-at(1) = 61)
      advance(2);
      make(<punctuation-token>, span: span-here(lo), form: #"assign")
    else
      advance(1);
      make(<punctuation-token>, span: span-here(lo), form: #"colon")
    end
  elseif (b = 61)    // '=' -- check for "==", "=>"
    if (peek-at(1) = 61)
      advance(2);
      make(<punctuation-token>, span: span-here(lo), form: #"equal-equal")
    elseif (peek-at(1) = 62)
      advance(2);
      make(<punctuation-token>, span: span-here(lo), form: #"arrow")
    else
      advance(1);
      make(<punctuation-token>, span: span-here(lo), form: #"equal")
    end
  elseif (b = 63)    // '?' -- check for "??" "?="
    if (peek-at(1) = 63)
      advance(2);
      make(<punctuation-token>, span: span-here(lo), form: #"query-query")
    elseif (peek-at(1) = 61)
      advance(2);
      make(<punctuation-token>, span: span-here(lo), form: #"query-equal")
    else
      advance(1);
      make(<punctuation-token>, span: span-here(lo), form: #"query")
    end
  else
    // Unknown punctuation — make progress, emit error.
    advance(1);
    make(<error-token>,
         span: span-here(lo),
         message: "unrecognised character")
  end
end function;

// ─── next-token dispatcher ────────────────────────────────────────────────
//
// Single point of dispatch: peek the first byte and route to the right
// scanner. Each scanner is responsible for advancing `*pos*` past the
// token it consumes (and for advancing at least one byte even on
// error). The dispatcher never decides "skip this" — every input byte
// ends up in exactly one token.
//
// The `else` arm is the catch-all `<error-token>` producer for bytes
// that no scanner accepted (e.g. a stray `@` outside an identifier).

define function next-token () => (t :: <token>)
  let lo = *pos*;
  if (at-end?())
    make(<eof-token>, span: span-here(lo))
  else
    let b = current-byte();
    if (is-whitespace-byte?(b))
      scan-whitespace(lo)
    elseif (b = 47 & peek-at(1) = 47)  // "//"
      advance(2);
      scan-line-comment(lo)
    elseif (b = 47 & peek-at(1) = 42)  // "/*"
      scan-block-comment(lo)
    elseif (b = 34)  // '"'
      scan-string(lo)
    elseif (b = 39)  // '\''
      scan-character(lo)
    elseif (b = 35)  // '#'
      scan-hash(lo)
    elseif (is-ascii-digit?(b))
      scan-integer(lo)
    elseif (is-name-start?(b))
      scan-identifier(lo)
    elseif (b = 40 | b = 41 | b = 91 | b = 93 | b = 123 | b = 125
              | b = 59 | b = 44 | b = 46 | b = 58 | b = 61 | b = 63)
      scan-punctuation(lo)
    elseif (b = 45)  // '-' — bare minus (signs are NOT folded; §9 #2)
      advance(1);
      make(<punctuation-token>, span: span-here(lo), form: #"minus")
    elseif (b = 43)  // '+'
      advance(1);
      make(<punctuation-token>, span: span-here(lo), form: #"plus")
    else
      // Catch-all: unrecognised byte. Advance one byte and emit an
      // error-token so the loop terminates.
      advance(1);
      make(<error-token>,
           span: span-here(lo),
           message: "unrecognised byte")
    end
  end
end function;

// ─── lex — public entry point ─────────────────────────────────────────────
//
// Reset the cursor, walk through the source one token at a time, append
// each to a stretchy vector, stop after pushing the EOF token. Always
// produces at least one token (the EOF).

// Inner accumulation loop. Reads `*tokens*` from the cell-backed module
// variable each iteration so the stretchy vector stays reachable even
// when local roots go stale under sustained allocation pressure.
// See GAP-007.
define function lex-into () => ()
  let done = #f;
  until (done)
    let t = next-token();
    %stretchy-vector-push(*tokens*, t);
    if (instance?(t, <eof-token>))
      done := #t;
    end;
  end;
end function;

define function lex (source :: <byte-string>) => (tokens)
  *src* := source;
  *pos* := 0;
  *tokens* := %make-stretchy-vector(64);
  lex-into();
  *tokens*
end function;

// ─── main — driver entry for `dump-dylan-tokens` ──────────────────────────
//
// The nod-driver subcommand bakes this file + a thin wrapper into an
// AOT EXE, runs the EXE with the user's path as argv[1], and forwards
// the stdout. Keeping `main` here means the subcommand's wrapper is
// effectively zero code; the wrapper module just re-exports `main`.
//
// Empty argv[1] → print a usage line to stderr-style stdout and exit
// cleanly. Sprint 45a doesn't have a process-exit primitive that
// returns non-zero from main, so usage failures still exit 0; the
// driver layer can detect the empty-stdout case if it wants to.

define function main () => ()
  let path = %argv1();
  if (empty?(path))
    format-out("dylan-lexer: missing input path\n");
  else
    let source = %read-file(path);
    if (empty?(source))
      format-out("dylan-lexer: could not read %s\n", path);
    else
      let tokens = lex(source);
      format-out("%s", dump-tokens(tokens, source));
    end;
  end;
end function main;
