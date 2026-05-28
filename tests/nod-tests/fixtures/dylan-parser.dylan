Module: dylan-lexer

// Sprint 46 — Recursive-descent Dylan parser in Dylan.
//
// Consumes a <stretchy-vector> of <token> values produced by dylan-lexer.dylan
// and produces an AST.  The grammar is transcribed directly from
//   sources/dfmc/reader/parser.dylgram
// which is a yacc-like BNF for the full Dylan language.
//
// This file must be loaded AFTER dylan-lexer.dylan (same compilation unit).
// All token classes and their accessor methods are assumed in scope.
//
// Key grammar terminal classes mapped to our token types:
//   DEFINE            → <keyword-token> keyword: #"define"
//   END               → <keyword-token> keyword: #"end"
//   OTHERWISE         → <keyword-token> keyword: #"otherwise"
//   BEGIN-WORD        → <keyword-token> keyword: {begin if case select for
//                         while unless until block iterate when cond}
//   FUNCTION-WORD     → <keyword-token> keyword: {method function}
//   DEFINE-BODY-WORD  → <keyword-token> keyword: {class generic module library
//                         method function}
//   DEFINE-LIST-WORD  → <keyword-token> keyword: {variable constant domain}
//   LOCAL-DECL-WORD   → <keyword-token> keyword: {let}
//   LOCAL-METHODS-WORD→ <keyword-token> keyword: {local}
//   BINARY-OPERATOR   → <punctuation-token> form: {equal equal-equal plus minus
//                         star slash caret amp bar less greater less-equal
//                         greater-equal tilde-equal tilde-equal-equal dot-dot
//                         assign}
//   UNARY-OPERATOR    → <punctuation-token> form: {tilde minus}
//   UNRESERVED-NAME   → <identifier-token> | <escaped-ident-token>
//   NAME              → any <identifier-token>, <escaped-ident-token>, or
//                       <keyword-token>
//   NUMBER            → <integer-token> | <float-token> | <ratio-token>
//   STRING            → <string-literal-token>
//   CHARACTER-LITERAL → <character-literal-token>
//   SYMBOL            → <symbol-literal-token> | <keyword-name-token>

// ── 1. Token stream ───────────────────────────────────────────────────────
//
// Wraps the flat token vector from lex() with a position cursor.
// ts-peek() and ts-advance() both skip whitespace and comment tokens
// automatically so the parse functions never see non-semantic tokens.

define class <token-stream> (<object>)
  slot ts-tokens :: <stretchy-vector>, init-keyword: tokens:;
  slot ts-pos    :: <integer>,        init-value: 0;
end class;

define function make-token-stream (toks :: <stretchy-vector>)
 => (ts :: <token-stream>)
  make(<token-stream>, tokens: toks)
end function;

// Advance past whitespace / comment tokens.
define function ts-skip (ts :: <token-stream>) => ()
  let toks = ts-tokens(ts);
  let n    = size(toks);
  until (ts-pos(ts) >= n
         | (~ instance?(toks[ts-pos(ts)], <whitespace-token>)
            & ~ instance?(toks[ts-pos(ts)], <comment-token>)))
    ts-pos(ts) := ts-pos(ts) + 1;
  end
end function;

// Return the next meaningful token without consuming it.
define function ts-peek (ts :: <token-stream>) => (t :: <token>)
  ts-skip(ts);
  let toks = ts-tokens(ts);
  let p    = ts-pos(ts);
  let n    = size(toks);
  if (p >= n) toks[n - 1] else toks[p] end
end function;

// Consume and return the next meaningful token.
define function ts-advance (ts :: <token-stream>) => (t :: <token>)
  ts-skip(ts);
  let t = ts-tokens(ts)[ts-pos(ts)];
  ts-pos(ts) := ts-pos(ts) + 1;
  t
end function;

define function ts-at-end? (ts :: <token-stream>) => (yes? :: <boolean>)
  instance?(ts-peek(ts), <eof-token>)
end function;

// Consume a token, signalling an error if its kind is wrong.
// `what` is a descriptive string for error messages.
define function ts-expect-keyword (ts :: <token-stream>, kw :: <symbol>,
                                   what :: <byte-string>)
 => (t :: <token>)
  let t = ts-peek(ts);
  if (instance?(t, <keyword-token>) & keyword-token-keyword(t) = kw)
    ts-advance(ts)
  else
    error(what)
  end
end function;

define function ts-expect-punct (ts :: <token-stream>, form :: <symbol>,
                                 what :: <byte-string>)
 => (t :: <token>)
  let t = ts-peek(ts);
  if (instance?(t, <punctuation-token>) & punctuation-token-form(t) = form)
    ts-advance(ts)
  else
    error(what)
  end
end function;

// ── 2. Token predicates ───────────────────────────────────────────────────

define function is-keyword? (t :: <token>, kw :: <symbol>)
 => (yes? :: <boolean>)
  instance?(t, <keyword-token>) & keyword-token-keyword(t) = kw
end function;

define function is-punct? (t :: <token>, form :: <symbol>)
 => (yes? :: <boolean>)
  instance?(t, <punctuation-token>) & punctuation-token-form(t) = form
end function;

define function is-define-token? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"define")
end function;

define function is-end-token? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"end")
end function;

define function is-otherwise-token? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"otherwise")
end function;

// BEGIN-WORD-ONLY and combined BEGIN-WORD variants:
// Words that open a statement macro (terminated by END).
define function is-begin-word? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <keyword-token>))
    let kw = keyword-token-keyword(t);
    kw = #"begin"   | kw = #"if"      | kw = #"case"   | kw = #"select"
      | kw = #"for"   | kw = #"while"  | kw = #"unless" | kw = #"until"
      | kw = #"block" | kw = #"iterate" | kw = #"when"  | kw = #"cond"
  else
    #f
  end
end function;

// FUNCTION-WORD: `method` and `function` begin an anonymous function body.
define function is-function-word? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <keyword-token>))
    let kw = keyword-token-keyword(t);
    kw = #"method" | kw = #"function"
  else
    #f
  end
end function;

// DEFINE-BODY-WORD: word after `define` that takes a body ending with `end`.
define function is-define-body-word? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <keyword-token>))
    let kw = keyword-token-keyword(t);
    kw = #"class" | kw = #"generic" | kw = #"module" | kw = #"library"
      | kw = #"method" | kw = #"function"
  else
    #f
  end
