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

define method print-token
    (t :: <token>, source :: <byte-string>, stream :: <string-stream>)
 => ()
  let span = token-span(t);
  let start-packed = offset-to-line-col-packed(source, span-start(span));
  let end-packed   = offset-to-line-col-packed(source, span-end(span));
  write-line-col(stream, unpack-line(start-packed), unpack-col(start-packed));
  write-byte(stream, 45);  // '-'
  write-line-col(stream, unpack-line(end-packed),   unpack-col(end-packed));
  write-string(stream, "  ");
  write-string(stream, token-kind-name(t));
  // GAP-005 + GAP-006 both fixed: else-less if lowers cleanly AND
  // codegen tolerates void-returning calls in if-arms by binding
  // their dst to the nil singleton. The natural side-effect-only
  // form just works now.
  if (~instance?(t, <eof-token>))
    write-string(stream, "  ");
    write-escaped-source-text(stream, token-source-text(t, source));
  end;
end method;

// ─── write-line-col — small helper: `<line>:<col>` into a stream ─────────

define function write-line-col
    (stream :: <string-stream>, line :: <integer>, col :: <integer>) => ()
  write-string(stream, nod-int-to-string(line));
  write-byte(stream, 58);  // ':'
  write-string(stream, nod-int-to-string(col));
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

define function dump-tokens
    (tokens, source :: <byte-string>) => (text :: <byte-string>)
  let stream = make-string-stream();
  let n = %stretchy-vector-size(tokens);
  let i = 0;
  until (i = n)
    let t = %stretchy-vector-element(tokens, i);
    print-token(t, source, stream);
    write-byte(stream, 10);  // '\n' — every line ends in LF, even the last.
    i := i + 1;
  end;
  as-byte-string(stream)
end function;

// ─── lex — Sprint 45a STUB ────────────────────────────────────────────────
//
// Real implementation lands in 45b. Returns a one-element stretchy
// vector holding a single `<eof-token>` at byte offset 0, so the dump
// path is fully exercisable end-to-end. The driver's
// `dump-dylan-tokens` subcommand pipes its argv[1] file through this
// stub today; the same call shape lights up the real lexer in 45b
// without any driver changes.

define function lex (source :: <byte-string>) => (tokens)
  let tokens = %make-stretchy-vector(1);
  let eof-span = make(<span>, start: 0, end: 0);
  %stretchy-vector-push(tokens, make(<eof-token>, span: eof-span));
  tokens
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
