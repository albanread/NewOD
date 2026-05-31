# 2026-05-31 — DylanAst → ast::Module: the parser starts *replacing* parse_module

*Sprint 51e, fork #2. Continuation of
[the kind-coverage grind](2026-05-31-parser-kind-coverage.md). The
literal-span fix (fork #1) unblocked this.*

## Goal

Build the thing the whole AST wire format was for: a translator that
turns the Dylan parser's wire tree into the **canonical**
`nod_reader::ast::Module`, wired behind `--parse-with-dylan` so the
Dylan parser can *replace* `parse_module` for the files it fully
understands — with a verify-style fall-back to the Rust parser on
anything it can't yet reconstruct, and a **byte-identical** gate
proving "translated" means "the two parsers agree," not "didn't crash."

## What we did

1. **Wire enrichment — function signatures (kinds 25–30).** The wire
   carried only a definition's *body*; `ast::Item::DefineFunction`
   needs `{name, params, return_, body}`. Reshaped the
   `<ast-body-definition>` emitter to emit, as children dispatched by
   KIND (not position): `DefName`(27, the name token), `ParamList`(25,
   with `Param`(28) children carrying an optional type-expr child),
   `ReturnSpec`(26, emitted *only* when an `=>` is present, with
   `ReturnValue`(30) children), and the `Body`. `VarMarker`(29) is a
   sentinel child for `#rest`/`#key`/`#all-keys`/`#next`, which the v1
   host declines. Matching Rust `Kind` variants + `DYLAN_AST_WIRE.md`
   rows.

2. **`dylan_to_ast.rs` — the translator.** `to_ast_module(tree, src) ->
   Result<Module, Unsupported>`. Header re-scanned host-side with
   `scan_preamble` (the Dylan parser doesn't model it — see below);
   `DefineFunction`/`DefineMethod` rebuilt from the signature children;
   bodies → `Vec<Statement>`; expressions for the cheap subset
   (`Ident`, `String`, `Integer`, `Float`, `Bool`, `Call`). Anything
   else returns `Unsupported`.

3. **`--parse-with-dylan` flag.** In `run_dump_ast`: try the Dylan
   parse + translate; print and return on `Ok`; on *any*
   `Unsupported`/wire error, fall through to `parse_module_with_macros`.
   Deliberately does **not** imply `--lex-with-dylan` — the Rust
   fallback keeps the Rust lexer so a fallback's AST is identical to
   plain `dump-ast`, keeping the gate measuring the *translator*.

4. **Translation-coverage gate (`dylan_parse_translate.rs`).** Runs
   both `dump-ast` and `--parse-with-dylan dump-ast` over the corpus;
   asserts byte-identical stdout on every fixture; tallies
   translated-vs-fell-back and ranks the fall-back reasons as the
   next-increment punch-list. Asserts ≥ `hello.dylan` translates.

**Result: hello.dylan translates byte-identically via the Dylan
parser** — the first time the Dylan front-end's output, lifted to the
canonical AST, exactly equals the Rust parser. 1/34 translated; 33
fall back (cleanly).

## Why

The bar was deliberately **byte-identical `format_ast_module`**, not
"lowers OK." A weaker bar (does it compile? does it run?) would let
subtle structural disagreements slide; equality of the canonical dump
is the strongest cheap proof that the two parsers built the *same*
tree. The flag is the verify-mode philosophy taken one step further:
51c ran both parsers and compared accept/reject; this runs both and
compares the **whole AST**, then *uses* the Dylan one when they agree.

The fall-back-on-`Unsupported` design means the output is never wrong,
only "translated" or "fell back" — so the gate can ratchet: each
increment teaches the translator one more kind and watches the
translated count rise, exactly like the node-coverage harness ratchets
the structured-node count. Two dashboards now: *nodes structured*
(emitter side, 99%) and *files translated* (translator side, 1/34).

## Discovered

1. **`format_ast_module` prints no spans — the comparison is
   span-independent.** This collapsed a whole imagined difficulty.
   We feared having to make every span exactly match; in fact the dump
   is purely names / structure / values / operators / modifiers. The
   translator threads real spans through anyway (so the `Module` is
   usable downstream), but the *gate* doesn't care, which is why a
   coarse-span wire format is still enough to prove AST equality.

2. **`ast::Expr::String` stores the RAW quoted source slice, not the
   decoded value.** The Rust parser keeps `"\"hello\\n\""` verbatim —
   so translation is literally `&src[span]`, no escape decoding. The
   wire philosophy ("spans not values, host re-reads source") turned
   out to match the AST's own representation exactly.

3. **The Dylan parser lexes the module header as ordinary body
   forms.** `Module: hello` shows up in the wire as a `SymbolLit
   "Module:"` + `VariableRef "hello"` pair at the top of the Body —
   the Dylan parser has no header concept. The host owns the header
   (re-scan with `scan_preamble`) and skips body forms that lie inside
   the preamble. Clean division of labour: trivial header parsing stays
   Rust-side; the Dylan parser does the items.

4. **The bare-return-type asymmetry maps cleanly.** `=> (<integer>)`:
   the Dylan parser stores the type AS the value's token (tok =
   `<integer>`, type = #f), while Rust models it as `name: None, type:
   Ident("<integer>")`. The rule "ReturnValue with no type-child →
   name None, type Ident(span); with a child → name Some(span), type
   child" reconciles them without a special case.

5. **The byte-identical gate immediately earned its keep — and exposed
   a subtle translator bug.** First run: two divergences
   (`stdlib-min`, `ide_win_calls`). Both emitted a too-*empty* Module
   instead of falling back. Cause: their `define macro` /
   `define c-function` forms emit as **unspanned `Error 0..0`** nodes,
   and the header-skip heuristic (`span_hi <= body_start`) treated
   `span_hi == 0` as "inside the preamble" → silently dropped them →
   `Ok(empty)` instead of `Unsupported`. The lesson: *"skip the header"
   and "an unspanned node" look identical under a `<=` test.* Fix:
   never treat `span_hi == 0` as a header form, and force `Unsupported`
   on any `Error` node. This is precisely the failure the gate exists
   to catch — a silent wrong translation that a weaker "did it run?"
   check would have waved through.

## Where it leaves us

`--parse-with-dylan` is live and authoritative for the files it
understands; the gate guarantees it can never silently diverge from
the Rust parser. **Translated: 1/34** (`hello.dylan`). The fall-back
punch-list — the next-increment to-do list, ranked — is:

| Count | Reason | Next increment |
|------:|--------|----------------|
| 13 | top-level `DefineClass` | translate `Item::DefineClass` (supers + slots) |
| 6 | `Error` node | (genuinely unparsed — emitter work, not translator) |
| 6 | expression `BinaryOp` | `BinOp` w/ operator recovered from `&src` gap |
| 4 | expression `LocalDecl` | `let` → `Statement::Let` |
| 2 | expression `Statement` | `if`/`while`/`block` → `Expr`/`Statement` |
| 1 | top-level `BinaryOp` | — |

The obvious next move is **`BinaryOp` + `Statement(if)` + `LocalDecl`**,
which together flip `factorial.dylan` (and much of the corpus) to
translated — the operator-from-`&src` recovery is the one genuinely
new technique. Each is one translator function + (where needed) a wire
tweak, measured by the translated count climbing.