end function;

// DEFINE-LIST-WORD: word after `define` that takes a list (no `end`).
define function is-define-list-word? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <keyword-token>))
    let kw = keyword-token-keyword(t);
    kw = #"variable" | kw = #"constant" | kw = #"domain"
  else
    #f
  end
end function;

// LOCAL-DECLARATION-WORD: `let` introduces a local binding.
define function is-local-decl-word? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"let")
end function;

// LOCAL-METHODS-WORD: `local` introduces local method definitions.
define function is-local-methods-word? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"local")
end function;

// Any token that can appear as a name in a NAME position.
define function is-name-token? (t :: <token>) => (yes? :: <boolean>)
  instance?(t, <identifier-token>)
    | instance?(t, <escaped-ident-token>)
    | instance?(t, <keyword-token>)
end function;

// NAME-NOT-END: names except `end` (used in end-clause parsing).
define function is-name-not-end? (t :: <token>) => (yes? :: <boolean>)
  is-name-token?(t) & ~ is-end-token?(t)
end function;

// ORDINARY-NAME: unreserved names plus define-words that can be used as
// binding names (identifiers, escaped operators, define/list words).
define function is-ordinary-name? (t :: <token>) => (yes? :: <boolean>)
  instance?(t, <identifier-token>)
    | instance?(t, <escaped-ident-token>)
    | is-define-body-word?(t)
    | is-define-list-word?(t)
end function;

// BINARY-OPERATOR: tokens that appear as infix operators.
// `:=` (assign) is included here for assignment expressions.
define function is-binary-op? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <punctuation-token>))
    let f = punctuation-token-form(t);
    f = #"equal"       | f = #"equal-equal"
      | f = #"plus"    | f = #"minus"
      | f = #"star"    | f = #"slash"    | f = #"caret"
      | f = #"amp"     | f = #"bar"
      | f = #"less"    | f = #"greater"
      | f = #"less-equal"        | f = #"greater-equal"
      | f = #"tilde-equal"       | f = #"tilde-equal-equal"
      | f = #"dot-dot" | f = #"assign"
  else
    #f
  end
end function;

// UNARY-OPERATOR: `~` (logical not) and `-` (negation) in prefix position.
define function is-unary-op? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <punctuation-token>))
    let f = punctuation-token-form(t);
    f = #"tilde" | f = #"minus"
  else
    #f
  end
end function;

