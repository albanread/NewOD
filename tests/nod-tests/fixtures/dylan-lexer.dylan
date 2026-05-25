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
// Generic so future stream-shaped consumers (Sprint 45d's oracle test,
// the IDE inspector, etc.) can stream a token rather than concatenate.
// Sprint 45a uses string concatenation inside `dump-tokens` directly
// for now — Dylan has no `<stream>` class yet (DEFERRED.md: streams).
// These methods exist so the API surface is in place; they're called
// only via `print-token-to-string` below, which the dumper uses.

define method print-token (t :: <token>, source :: <byte-string>, stream)
 => ()
  // Sprint 45a stub — see comment above. Future sprints route this
  // through a `<stream>`-typed argument.
  // SPRINT 45a DESIGN-QUESTION: no <stream> class exists yet; the
  // `stream` parameter is untyped. When streams land we type it.
  #f
end method;

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

// ─── escape-source-text ───────────────────────────────────────────────────
//
// Replace control bytes and the quote / backslash with their canonical
// dump escapes. Output bytes follow these rules:
//   * byte 10  (LF)  → `\n`   (two characters: backslash + 'n')
//   * byte 9   (TAB) → `\t`
//   * byte 92  (`\`) → `\\`
//   * byte 34  (`"`) → `\"`
//   * byte 32  (` `) → `\s`   (so whitespace runs are visible)
//   * other printable bytes pass through unchanged
//   * other control bytes pass through unchanged — Sprint 45a doesn't
//     bother with hex escapes yet; the corpus we care about is LF-only.
//
// Concatenate-as-you-go shape: O(N²) on long strings but the lines we
// hand to it are token-sized (rarely > 80 bytes) so the cost is
// negligible. A two-pass sizing+writing variant tripped Sprint 42-pre's
// lower_if env-merge bug — too many SSA join points in the `elseif`
// chain for the current narrowing pass to model. Once 45c lifts these
// to stdlib character predicates we revisit.

define function escape-byte (b :: <integer>) => (out :: <byte-string>)
  if (b = 10)
    "\\n"
  elseif (b = 9)
    "\\t"
  elseif (b = 92)
    "\\\\"
  elseif (b = 34)
    "\\\""
  elseif (b = 32)
    "\\s"
  else
    let one = %byte-string-allocate(1);
    %byte-string-element-setter(b, one, 0);
    one
  end
end function;

define function escape-source-text (s :: <byte-string>) => (out :: <byte-string>)
  let n = %byte-string-size(s);
  let acc = "";
  let i = 0;
  until (i = n)
    let b = %byte-string-element(s, i);
    acc := concatenate(acc, escape-byte(b));
    i := i + 1;
  end;
  acc
end function;

// ─── print-token-to-string ────────────────────────────────────────────────
//
// Build one canonical dump line for a token. Returns the line WITHOUT
// the trailing newline (the caller adds it). Format:
//
//   <start-line>:<start-col>-<end-line>:<end-col>  <KIND>  <escaped-text>
//
// Fields separated by EXACTLY two spaces. EOF tokens emit just the
// position + KIND (no trailing escaped-text, no trailing two spaces).
// This locks in the oracle-test contract for sprint 45d.

define function print-token-to-string
    (t :: <token>, source :: <byte-string>) => (line :: <byte-string>)
  let span = token-span(t);
  let start-packed = offset-to-line-col-packed(source, span-start(span));
  let end-packed   = offset-to-line-col-packed(source, span-end(span));
  let sl = unpack-line(start-packed);
  let sc = unpack-col(start-packed);
  let el = unpack-line(end-packed);
  let ec = unpack-col(end-packed);
  let pos = concatenate(
              concatenate(
                concatenate(nod-int-to-string(sl), ":"),
                concatenate(nod-int-to-string(sc), "-")),
              concatenate(
                concatenate(nod-int-to-string(el), ":"),
                nod-int-to-string(ec)));
  let kind = token-kind-name(t);
  let head = concatenate(pos, concatenate("  ", kind));
  // EOF lines stop after the kind tag — there's no source text to show.
  if (instance?(t, <eof-token>))
    head
  else
    let txt = escape-source-text(token-source-text(t, source));
    concatenate(head, concatenate("  ", txt))
  end
end function;

// ─── dump-tokens ──────────────────────────────────────────────────────────
//
// Build the whole-buffer dump. Walks the stretchy-vector, calls
// `print-token-to-string` per token, joins with '\n', and appends a
// trailing newline so the dump is line-oriented (every token line ends
// in '\n', including the last).

define function dump-tokens
    (tokens, source :: <byte-string>) => (text :: <byte-string>)
  let n = %stretchy-vector-size(tokens);
  let acc = "";
  let i = 0;
  until (i = n)
    let t = %stretchy-vector-element(tokens, i);
    let line = print-token-to-string(t, source);
    acc := concatenate(acc, concatenate(line, "\n"));
    i := i + 1;
  end;
  acc
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