// Tokens that terminate a body at nesting depth 0.
// Used by parse-body to know when to stop consuming constituents.
define function is-body-terminator? (t :: <token>) => (yes? :: <boolean>)
  is-end-token?(t)
    | is-keyword?(t, #"else")
    | is-keyword?(t, #"elseif")
    | is-keyword?(t, #"cleanup")
    | is-keyword?(t, #"exception")
    | is-keyword?(t, #"finally")
    | is-keyword?(t, #"otherwise")
    | instance?(t, <eof-token>)
    | is-punct?(t, #"rparen")
    | is-punct?(t, #"rbracket")
    | is-punct?(t, #"rbrace")
end function;

// ── 3. AST node classes ───────────────────────────────────────────────────
//
// Every node carries the leading token for source-location reporting.

// Abstract base.
define class <ast-node> (<object>)
  slot node-token :: <object>, init-value: #f;   // leading <token> or #f
end class;

// Ordered sequence of constituents (body of a definition, statement, etc.).
define class <ast-body> (<ast-node>)
  slot body-constituents :: <stretchy-vector>;
end class;

// Placeholder for a parse error (partial error recovery).
define class <ast-error-node> (<ast-node>)
  slot ast-error-msg :: <byte-string>, init-keyword: message:;
end class;

// `define [modifiers] BODY-WORD body-fragment ... end [WORD] [NAME]`
// e.g. define class <Foo> (<Bar>) ... end class <Foo>
define class <ast-body-definition> (<ast-node>)
  slot defn-modifiers   :: <stretchy-vector>;   // vector of <token>
  slot defn-word        :: <token>,    init-keyword: word:;
  slot defn-body        :: <ast-body>, init-keyword: body:;
  slot defn-end-word    :: <object>, init-value: #f;   // <token> or #f
  slot defn-end-name    :: <object>, init-value: #f;   // <token> or #f
  // Method / function definitions carry a name and signature.
  slot defn-method-name :: <object>, init-value: #f;   // <token> or #f
  slot defn-params      :: <object>, init-value: #f;   // <ast-param-list> or #f
  slot defn-return      :: <object>, init-value: #f;   // <ast-return-spec> or #f
end class;

// `define [modifiers] LIST-WORD list-fragment`
// e.g. define constant pi = 3.14159;
define class <ast-list-definition> (<ast-node>)
  slot defn-modifiers :: <stretchy-vector>;   // vector of <token>
  slot defn-word      :: <token>,    init-keyword: word:;
  slot defn-list      :: <ast-body>, init-keyword: list:;
end class;

// `let var [:: type] = expr`  /  `let (a, b) = expr`
define class <ast-local-decl> (<ast-node>)
  slot ldecl-word :: <token>,    init-keyword: word:;
  slot ldecl-list :: <ast-body>, init-keyword: list:;
end class;

// `local method name params ... end method name, ...`
define class <ast-local-methods> (<ast-node>)
  slot lmethods-items :: <stretchy-vector>;
end class;

// `left OP right` — left-associative binary expression
define class <ast-binary-op> (<ast-node>)
  slot binop-left     :: <ast-node>, init-keyword: left:;
  slot binop-operator :: <token>,    init-keyword: operator:;
  slot binop-right    :: <ast-node>, init-keyword: right:;
end class;

// `OP operand` — prefix unary expression
define class <ast-unary-op> (<ast-node>)
  slot unary-op      :: <token>,    init-keyword: op:;
  slot unary-operand :: <ast-node>, init-keyword: operand:;
end class;

// `function(arg, ...)` — function call
define class <ast-call> (<ast-node>)
  slot call-fn   :: <ast-node>, init-keyword: fn:;
  slot call-args :: <stretchy-vector>;
end class;

// `receiver.name` — dot-notation call: name(receiver)
define class <ast-dot-call> (<ast-node>)
  slot dot-receiver :: <ast-node>, init-keyword: receiver:;
  slot dot-name     :: <token>,    init-keyword: name:;
end class;

// `receiver[args]` — subscript: element(receiver, args)
define class <ast-subscript> (<ast-node>)
  slot sub-receiver :: <ast-node>, init-keyword: receiver:;
  slot sub-args     :: <stretchy-vector>;
end class;

// A reference to a variable / function / class name
define class <ast-variable-ref> (<ast-node>)
  slot varref-tok :: <token>, init-keyword: tok:;
end class;

// Abstract base for all literal values.
define class <ast-literal> (<ast-node>) end class;

define class <ast-integer-lit> (<ast-literal>)
  slot lit-value :: <integer>, init-keyword: value:;
  slot lit-radix :: <integer>, init-keyword: radix:;
end class;

define class <ast-float-lit> (<ast-literal>)
  slot lit-raw :: <byte-string>, init-keyword: raw:;
end class;

define class <ast-ratio-lit> (<ast-literal>)
  slot lit-raw :: <byte-string>, init-keyword: raw:;
end class;

define class <ast-string-lit> (<ast-literal>)
  slot lit-value :: <byte-string>, init-keyword: value:;
end class;

define class <ast-char-lit> (<ast-literal>)
  slot lit-codepoint :: <integer>, init-keyword: codepoint:;
end class;

define class <ast-boolean-lit> (<ast-literal>)
  slot lit-value :: <boolean>, init-keyword: value:;
end class;

define class <ast-symbol-lit> (<ast-literal>)
  slot lit-name :: <byte-string>, init-keyword: name:;
end class;

// `#(a, b, c)`  or  `#(a, b . tail)` — list literal
define class <ast-list-lit> (<ast-literal>)
  slot lit-elems :: <stretchy-vector>;
  slot lit-tail  :: <object> = #f;   // #f for proper list; <ast-node> for improper
end class;

// `#[a, b, c]` — vector literal
define class <ast-vector-lit> (<ast-literal>)
  slot lit-elems :: <stretchy-vector>;
end class;

// `BEGIN-WORD body END [end-word] [end-name]`
// Covers: begin...end, if...end, for...end, method...end, etc.
define class <ast-statement> (<ast-node>)
  slot stmt-word     :: <token>,    init-keyword: word:;
  slot stmt-body     :: <ast-body>, init-keyword: body:;
  slot stmt-end-word :: <object> = #f;   // <token> in `end method` or #f
  slot stmt-end-name :: <object> = #f;   // <token> in `end method foo` or #f
  // Anonymous method / function literals carry a signature.
  slot stmt-method-name :: <object> = #f;   // <token> or #f (local method name)
  slot stmt-params      :: <object> = #f;   // <ast-param-list> or #f
  slot stmt-return      :: <object> = #f;   // <ast-return-spec> or #f
end class;

// A positional call argument
define class <ast-pos-arg> (<ast-node>)
  slot pos-arg-value :: <ast-node>, init-keyword: value:;
end class;

// A keyword call argument  `keyword: value`
define class <ast-kw-arg> (<ast-node>)
  slot kw-arg-key   :: <token>,    init-keyword: key:;
  slot kw-arg-value :: <ast-node>, init-keyword: value:;
end class;

// `name [:: type]` — variable binding in let / parameter list
define class <ast-typed-name> (<ast-node>)
  slot typed-name-tok  :: <token>,  init-keyword: tok:;
  slot typed-name-type :: <object>, init-value: #f;   // #f or <ast-node>
end class;

// `keyword [:: type] [= default]` — one `#key` parameter spec.
define class <ast-key-spec> (<ast-node>)
  slot key-spec-tok     :: <token>,  init-keyword: tok:;
  slot key-spec-type    :: <object>, init-value: #f;   // #f or <ast-node>
  slot key-spec-default :: <object>, init-value: #f;   // #f or <ast-node>
end class;

// `( var, ..., #rest r, #key k ..., #all-keys, #next n )`
// A method / function parameter list.
//   params-required : vector of <ast-typed-name>
//   params-rest     : <token> name after #rest, or #f
//   params-keys     : vector of <ast-key-spec> after #key
//   params-key?     : #t if #key appeared (even with no specs)
//   params-all-keys?: #t if #all-keys appeared
//   params-next     : <token> name after #next, or #f
define class <ast-param-list> (<ast-node>)
  slot params-required :: <stretchy-vector>;
  slot params-rest     :: <object>,  init-value: #f;   // <token> or #f
  slot params-keys     :: <stretchy-vector>;
  slot params-key?     :: <boolean>, init-value: #f;
  slot params-all-keys? :: <boolean>, init-value: #f;
  slot params-next     :: <object>,  init-value: #f;   // <token> or #f
end class;

// `=> spec` — a return specification.
//   ret-present?  : #t when an `=>` was actually present
//   ret-values    : vector of <ast-typed-name> (value name [:: type])
//   ret-rest      : <token> name after #rest, or #f
//   ret-rest-type : type after `#rest name :: type`, or #f
define class <ast-return-spec> (<ast-node>)
  slot ret-present?  :: <boolean>, init-value: #f;
  slot ret-values    :: <stretchy-vector>;
  slot ret-rest      :: <object>, init-value: #f;   // <token> or #f
  slot ret-rest-type :: <object>, init-value: #f;   // <ast-node> or #f
end class;

// ── 4. Constructors for AST nodes with vector slots ───────────────────────
//
// Dylan's `init-value:` shares one initial value across instances, which
// would alias all stretchy-vectors.  Use explicit constructors instead.

define function make-ast-body () => (b :: <ast-body>)
  let b = make(<ast-body>);
  body-constituents(b) := make(<stretchy-vector>);
  b
end function;

define function make-ast-call (func :: <ast-node>) => (c :: <ast-call>)
  let c = make(<ast-call>, fn: func);
  call-args(c) := make(<stretchy-vector>);
  c
end function;

define function make-ast-subscript (recv :: <ast-node>) => (s :: <ast-subscript>)
  let s = make(<ast-subscript>, receiver: recv);
  sub-args(s) := make(<stretchy-vector>);
  s
end function;

define function make-ast-body-definition (word :: <token>)
 => (d :: <ast-body-definition>)
  let d = make(<ast-body-definition>, word: word, body: make-ast-body());
  defn-modifiers(d) := make(<stretchy-vector>);
  d
end function;

define function make-ast-list-definition (word :: <token>)
 => (d :: <ast-list-definition>)
  let d = make(<ast-list-definition>, word: word, list: make-ast-body());
  defn-modifiers(d) := make(<stretchy-vector>);
  d
end function;

define function make-ast-local-methods () => (m :: <ast-local-methods>)
  let m = make(<ast-local-methods>);
  lmethods-items(m) := make(<stretchy-vector>);
  m
end function;

define function make-ast-list-lit () => (l :: <ast-list-lit>)
  let l = make(<ast-list-lit>);
  lit-elems(l) := make(<stretchy-vector>);
  l
end function;

define function make-ast-vector-lit () => (v :: <ast-vector-lit>)
  let v = make(<ast-vector-lit>);
  lit-elems(v) := make(<stretchy-vector>);
  v
end function;

define function make-ast-param-list () => (p :: <ast-param-list>)
  let p = make(<ast-param-list>);
  params-required(p) := make(<stretchy-vector>);
  params-keys(p)     := make(<stretchy-vector>);
  p
end function;

define function make-ast-return-spec () => (r :: <ast-return-spec>)
  let r = make(<ast-return-spec>);
  ret-values(r) := make(<stretchy-vector>);
  r
end function;

// ── 5. Name extraction helpers ────────────────────────────────────────────

// Retrieve a printable name from a name-like token.
define function token-name (t :: <token>) => (s :: <byte-string>)
  if (instance?(t, <identifier-token>))
    identifier-token-name(t)
  elseif (instance?(t, <escaped-ident-token>))
    escaped-ident-token-name(t)
  elseif (instance?(t, <keyword-name-token>))
    keyword-name-token-name(t)
  elseif (instance?(t, <keyword-token>))
    // Map keyword symbol to its string spelling.
    let kw = keyword-token-keyword(t);
    if      (kw = #"define")    "define"
    elseif  (kw = #"end")       "end"
    elseif  (kw = #"otherwise") "otherwise"
    elseif  (kw = #"if")        "if"
    elseif  (kw = #"else")      "else"
    elseif  (kw = #"elseif")    "elseif"
    elseif  (kw = #"then")      "then"
    elseif  (kw = #"begin")     "begin"
    elseif  (kw = #"method")    "method"
    elseif  (kw = #"function")  "function"
    elseif  (kw = #"class")     "class"
    elseif  (kw = #"generic")   "generic"
    elseif  (kw = #"module")    "module"
    elseif  (kw = #"library")   "library"
    elseif  (kw = #"let")       "let"
    elseif  (kw = #"local")     "local"
    elseif  (kw = #"variable")  "variable"
    elseif  (kw = #"constant")  "constant"
    elseif  (kw = #"domain")    "domain"
    elseif  (kw = #"for")       "for"
    elseif  (kw = #"while")     "while"
    elseif  (kw = #"until")     "until"
    elseif  (kw = #"unless")    "unless"
    elseif  (kw = #"case")      "case"
    elseif  (kw = #"select")    "select"
    elseif  (kw = #"block")     "block"
    else                        "???"
    end
  else
    "???"
  end
end function;

// ── 6. Parse helpers ──────────────────────────────────────────────────────

// Fail-fast: print the message to stdout for visibility, then call
// `%error` to signal a <simple-error>. The runtime's unhandled-
// signalled-condition path raises a Rust panic, which the Sprint 45g
// crash dumper catches and reports with GC + safepoint state, exiting
// 99. This makes the in-flight parser crash at the closest point to
// the actual syntax problem rather than building a partial AST with
// inline error nodes that fail later, far from the originating site.
// The trailing `make(<ast-error-node>, ...)` is unreachable but
// satisfies the return type — `%error` never returns. Once the parser
// is feature-complete and we want recoverable diagnostics, this
// function can revert to its earlier `make(<ast-error-node>, ...)`
// behaviour and the call sites stay unchanged.
define function parse-error (msg :: <byte-string>) => (n :: <ast-error-node>)
  format-out("parse-error: %s\n", msg);
  %error(msg);
  make(<ast-error-node>, message: msg)
end function;

// ── 7. Parsing: top-level entry point ────────────────────────────────────
//
// parse-dylan(tokens) → <ast-body>
//   Wraps the token vector in a stream and parses a source-record (body).

define function parse-dylan (tokens :: <stretchy-vector>) => (result :: <ast-body>)
  let ts = make-token-stream(tokens);
  parse-body(ts)
end function;

// ── 8. Parsing: body and constituents ─────────────────────────────────────
//
// body:
//     constituents SEMICOLON-OPT
//
// Parse a sequence of semicolon-separated constituents until a body
// terminator is seen.

define function parse-body (ts :: <token-stream>) => (b :: <ast-body>)
  let b = make-ast-body();
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-body-terminator?(t))
      done? := #t;
    else
      let node = parse-constituent(ts);
      add!(body-constituents(b), node);
      // Consume an optional semicolon between constituents.
      if (is-punct?(ts-peek(ts), #"semicolon"))
        ts-advance(ts);
      end;
    end;
  end;
  b
end function;

// constituent:
//     definition
//     local-declaration
//     expression
//
// Dispatch by looking at the first token.

define function parse-constituent (ts :: <token-stream>) => (n :: <ast-node>)
  let t = ts-peek(ts);
  if (is-define-token?(t))
    parse-definition(ts)
  elseif (is-local-decl-word?(t))
    parse-local-decl(ts)
  elseif (is-local-methods-word?(t))
    parse-local-methods(ts)
  else
    parse-expression(ts)
  end
end function;

// ── 9. Parsing: definitions ───────────────────────────────────────────────
//
// definition:
//     DEFINE modifiers DEFINE-BODY-WORD body-fragment ... definition-tail
//     DEFINE modifiers DEFINE-LIST-WORD list-fragment
//
// definition-tail:
//     END
//     END NAME-NOT-END
//     END DEFINE-BODY-WORD NAME-NOT-END

define function parse-definition (ts :: <token-stream>) => (n :: <ast-node>)
  // Consume `define`.
  ts-advance(ts);
  // Parse optional modifiers: unreserved names before the define-word.
  let modifiers = make(<stretchy-vector>);
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-ordinary-name?(t)
          & ~ is-define-body-word?(t)
          & ~ is-define-list-word?(t))
      add!(modifiers, ts-advance(ts));
    else
      done? := #t;
    end;
  end;
  let word = ts-peek(ts);
  if (is-define-body-word?(word))
    // DEFINE modifiers BODY-WORD body ... end [word] [name]
    ts-advance(ts);   // consume the word
    let d = make-ast-body-definition(word);
    defn-modifiers(d) := modifiers;
    // Parse body-fragment until `end` (or EOF).
    defn-body(d) := parse-body(ts);
    // Parse definition-tail.
    parse-definition-tail(ts, d);
    d
  elseif (is-define-list-word?(word))
    // DEFINE modifiers LIST-WORD list-fragment  (no `end`)
    ts-advance(ts);   // consume the word
    let d = make-ast-list-definition(word);
    defn-modifiers(d) := modifiers;
    // List-fragment: everything up to the terminating semicolon or EOF.
    defn-list(d) := parse-list-fragment(ts);
    d
  else
    parse-error("define: expected a define-body or define-list word")
  end
end function;

// definition-tail:
//     END
//     END NAME-NOT-END
//     END DEFINE-BODY-WORD NAME-NOT-END

define function parse-definition-tail (ts :: <token-stream>,
                                       d  :: <ast-body-definition>) => ()
  if (is-end-token?(ts-peek(ts)))
    ts-advance(ts);   // consume `end`
    // Optional: `end word` or `end word name`
    let t1 = ts-peek(ts);
    if (is-name-not-end?(t1) & ~ is-punct?(t1, #"semicolon"))
      let word = ts-advance(ts);
      defn-end-word(d) := word;
      let t2 = ts-peek(ts);
      if (is-name-not-end?(t2) & ~ is-punct?(t2, #"semicolon"))
        defn-end-name(d) := ts-advance(ts);
      end;
    end;
  end;
end function;

// list-fragment: expressions and punctuation up to `;` or EOF.
// Used for `define variable`, `define constant`, etc.
// We parse it as a body so we get structured nodes.

define function parse-list-fragment (ts :: <token-stream>) => (b :: <ast-body>)
  let b = make-ast-body();
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-body-terminator?(t) | is-punct?(t, #"semicolon"))
      done? := #t;
    else
      let node = parse-expression(ts);
      add!(body-constituents(b), node);
      // Commas inside list-fragment (multiple declarators).
      if (is-punct?(ts-peek(ts), #"comma"))
        ts-advance(ts);
      end;
    end;
  end;
  b
end function;

// ── 10. Parsing: local declarations ──────────────────────────────────────
//
// local-declaration:
//     LOCAL-DECLARATION-WORD list-fragment
//
// e.g.  let x = 5
//        let x :: <integer> = foo()
//        let (a, b) = values(1, 2)

define function parse-local-decl (ts :: <token-stream>) => (n :: <ast-node>)
  let word = ts-advance(ts);   // consume `let`
  let list = parse-list-fragment(ts);
  let d = make(<ast-local-decl>, word: word, list: list);
  d
end function;

// local-declaration — local methods:
//     LOCAL-METHODS-WORD local-method , local-method ...
//
// local-method:
//     FUNCTION-WORD body-fragment definition-tail
//     variable-name body-fragment definition-tail

define function parse-local-methods (ts :: <token-stream>) => (n :: <ast-node>)
  let kw = ts-advance(ts);    // consume `local`
  let m = make-ast-local-methods();
  node-token(m) := kw;
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let item = parse-local-method-item(ts);
    add!(lmethods-items(m), item);
    if (is-punct?(ts-peek(ts), #"comma"))
      ts-advance(ts);
    else
      done? := #t;
    end;
  end;
  m
end function;

define function parse-local-method-item (ts :: <token-stream>) => (n :: <ast-node>)
  let t = ts-peek(ts);
  if (is-function-word?(t))
    // `method name params body end method name`
    let word = ts-advance(ts);
    let body = parse-body(ts);
    // Consume the end clause for this local method.
    let dummy = make-ast-body-definition(word);
    parse-definition-tail(ts, dummy);
    let s = make(<ast-statement>, word: word, body: body);
    stmt-end-word(s) := defn-end-word(dummy);
    stmt-end-name(s) := defn-end-name(dummy);
    s
  elseif (is-name-token?(t))
    // `name params body end method name`  (implicit `method` word)
    let word = ts-advance(ts);
    let body = parse-body(ts);
    let dummy = make-ast-body-definition(word);
    parse-definition-tail(ts, dummy);
    let s = make(<ast-statement>, word: word, body: body);
    stmt-end-word(s) := defn-end-word(dummy);
    stmt-end-name(s) := defn-end-name(dummy);
    s
  else
    parse-error("local: expected method name or function word")
  end
end function;

// ── 11. Parsing: expressions ──────────────────────────────────────────────
//
// expression:
//     expression-guts  ← flattened by binop-fragment
//
// expression-guts:
//     binary-operand
//     expression-guts BINARY-OPERATOR binary-operand    ← left-associative
//
// We build a left-associative <ast-binary-op> tree.

define function parse-expression (ts :: <token-stream>) => (n :: <ast-node>)
  let left = parse-binary-operand(ts);
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-binary-op?(t))
      let op = ts-advance(ts);
      let right = parse-binary-operand(ts);
      left := make(<ast-binary-op>, left: left, operator: op, right: right);
    else
      done? := #t;
    end;
  end;
  left
end function;

// binary-operand:
//     SYMBOL                          ← keyword argument name (foo:)
//     UNARY-OPERATOR operand
//     operand

define function parse-binary-operand (ts :: <token-stream>) => (n :: <ast-node>)
  let t = ts-peek(ts);
  if (instance?(t, <keyword-name-token>))
    // A keyword-name token in a non-argument context becomes a symbol literal.
    let tok = ts-advance(ts);
    make(<ast-symbol-lit>, name: keyword-name-token-name(tok))
  elseif (is-unary-op?(t))
    let op      = ts-advance(ts);
    let operand = parse-operand(ts);
    make(<ast-unary-op>, op: op, operand: operand)
  else
    parse-operand(ts)
  end
end function;

// operand:
//     operand LPAREN arguments-OPT RPAREN     ← function call
//     operand LBRACKET arguments RBRACKET     ← subscript
//     operand DOT variable-name               ← dot call
//     leaf

define function parse-operand (ts :: <token-stream>) => (n :: <ast-node>)
  let node = parse-leaf(ts);
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-punct?(t, #"lparen"))
      // f(args)
      ts-advance(ts);
      let c = make-ast-call(node);
      node-token(c) := t;
      if (~ is-punct?(ts-peek(ts), #"rparen"))
        parse-arguments-into(ts, call-args(c));
      end;
      ts-expect-punct(ts, #"rparen", "expected ) after arguments");
      node := c;
    elseif (is-punct?(t, #"lbracket"))
      // x[args]
      ts-advance(ts);
      let s = make-ast-subscript(node);
      node-token(s) := t;
      if (~ is-punct?(ts-peek(ts), #"rbracket"))
        parse-arguments-into(ts, sub-args(s));
      end;
      ts-expect-punct(ts, #"rbracket", "expected ] after subscript");
      node := s;
    elseif (is-punct?(t, #"dot"))
      // x.name
      ts-advance(ts);
      let name-tok = ts-peek(ts);
      if (is-name-token?(name-tok))
        ts-advance(ts);
        let d = make(<ast-dot-call>, receiver: node, name: name-tok);
        node-token(d) := t;
        node := d;
      else
        done? := #t;
      end;
    else
      done? := #t;
    end;
  end;
  node
end function;

// ── 12. Parsing: leaf ─────────────────────────────────────────────────────
//
// leaf:
//     literal
//     variable-name
//     LPAREN expression RPAREN
//     function-macro-call     ← FUNCTION-WORD ( body-fragment )
//     statement               ← BEGIN-WORD body END

define function parse-leaf (ts :: <token-stream>) => (n :: <ast-node>)
  let t = ts-peek(ts);
  if (instance?(t, <integer-token>))
    let tok = ts-advance(ts);
    make(<ast-integer-lit>, value: integer-token-value(tok),
                            radix: integer-token-radix(tok))
  elseif (instance?(t, <float-token>))
    let tok = ts-advance(ts);
    make(<ast-float-lit>, raw: float-token-raw-text(tok))
  elseif (instance?(t, <ratio-token>))
    let tok = ts-advance(ts);
    make(<ast-ratio-lit>, raw: ratio-token-raw-text(tok))
  elseif (instance?(t, <string-literal-token>))
    parse-string-literal(ts)
  elseif (instance?(t, <character-literal-token>))
    let tok = ts-advance(ts);
    make(<ast-char-lit>, codepoint: character-literal-token-codepoint(tok))
  elseif (instance?(t, <boolean-literal-token>))
    let tok = ts-advance(ts);
    make(<ast-boolean-lit>, value: boolean-literal-token-value(tok))
  elseif (instance?(t, <symbol-literal-token>))
    let tok = ts-advance(ts);
    make(<ast-symbol-lit>, name: symbol-literal-token-name(tok))
  elseif (instance?(t, <keyword-name-token>))
    // keyword: in expression context → symbol literal
    let tok = ts-advance(ts);
    make(<ast-symbol-lit>, name: keyword-name-token-name(tok))
  elseif (instance?(t, <literal-vector-open>))
    // #(  — list literal
    parse-list-literal(ts)
  elseif (instance?(t, <literal-sequence-open>))
    // #[  — vector literal
    parse-vector-literal(ts)
  elseif (is-keyword?(t, #"hash-next") | is-keyword?(t, #"hash-rest")
          | is-keyword?(t, #"hash-key") | is-keyword?(t, #"hash-all-keys"))
    // #next, #rest, #key, #all-keys — treat as symbol
    let tok = ts-advance(ts);
    make(<ast-symbol-lit>,
         name: token-name(tok))
  elseif (is-punct?(t, #"lparen"))
    // Parenthesised expression
    ts-advance(ts);
    let inner = parse-expression(ts);
    ts-expect-punct(ts, #"rparen", "expected ) after parenthesised expression");
    inner
  elseif (is-function-word?(t))
    // FUNCTION-WORD ( body ) — function macro call  (method (...) => (...) body end)
    parse-function-literal(ts)
  elseif (is-begin-word?(t))
    // BEGIN-WORD body END [word] [name]
    parse-statement(ts)
  elseif (is-name-token?(t))
    // variable reference: any name including keywords used as names
    let tok = ts-advance(ts);
    make(<ast-variable-ref>, tok: tok)
  else
    // Unrecognised leaf — consume and return error node.
    let tok = ts-advance(ts);
    parse-error("unexpected token in expression")
  end
end function;

// string-literal: adjacent strings are concatenated (§6.4.2)
define function parse-string-literal (ts :: <token-stream>) => (n :: <ast-string-lit>)
  let first = ts-advance(ts);
  let value = string-literal-token-decoded(first);
  until (~ instance?(ts-peek(ts), <string-literal-token>))
    let next = ts-advance(ts);
    value := concatenate(value, string-literal-token-decoded(next));
  end;
  let n = make(<ast-string-lit>, value: value);
  node-token(n) := first;
  n
end function;

// #( constants-OPT )  or  #( constants . constant )
define function parse-list-literal (ts :: <token-stream>) => (n :: <ast-list-lit>)
  let open-tok = ts-advance(ts);   // consume <literal-vector-open>
  let l = make-ast-list-lit();
  node-token(l) := open-tok;
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-punct?(t, #"rparen"))
      done? := #t;
    elseif (is-punct?(t, #"dot"))
      // improper list tail: . constant )
      ts-advance(ts);
      lit-tail(l) := parse-constant(ts);
      done? := #t;
    else
      add!(lit-elems(l), parse-constant(ts));
      if (is-punct?(ts-peek(ts), #"comma"))
        ts-advance(ts);
      end;
    end;
  end;
  ts-expect-punct(ts, #"rparen", "expected ) after list literal");
  l
end function;

// #[ constants-OPT ]
define function parse-vector-literal (ts :: <token-stream>) => (n :: <ast-vector-lit>)
  let open-tok = ts-advance(ts);   // consume <literal-sequence-open>
  let v = make-ast-vector-lit();
  node-token(v) := open-tok;
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-punct?(t, #"rbracket"))
      done? := #t;
    else
      add!(lit-elems(v), parse-constant(ts));
      if (is-punct?(ts-peek(ts), #"comma"))
        ts-advance(ts);
      end;
    end;
  end;
  ts-expect-punct(ts, #"rbracket", "expected ] after vector literal");
  v
end function;

// constant:  literal | SYMBOL
// Used inside #(...) and #[...] literal bodies.
define function parse-constant (ts :: <token-stream>) => (n :: <ast-node>)
  let t = ts-peek(ts);
  if (instance?(t, <symbol-literal-token>) | instance?(t, <keyword-name-token>))
    let tok = ts-advance(ts);
    if (instance?(tok, <keyword-name-token>))
      make(<ast-symbol-lit>, name: keyword-name-token-name(tok))
    else
      make(<ast-symbol-lit>, name: symbol-literal-token-name(tok))
    end
  else
    parse-leaf(ts)
  end
end function;

// function-literal: `method params => (types) body end [method] [name]`
//                   `function params => (types) body end [function] [name]`
// These are anonymous function expressions in leaf position.
define function parse-function-literal (ts :: <token-stream>) => (n :: <ast-statement>)
  let word = ts-advance(ts);   // consume `method` or `function`
  let body = parse-body(ts);
  let s = make(<ast-statement>, word: word, body: body);
  node-token(s) := word;
  // Consume end-clause if present (function literals always have `end`).
  if (is-end-token?(ts-peek(ts)))
    ts-advance(ts);
    let t1 = ts-peek(ts);
    if (is-name-not-end?(t1) & ~ is-punct?(t1, #"semicolon"))
      stmt-end-word(s) := ts-advance(ts);
      let t2 = ts-peek(ts);
      if (is-name-not-end?(t2) & ~ is-punct?(t2, #"semicolon"))
        stmt-end-name(s) := ts-advance(ts);
      end;
    end;
  end;
  s
end function;

// ── 13. Parsing: statements ───────────────────────────────────────────────
//
// statement:
//     BEGIN-WORD body-fragment-OPT end-clause
//
// end-clause:
//     END [BEGIN-WORD]
//     END MACRO-CASE-BEGIN-WORD

define function parse-statement (ts :: <token-stream>) => (n :: <ast-statement>)
  let word = ts-advance(ts);   // consume begin-word
  let body = parse-body(ts);
  let s = make(<ast-statement>, word: word, body: body);
  node-token(s) := word;
  // Consume the `end` and optional tail name.
  if (is-end-token?(ts-peek(ts)))
    ts-advance(ts);
    let t = ts-peek(ts);
    if (is-name-not-end?(t) & ~ is-punct?(t, #"semicolon"))
      stmt-end-word(s) := ts-advance(ts);
    end;
  end;
  s
end function;

// ── 14. Parsing: arguments ────────────────────────────────────────────────
//
// arguments-guts:
//     argument
//     arguments-guts COMMA argument
//
// argument:
//     SYMBOL expression       ← keyword argument
//     expression-no-symbol    ← positional argument (non-symbol lead)
//     SYMBOL                  ← bare keyword

define function parse-arguments-into (ts :: <token-stream>,
                                      args :: <stretchy-vector>) => ()
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (instance?(t, <keyword-name-token>))
      // `keyword: expr`  or bare `keyword:` if next is , or )
      let key-tok = ts-advance(ts);
      let next = ts-peek(ts);
      if (is-punct?(next, #"comma") | is-punct?(next, #"rparen")
            | is-punct?(next, #"rbracket") | is-body-terminator?(next))
        // Bare keyword argument (just the keyword, no value)
        let arg = make(<ast-kw-arg>, key: key-tok,
                       value: make(<ast-symbol-lit>,
                                   name: keyword-name-token-name(key-tok)));
        add!(args, arg);
      else
        let val = parse-expression(ts);
        let arg = make(<ast-kw-arg>, key: key-tok, value: val);
        add!(args, arg);
      end;
    else
      let val = parse-expression(ts);
      let arg = make(<ast-pos-arg>, value: val);
      add!(args, arg);
    end;
    // Consume comma separator; stop on anything else.
    if (is-punct?(ts-peek(ts), #"comma"))
      ts-advance(ts);
    else
      done? := #t;
    end;
  end;
end function;

// ── 15. Parsing: variable declarations ───────────────────────────────────
//
// variable:
//     variable-name
//     variable-name COLON-COLON type
//
// Used in parameter lists and let bindings.

define function parse-variable (ts :: <token-stream>) => (v :: <ast-typed-name>)
  let name-tok = ts-peek(ts);
  if (is-name-token?(name-tok))
    ts-advance(ts);
    let v = make(<ast-typed-name>, tok: name-tok);
    if (is-punct?(ts-peek(ts), #"colon-colon"))
      ts-advance(ts);   // consume `::`
      typed-name-type(v) := parse-expression(ts);
    end;
    v
  else
    make(<ast-typed-name>, tok: name-tok)   // best-effort; caller checks
  end
end function;

// ── 16. AST dump ─────────────────────────────────────────────────────────
//
// A simple indented text dump for debugging and snapshot testing.
// Writes to a <stretchy-vector> of bytes (byte-string accumulator),
// returns the completed string.

define function dump-ast (node :: <ast-node>) => (s :: <byte-string>)
  let acc = make(<stretchy-vector>);
  dump-node(node, acc, 0);
  // Flatten accumulator to a single <byte-string>.
  let total = size(acc);
  let result = make(<byte-string>, size: total);
  let i = 0;
  until (i >= total)
    result[i] := acc[i];
    i := i + 1;
  end;
  result
end function;

// Append all bytes of s to acc.
define function acc-string (acc :: <stretchy-vector>, s :: <byte-string>) => ()
  let n = size(s);
  let i = 0;
  until (i >= n)
    add!(acc, s[i]);
    i := i + 1;
  end;
end function;

define function acc-indent (acc :: <stretchy-vector>, depth :: <integer>) => ()
  let i = 0;
  until (i >= depth)
    add!(acc, 32);  // space
    add!(acc, 32);  // space
    i := i + 1;
  end;
end function;

define function acc-newline (acc :: <stretchy-vector>) => ()
  add!(acc, 10);  // '\n'
end function;

define function dump-node (node :: <ast-node>,
                           acc  :: <stretchy-vector>,
                           depth :: <integer>) => ()
  acc-indent(acc, depth);
  if (instance?(node, <ast-body>))
    acc-string(acc, "BODY");
    acc-newline(acc);
    let items = body-constituents(node);
    let n = size(items);
    let i = 0;
    until (i >= n)
      dump-node(items[i], acc, depth + 1);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-body-definition>))
    acc-string(acc, "DEFINE-BODY ");
    acc-string(acc, token-name(defn-word(node)));
    acc-newline(acc);
    dump-node(defn-body(node), acc, depth + 1);
  elseif (instance?(node, <ast-list-definition>))
    acc-string(acc, "DEFINE-LIST ");
    acc-string(acc, token-name(defn-word(node)));
    acc-newline(acc);
    dump-node(defn-list(node), acc, depth + 1);
  elseif (instance?(node, <ast-local-decl>))
    acc-string(acc, "LET");
    acc-newline(acc);
    dump-node(ldecl-list(node), acc, depth + 1);
  elseif (instance?(node, <ast-local-methods>))
    acc-string(acc, "LOCAL");
    acc-newline(acc);
    let items = lmethods-items(node);
    let n = size(items);
    let i = 0;
    until (i >= n)
      dump-node(items[i], acc, depth + 1);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-binary-op>))
    acc-string(acc, "BINOP");
    acc-newline(acc);
    dump-node(binop-left(node), acc, depth + 1);
    acc-indent(acc, depth + 1);
    if (instance?(binop-operator(node), <punctuation-token>))
      acc-string(acc, write-to-string(punctuation-token-form(binop-operator(node))));
    else
      acc-string(acc, "?op?");
    end;
    acc-newline(acc);
    dump-node(binop-right(node), acc, depth + 1);
  elseif (instance?(node, <ast-unary-op>))
    acc-string(acc, "UNOP");
    acc-newline(acc);
    dump-node(unary-operand(node), acc, depth + 1);
  elseif (instance?(node, <ast-call>))
    acc-string(acc, "CALL");
    acc-newline(acc);
    dump-node(call-fn(node), acc, depth + 1);
    let args = call-args(node);
    let n = size(args);
    let i = 0;
    until (i >= n)
      dump-node(args[i], acc, depth + 1);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-dot-call>))
    acc-string(acc, "DOT ");
    acc-string(acc, token-name(dot-name(node)));
    acc-newline(acc);
    dump-node(dot-receiver(node), acc, depth + 1);
  elseif (instance?(node, <ast-subscript>))
    acc-string(acc, "SUBSCRIPT");
    acc-newline(acc);
    dump-node(sub-receiver(node), acc, depth + 1);
    let args = sub-args(node);
    let n = size(args);
    let i = 0;
    until (i >= n)
      dump-node(args[i], acc, depth + 1);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-variable-ref>))
    acc-string(acc, "NAME ");
    acc-string(acc, token-name(varref-tok(node)));
    acc-newline(acc);
  elseif (instance?(node, <ast-integer-lit>))
    acc-string(acc, "INT ");
    acc-string(acc, integer-to-string(lit-value(node)));
    acc-newline(acc);
  elseif (instance?(node, <ast-float-lit>))
    acc-string(acc, "FLOAT ");
    acc-string(acc, lit-raw(node));
    acc-newline(acc);
  elseif (instance?(node, <ast-ratio-lit>))
    acc-string(acc, "RATIO ");
    acc-string(acc, lit-raw(node));
    acc-newline(acc);
  elseif (instance?(node, <ast-string-lit>))
    acc-string(acc, "STRING \"");
    acc-string(acc, lit-value(node));
    acc-string(acc, "\"");
    acc-newline(acc);
  elseif (instance?(node, <ast-char-lit>))
    acc-string(acc, "CHAR");
    acc-newline(acc);
  elseif (instance?(node, <ast-boolean-lit>))
    if (lit-value(node))
      acc-string(acc, "BOOL #t");
    else
      acc-string(acc, "BOOL #f");
    end;
    acc-newline(acc);
  elseif (instance?(node, <ast-symbol-lit>))
    acc-string(acc, "SYMBOL ");
    acc-string(acc, lit-name(node));
    acc-newline(acc);
  elseif (instance?(node, <ast-list-lit>))
    acc-string(acc, "LIST-LIT");
    acc-newline(acc);
    let elems = lit-elems(node);
    let n = size(elems);
    let i = 0;
    until (i >= n)
      dump-node(elems[i], acc, depth + 1);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-vector-lit>))
    acc-string(acc, "VECTOR-LIT");
    acc-newline(acc);
    let elems = lit-elems(node);
    let n = size(elems);
    let i = 0;
    until (i >= n)
      dump-node(elems[i], acc, depth + 1);
      i := i + 1;
    end;
  elseif (instance?(node, <ast-statement>))
    acc-string(acc, "STMT ");
    acc-string(acc, token-name(stmt-word(node)));
    acc-newline(acc);
    dump-node(stmt-body(node), acc, depth + 1);
  elseif (instance?(node, <ast-pos-arg>))
    acc-string(acc, "ARG");
    acc-newline(acc);
    dump-node(pos-arg-value(node), acc, depth + 1);
  elseif (instance?(node, <ast-kw-arg>))
    acc-string(acc, "KWARG ");
    acc-string(acc, keyword-name-token-name(kw-arg-key(node)));
    acc-newline(acc);
    dump-node(kw-arg-value(node), acc, depth + 1);
  elseif (instance?(node, <ast-typed-name>))
    acc-string(acc, "TYPED-NAME ");
    acc-string(acc, token-name(typed-name-tok(node)));
    acc-newline(acc);
  elseif (instance?(node, <ast-error-node>))
    acc-string(acc, "ERROR: ");
    acc-string(acc, ast-error-msg(node));
    acc-newline(acc);
  else
    acc-string(acc, "???");
    acc-newline(acc);
  end;
end function;

// ── 17. Main ──────────────────────────────────────────────────────────────
//
// Entry point for `nod-driver parse-dylan <source-file>`.
// Compiled together with dylan-lexer.dylan (which supplies lex(),
// load-source-via-rope(), %argv1(), format-out etc.) as a two-file
// AOT build.  main() here is the sole entry point; dylan-lexer.dylan
// has no main() of its own.

define function main () => ()
  let path = %argv1();
  if (empty?(path))
    format-out("dylan-parser: missing input path\n");
  else
    let source = load-source-via-rope(path);
    if (empty?(source))
      format-out("dylan-parser: could not read %s\n", path);
    else
      let tokens = lex(source);
      let ast    = parse-dylan(tokens);
      let dump   = dump-ast(ast);
      format-out("%s", dump);
    end;
  end;
end function main;

main();

// eof
