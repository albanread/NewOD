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

// SLOT-ALLOCATION-WORD: `slot` itself, the word that introduces a slot
// spec in a `define class` body.  The lexer classifies `slot` as a
// <keyword-token> keyword: #"slot" (see classify-keyword).
define function is-slot-word? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"slot")
end function;

// SLOT-ADJECTIVE: a word that may precede `slot` in a slot spec, e.g.
//   constant slot ...  /  each-subclass slot ...  /  class slot ...
//   virtual slot ...   /  inherited slot ...      /  sealed slot ...
// The lexer maps each of these to a <keyword-token> with the matching
// symbol (see classify-keyword).  `inherited` can introduce an inherited
// slot directly, so it is accepted both as an adjective and (handled in
// parse-slot-spec) as a standalone allocation word.
define function is-slot-adjective? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <keyword-token>))
    let kw = keyword-token-keyword(t);
    kw = #"constant" | kw = #"each-subclass" | kw = #"class"
      | kw = #"virtual" | kw = #"inherited" | kw = #"sealed"
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

// MODIFIER-WORD: an adjective that may precede the define-word, e.g.
//   define SEALED generic ...   define OPEN ABSTRACT class ...
// Identifiers count (arbitrary unreserved adjectives), as do the adjective
// keywords the lexer reserves (sealed / open / abstract / concrete / primary
// / free).  Define-words themselves (class/generic/method/...) are excluded,
// so modifier collection stops cleanly at the define-word.
define function is-modifier-word? (t :: <token>) => (yes? :: <boolean>)
  if (instance?(t, <identifier-token>) | instance?(t, <escaped-ident-token>))
    #t
  elseif (instance?(t, <keyword-token>))
    let kw = keyword-token-keyword(t);
    kw = #"sealed" | kw = #"open" | kw = #"abstract"
      | kw = #"concrete" | kw = #"primary" | kw = #"free"
  else
    #f
  end
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
      // `=>` inside a body is a select/case arm separator (`key => body`).
      // It never reaches expression context elsewhere: method / function /
      // generic return specs consume their `=>` via parse-return-spec before
      // the body is parsed.  Modelling the arm as a left-associative BINOP
      // (`(key) => (body)`) is a faithful token-level capture for the macro
      // expander to interpret later.
      | f = #"arrow"
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
// NOTE: `otherwise` is deliberately NOT here.  In select / case it begins an
// arm (`otherwise => body`), exactly like a key arm (`1 => body`), so it stays
// inside the body and parses as BINOP(NAME otherwise, =>, body).  Treating it
// as a terminator/clause-separator would strand the leading `=>` of its arm.
define function is-body-terminator? (t :: <token>) => (yes? :: <boolean>)
  is-end-token?(t)
    | is-keyword?(t, #"else")
    | is-keyword?(t, #"elseif")
    | is-keyword?(t, #"cleanup")
    | is-keyword?(t, #"exception")
    | is-keyword?(t, #"finally")
    | instance?(t, <eof-token>)
    | is-punct?(t, #"rparen")
    | is-punct?(t, #"rbracket")
    | is-punct?(t, #"rbrace")
end function;

// CLAUSE-SEPARATOR: a keyword that introduces a fresh clause inside a
// statement (between the leading body and the closing `end`).  These are
// exactly the body-terminating clause keywords from is-body-terminator? —
// `parse-body` halts on each, and parse-statement then resumes a new clause
// when it sees one here:
//   if (c) ... elseif (c) ... else ...
//   block () ... cleanup ... exception (e) ... finally ...
// `otherwise` is NOT a separator: it stays in the select/case body as an arm
// (`otherwise => body`), parsed like any other `key => body` arm.
define function is-clause-separator? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"else")      | is-keyword?(t, #"elseif")
    | is-keyword?(t, #"cleanup")   | is-keyword?(t, #"exception")
    | is-keyword?(t, #"finally")
end function;

// FOR-CONNECTOR: a token that joins a for-clause variable to an expression in
// the `for` header (`i FROM 1 TO 10 BY 2`, `x = init THEN next`, `item IN c`).
// `above` / `below` are not lexer keywords (they lex as identifiers), so they
// are matched by name.  None of these are binary operators, so each one
// cleanly delimits the parts of a for-clause.
define function is-for-connector? (t :: <token>) => (yes? :: <boolean>)
  is-keyword?(t, #"from") | is-keyword?(t, #"to") | is-keyword?(t, #"by")
    | is-keyword?(t, #"in") | is-keyword?(t, #"then")
    | is-punct?(t, #"equal")
    | (instance?(t, <identifier-token>)
         & (identifier-token-name(t) = "above"
              | identifier-token-name(t) = "below"))
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

// A parenthesised fragment that is NOT a single bare expression: either a
// comma-separated list `(a, b)` or a typed binding `(e :: <error>)`, as found
// in clause heads (`block (return)`, `exception (e :: <error>)`).  A single
// untyped item is returned transparently as that item (ordinary grouping), so
// this node only appears for multi-item or typed heads.  Items are <ast-node>
// (typed items are <ast-typed-name>).
define class <ast-paren-list> (<ast-node>)
  slot paren-list-items :: <stretchy-vector>, init-keyword: items:;
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
  // Subsequent clauses introduced by a clause-separator keyword:
  //   if ... elseif ... else ...     block ... cleanup ... exception ...
  //   select/case ... otherwise ...
  // A <stretchy-vector> of <ast-statement-clause>, or #f when the statement
  // has only its leading body (begin/for/while/method-literal, etc.).  Held
  // as <object> so the unset default is the immutable #f rather than one
  // shared mutable vector across instances (see <ast-param-list> note).
  slot stmt-clauses     :: <object> = #f;   // <stretchy-vector> or #f
  // `for (clauses)` iteration header — a <stretchy-vector> of
  // <ast-for-clause>, or #f for every other statement (whose parenthesised
  // head, if any, is just an ordinary expression in the leading body).
  slot stmt-for-header  :: <object> = #f;   // <stretchy-vector> or #f
end class;

// One trailing clause of a multi-clause statement.  `clause-word` is the
// separator keyword that introduced it (else / elseif / cleanup / exception
// / finally / otherwise); `clause-body` is the body fragment up to the next
// separator or `end`.  A clause head such as `elseif (cond)` keeps its
// `(cond)` as the first constituent of `clause-body`, exactly as the leading
// `if`'s own condition lands as the first constituent of `stmt-body`.
define class <ast-statement-clause> (<ast-node>)
  slot clause-word :: <token>,    init-keyword: word:;
  slot clause-body :: <ast-body>, init-keyword: body:;
end class;

// One clause of a `for` iteration header.  Modelled uniformly as an optional
// leading variable name followed by a sequence of `connector expr` parts, so
// every DRM clause shape collapses to the same shape:
//   i from 1 to 10     → var i ;  parts (from 1) (to 10)
//   x = init then next → var x ;  parts (= init) (then next)
//   item in coll       → var item ; parts (in coll)
//   until count > 100  → var #f ;  parts (until count > 100)   (end-test)
// The `for` macro interprets the connector sequence later; the parser just
// captures the fragment faithfully.
define class <ast-for-clause> (<ast-node>)
  slot for-clause-var   :: <object> = #f;   // <token> (loop variable) or #f
  slot for-clause-parts :: <stretchy-vector>, init-keyword: parts:;
end class;

// One `connector expr` part of a for-clause.  `for-part-conn` is the
// connector token (the keyword from/to/by/in/then/while/until, the `=` punct,
// or the identifier above/below); `for-part-expr` is the expression after it.
define class <ast-for-part> (<ast-node>)
  slot for-part-conn :: <token>,    init-keyword: conn:;
  slot for-part-expr :: <ast-node>, init-keyword: expr:;
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
//
// NOTE: every slot is given an explicit `init-keyword:` and supplied at
// `make` time (see make-ast-key-spec).  See the GAP note on slot
// defaulting near <ast-param-list>; `init-value:` / `= #f` defaults are
// NOT reliably applied for these classes in the current compiler.
define class <ast-key-spec> (<ast-node>)
  slot key-spec-tok     :: <token>,  init-keyword: tok:;
  slot key-spec-type    :: <object>, init-keyword: type:;     // #f or <ast-node>
  slot key-spec-default :: <object>, init-keyword: default:;  // #f or <ast-node>
end class;

// `( var, ..., #rest r, #key k ..., #all-keys, #next n )`
// A method / function parameter list.
//   params-required : vector of <ast-typed-name>
//   params-rest     : <token> name after #rest, or #f
//   params-keys     : vector of <ast-key-spec> after #key
//   params-key?     : #t if #key appeared (even with no specs)
//   params-all-keys?: #t if #all-keys appeared
//   params-next     : <token> name after #next, or #f
//
// COMPILER GAP (Sprint 46) — slot defaults are NOT applied for these
// classes.  A slot declared `slot x :: <object> = #f;` or
// `slot x :: <object>, init-value: #f;` reads back GARBAGE (a non-#f,
// faulting value) when the instance is built with `make` and other slots
// carry `init-keyword:` with no default.  Symptom: `instance?`/`==` on the
// "defaulted" slot's value raises EXCEPTION_ACCESS_VIOLATION, and a
// defaulted `<boolean>` slot reads `#t` when it should be `#f`.
// Workaround used here (the file's existing idiom, see §4): give EVERY
// slot an explicit `init-keyword:` and supply ALL of them at `make` time
// in the constructor — never rely on `init-value:` / `= default`.
// The `#f`/#t flags are typed `<object>` (not `<boolean>`) for the same
// reason.  Minimal repro for the lead is in the final report.
define class <ast-param-list> (<ast-node>)
  slot params-required :: <stretchy-vector>, init-keyword: required:;
  slot params-keys     :: <stretchy-vector>, init-keyword: keys:;
  slot params-rest     :: <object>,  init-keyword: rest:;     // <token> or #f
  slot params-key?     :: <object>,  init-keyword: key?:;     // #f / #t
  slot params-all-keys? :: <object>, init-keyword: all-keys?:; // #f / #t
  slot params-next     :: <object>,  init-keyword: next:;     // <token> or #f
end class;

// `=> spec` — a return specification.
//   ret-present?  : #t when an `=>` was actually present
//   ret-values    : vector of <ast-typed-name> (value name [:: type])
//   ret-rest      : <token> name after #rest, or #f
//   ret-rest-type : type after `#rest name :: type`, or #f
define class <ast-return-spec> (<ast-node>)
  slot ret-present?  :: <object>, init-keyword: present?:;   // #f / #t
  slot ret-values    :: <stretchy-vector>, init-keyword: values:;
  slot ret-rest      :: <object>, init-keyword: rest:;       // <token> or #f
  slot ret-rest-type :: <object>, init-keyword: rest-type:;  // <ast-node> or #f
end class;

// `slot NAME [:: type] [= default] [, init-option ...]` — one slot spec.
//
// adjectives    : vector of <token> (the words before `slot`, e.g.
//                 `constant`, `each-subclass`, `class`, `virtual`,
//                 `inherited`, `sealed`).  Recorded verbatim as tokens.
// slot-word     : the `slot` <keyword-token> itself (allocation word).
// slot-name-tok : the slot's name token (e.g. `point-x`).
// slot-type     : the type after `::`, or #f.
// slot-init-kw  : the init-keyword / required-init-keyword name token
//                 (a <keyword-name-token>, e.g. `x:`), or #f.
// slot-required?: #t when the keyword came from `required-init-keyword:`.
// slot-init     : the init-value / init-function expr, OR the `= default`
//                 shorthand expression (sugar for init-value:), or #f.
//
// Same compiler-gap workaround as <ast-param-list>: EVERY slot carries an
// explicit `init-keyword:` and is supplied at `make` time; flags are typed
// <object> so a faulting "defaulted" value never leaks in.
define class <ast-slot-spec> (<ast-node>)
  slot slot-adjectives :: <stretchy-vector>, init-keyword: adjectives:;
  slot slot-word       :: <object>, init-keyword: word:;       // <token> or #f
  slot slot-name-tok   :: <object>, init-keyword: name-tok:;   // <token> or #f
  slot slot-type       :: <object>, init-keyword: type:;       // <ast-node> or #f
  slot slot-init-kw    :: <object>, init-keyword: init-kw:;    // <token> or #f
  slot slot-required?  :: <object>, init-keyword: required?:;  // #f / #t
  slot slot-init       :: <object>, init-keyword: init:;       // <ast-node> or #f
end class;

// `define [modifiers] class NAME (super, ...) slot-spec ... end [class] [NAME]`
//   class-name : the class name <token>
//   supers     : vector of <ast-node> (one per superclass expression)
//   slots      : vector of <ast-slot-spec>
//   end-word / end-name : the `end class NAME` tail, like <ast-body-definition>.
//
// Init-keyword'd vector slots are built via make-ast-class-definition.
define class <ast-class-definition> (<ast-node>)
  slot defn-modifiers :: <stretchy-vector>, init-keyword: modifiers:;
  slot defn-word      :: <token>,  init-keyword: word:;      // the `class` keyword
  slot class-name     :: <object>, init-keyword: name:;      // <token> or #f
  slot class-supers   :: <stretchy-vector>, init-keyword: supers:;
  slot class-slots    :: <stretchy-vector>, init-keyword: slots:;
  slot defn-end-word  :: <object>, init-keyword: end-word:;  // <token> or #f
  slot defn-end-name  :: <object>, init-keyword: end-name:;  // <token> or #f
end class;

// `define [modifiers] generic NAME (params) => (returns) ;`
// A generic function declaration: a name and a signature (parameter list +
// return spec), but NO body and NO `end` — it is terminated by `;`.
define class <ast-generic-definition> (<ast-node>)
  slot defn-modifiers :: <stretchy-vector>, init-keyword: modifiers:;
  slot gen-word    :: <token>,  init-keyword: word:;   // the `generic` keyword
  slot gen-name    :: <object> = #f;   // <token> or #f
  slot gen-params  :: <object> = #f;   // <ast-param-list> or #f
  slot gen-return  :: <object> = #f;   // <ast-return-spec> or #f
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
  make(<ast-param-list>,
       required: make(<stretchy-vector>),
       keys: make(<stretchy-vector>),
       rest: #f, key?: #f, all-keys?: #f, next: #f)
end function;

define function make-ast-return-spec () => (r :: <ast-return-spec>)
  make(<ast-return-spec>,
       present?: #f,
       values: make(<stretchy-vector>),
       rest: #f, rest-type: #f)
end function;

define function make-ast-key-spec (name-tok :: <token>) => (k :: <ast-key-spec>)
  make(<ast-key-spec>, tok: name-tok, type: #f, default: #f)
end function;

// A fresh slot-spec with empty adjectives and every option cleared.
define function make-ast-slot-spec () => (s :: <ast-slot-spec>)
  make(<ast-slot-spec>,
       adjectives: make(<stretchy-vector>),
       word: #f, name-tok: #f, type: #f,
       init-kw: #f, required?: #f, init: #f)
end function;

// A fresh class definition with empty modifier/super/slot vectors.
define function make-ast-class-definition (word :: <token>)
 => (d :: <ast-class-definition>)
  make(<ast-class-definition>,
       modifiers: make(<stretchy-vector>),
       word: word,
       name: #f,
       supers: make(<stretchy-vector>),
       slots: make(<stretchy-vector>),
       end-word: #f, end-name: #f)
end function;

define function make-ast-generic-definition (word :: <token>)
 => (d :: <ast-generic-definition>)
  make(<ast-generic-definition>,
       modifiers: make(<stretchy-vector>),
       word: word)
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
    elseif  (kw = #"cleanup")   "cleanup"
    elseif  (kw = #"exception") "exception"
    elseif  (kw = #"finally")   "finally"
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
    elseif  (kw = #"slot")      "slot"
    elseif  (kw = #"each-subclass") "each-subclass"
    elseif  (kw = #"virtual")   "virtual"
    elseif  (kw = #"inherited") "inherited"
    elseif  (kw = #"sealed")    "sealed"
    elseif  (kw = #"open")      "open"
    elseif  (kw = #"abstract")  "abstract"
    elseif  (kw = #"concrete")  "concrete"
    elseif  (kw = #"primary")   "primary"
    elseif  (kw = #"free")      "free"
    elseif  (kw = #"domain")    "domain"
    elseif  (kw = #"for")       "for"
    elseif  (kw = #"from")      "from"
    elseif  (kw = #"to")        "to"
    elseif  (kw = #"by")        "by"
    elseif  (kw = #"in")        "in"
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
  // Parse optional modifiers: adjective words (identifiers or reserved
  // adjective keywords like `sealed` / `open`) before the define-word.
  let modifiers = make(<stretchy-vector>);
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-modifier-word?(t)
          & ~ is-define-body-word?(t)
          & ~ is-define-list-word?(t))
      add!(modifiers, ts-advance(ts));
    else
      done? := #t;
    end;
  end;
  let word = ts-peek(ts);
  if (is-keyword?(word, #"class"))
    // DEFINE modifiers class NAME (supers) slot-specs ... end [class] [NAME]
    ts-advance(ts);   // consume `class`
    parse-class-definition(ts, word, modifiers)
  elseif (is-keyword?(word, #"generic"))
    // DEFINE modifiers generic NAME (params) => (returns) ;   (no body, no end)
    ts-advance(ts);   // consume `generic`
    parse-generic-definition(ts, word, modifiers)
  elseif (is-define-body-word?(word))
    // DEFINE modifiers BODY-WORD body ... end [word] [name]
    ts-advance(ts);   // consume the word
    let d = make-ast-body-definition(word);
    defn-modifiers(d) := modifiers;
    if (is-function-word?(word))
      // `define method NAME (params) => (returns) body end ...`
      // `define function NAME (params) => (returns) body end ...`
      // Optional method name (a name token before the `(`).
      if (is-name-token?(ts-peek(ts)) & ~ is-punct?(ts-peek(ts), #"lparen"))
        defn-method-name(d) := ts-advance(ts);
      end;
      // Parameter list and return spec, if present.
      if (is-punct?(ts-peek(ts), #"lparen"))
        defn-params(d) := parse-parameter-list(ts);
      end;
      defn-return(d) := parse-return-spec(ts);
    end;
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

// ── 9b. Parsing: class definitions ────────────────────────────────────────
//
// class-definition:
//     DEFINE modifiers CLASS class-name superclass-list class-clauses
//       END [CLASS] [class-name]
//
// superclass-list:
//     LPAREN superclasses-OPT RPAREN
//
// superclasses:
//     expression , ...                 ← each superclass is an expression
//
// class-clauses:
//     class-clause ; ...               ← `;`/`,` separated, zero or more
//
// class-clause (we model `slot` specs; other member clauses are skipped):
//     slot-adjectives SLOT-WORD slot-name [:: type] [= default]
//       [, init-option ...]
//
// init-option:
//     INIT-KEYWORD SYMBOL               ← init-keyword: foo:
//     REQUIRED-INIT-KEYWORD SYMBOL      ← required-init-keyword: foo:
//     INIT-VALUE expression             ← init-value: <expr>
//     INIT-FUNCTION expression          ← init-function: <expr>
//
// Modelled after nod-reader's parse_define_class / parse_class_body.

define function parse-class-definition (ts :: <token-stream>,
                                        word :: <token>,
                                        modifiers :: <stretchy-vector>)
 => (d :: <ast-class-definition>)
  let d = make-ast-class-definition(word);
  defn-modifiers(d) := modifiers;
  node-token(d) := word;
  // Class name — a name token (e.g. <point>).
  if (is-name-token?(ts-peek(ts)))
    class-name(d) := ts-advance(ts);
  else
    parse-error("define class: expected class name");
  end;
  // Superclass list — `(expr, expr, ...)`.
  parse-super-list(ts, class-supers(d));
  // Slot specs and other clauses until `end`.
  parse-class-clauses(ts, class-slots(d));
  // definition-tail: `end [class] [NAME]`.  Reuse the body-definition shape
  // by parsing into a scratch node, then copy the end word/name across.
  let tail = make-ast-body-definition(word);
  parse-definition-tail(ts, tail);
  defn-end-word(d) := defn-end-word(tail);
  defn-end-name(d) := defn-end-name(tail);
  d
end function;

// generic-definition:
//     DEFINE modifiers GENERIC NAME parameter-list ARROW return-spec ;
//
// A generic has the same signature shape as a method/function but NO body
// and NO `end` — it is terminated by the body's `;`.  The signature is
// optional in degenerate forms, so each piece is guarded.
define function parse-generic-definition (ts :: <token-stream>,
                                          word :: <token>,
                                          modifiers :: <stretchy-vector>)
 => (d :: <ast-generic-definition>)
  let d = make-ast-generic-definition(word);
  defn-modifiers(d) := modifiers;
  node-token(d) := word;
  // Generic name — a name token before the parameter list's `(`.
  if (is-name-token?(ts-peek(ts)) & ~ is-punct?(ts-peek(ts), #"lparen"))
    gen-name(d) := ts-advance(ts);
  else
    parse-error("define generic: expected generic name");
  end;
  // Parameter list, then return spec.  No body, no `end`.
  if (is-punct?(ts-peek(ts), #"lparen"))
    gen-params(d) := parse-parameter-list(ts);
  end;
  gen-return(d) := parse-return-spec(ts);
  d
end function;

// superclass-list: `(` expr (`,` expr)* `)`, or absent.  Each superclass is
// a full expression (a class name, `subclass(<c>)`, a union, etc.).  Reuses
// parse-expression so commas inside an argument list are not swallowed —
// the comma at this level separates superclasses.
define function parse-super-list (ts :: <token-stream>,
                                  supers :: <stretchy-vector>) => ()
  if (is-punct?(ts-peek(ts), #"lparen"))
    ts-advance(ts);   // consume `(`
    let done? = #f;
    until (done? | ts-at-end?(ts))
      let t = ts-peek(ts);
      if (is-punct?(t, #"rparen"))
        done? := #t;
      elseif (is-punct?(t, #"comma"))
        ts-advance(ts);   // skip stray separators
      else
        add!(supers, parse-expression(ts));
        if (is-punct?(ts-peek(ts), #"comma"))
          ts-advance(ts);
        end;
      end;
    end;
    ts-expect-punct(ts, #"rparen", "expected ) to close superclass list");
  end;
end function;

// class-clauses: zero or more clauses separated by `;` (and/or `,`), until
// `end` (or EOF).  We model slot specs; any other member clause (init-form,
// bare `keyword foo:;`, etc.) is skipped up to the next `;` so the parse
// stays in sync.
define function parse-class-clauses (ts :: <token-stream>,
                                     slots :: <stretchy-vector>) => ()
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-end-token?(t))
      done? := #t;
    elseif (is-punct?(t, #"semicolon") | is-punct?(t, #"comma"))
      ts-advance(ts);   // skip separators between clauses
    elseif (is-slot-word?(t) | is-slot-adjective?(t))
      // A slot clause (possibly preceded by adjectives).  Confirm a `slot`
      // word follows the adjectives before committing; otherwise skip the
      // rest of this clause so the cursor stays in sync.
      let spec = parse-slot-spec(ts);
      if (instance?(spec, <ast-slot-spec>))
        add!(slots, spec);
      else
        skip-class-clause(ts);
      end;
    else
      // Unmodelled member clause — skip to the next `;` or `end`.
      skip-class-clause(ts);
    end;
  end;
end function;

// Skip an unmodelled class clause: consume tokens up to (but not past) the
// next `;` or `end`/EOF, so parse-class-clauses can resume cleanly.
define function skip-class-clause (ts :: <token-stream>) => ()
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-end-token?(t) | is-punct?(t, #"semicolon"))
      done? := #t;
    else
      ts-advance(ts);
    end;
  end;
end function;

// slot-spec:
//     slot-adjectives SLOT-WORD slot-name [:: type] [= default]
//       [, init-option ...]
//
// Returns an <ast-slot-spec>, or an <ast-error-node> if no `slot` word is
// found after the adjectives (caller treats non-slot-spec results as skips).
define function parse-slot-spec (ts :: <token-stream>) => (n :: <ast-node>)
  let s = make-ast-slot-spec();
  // Leading adjectives: zero or more slot-adjective keyword tokens.  Stop as
  // soon as we hit the `slot` word.
  let done-adj? = #f;
  until (done-adj? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-slot-word?(t))
      done-adj? := #t;
    elseif (is-slot-adjective?(t))
      add!(slot-adjectives(s), ts-advance(ts));
    else
      done-adj? := #t;
    end;
  end;
  // The `slot` allocation word.  If absent (e.g. a lone `inherited` clause
  // we don't model), bail with an error node so the caller can skip.
  if (is-slot-word?(ts-peek(ts)))
    let sw = ts-advance(ts);
    slot-word(s) := sw;
    node-token(s) := sw;
    // Slot name.
    if (is-name-token?(ts-peek(ts)))
      slot-name-tok(s) := ts-advance(ts);
    else
      parse-error("slot: expected slot name");
    end;
    // Optional `:: type`.
    if (is-punct?(ts-peek(ts), #"colon-colon"))
      ts-advance(ts);   // consume `::`
      slot-type(s) := parse-type-spec(ts);
    end;
    // Optional `= default` shorthand (sugar for init-value:).
    if (is-punct?(ts-peek(ts), #"equal"))
      ts-advance(ts);   // consume `=`
      slot-init(s) := parse-expression(ts);
    end;
    // Trailing `, init-option ...` clauses.  A comma followed by a
    // keyword-name token (e.g. `init-keyword:`) is an init option for THIS
    // slot; a comma followed by anything else separates slot specs and is
    // left for parse-class-clauses to consume.
    let more? = #t;
    until (~ more? | ts-at-end?(ts))
      if (is-punct?(ts-peek(ts), #"comma")
            & instance?(ts-peek-after-comma(ts), <keyword-name-token>))
        ts-advance(ts);   // consume `,`
        parse-slot-init-option(ts, s);
      else
        more? := #f;
      end;
    end;
    s
  else
    // No `slot` word — return an error node; caller skips remaining tokens.
    make(<ast-error-node>, message: "expected slot word in class clause")
  end
end function;

// Look one meaningful token PAST a leading comma without consuming anything.
// Used by parse-slot-spec to decide whether a comma begins an init option
// (next token is a keyword-name) or separates two slot specs.
define function ts-peek-after-comma (ts :: <token-stream>) => (t :: <token>)
  let save = ts-pos(ts);
  // Consume the comma we are positioned on.
  if (is-punct?(ts-peek(ts), #"comma"))
    ts-advance(ts);
  end;
  let t = ts-peek(ts);
  ts-pos(ts) := save;
  t
end function;

// init-option:  KEYWORD-NAME value
//   init-keyword: foo:            → slot-init-kw, slot-required? = #f
//   required-init-keyword: foo:   → slot-init-kw, slot-required? = #t
//   init-value: <expr>            → slot-init
//   init-function: <expr>         → slot-init
// Any other keyword-name option is consumed (value parsed) but not recorded.
define function parse-slot-init-option (ts :: <token-stream>,
                                        s :: <ast-slot-spec>) => ()
  let key-tok = ts-advance(ts);   // the <keyword-name-token>
  let key = keyword-name-token-name(key-tok);
  if (key = "init-keyword")
    // Value is a keyword-name token (the init keyword, e.g. `x:`).
    if (instance?(ts-peek(ts), <keyword-name-token>))
      slot-init-kw(s) := ts-advance(ts);
    else
      slot-init-kw(s) := %extract-symbol-value(ts);
    end;
  elseif (key = "required-init-keyword")
    slot-required?(s) := #t;
    if (instance?(ts-peek(ts), <keyword-name-token>))
      slot-init-kw(s) := ts-advance(ts);
    else
      slot-init-kw(s) := %extract-symbol-value(ts);
    end;
  elseif (key = "init-value")
    slot-init(s) := parse-expression(ts);
  elseif (key = "init-function")
    slot-init(s) := parse-expression(ts);
  else
    // Unknown option (setter:, type:, …) — consume its value, don't record.
    parse-expression(ts);
  end;
end function;

// Fallback for an init-keyword whose value is NOT a bare keyword-name token
// (e.g. a `#"sym"` symbol literal).  Parse it as an expression and, if it is
// a symbol literal, keep #f for the name token (we only model bare-keyword
// forms here); otherwise just discard.  Returns #f so the caller's
// slot-init-kw stays #f when the form is non-bare.
define function %extract-symbol-value (ts :: <token-stream>) => (t :: <object>)
  parse-expression(ts);
  #f
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
    // `method name (params) => (returns) body end method name`
    let word = ts-advance(ts);
    let s = make(<ast-statement>, word: word, body: make-ast-body());
    node-token(s) := word;
    // Optional method name before the parameter list.
    if (is-name-token?(ts-peek(ts)) & ~ is-punct?(ts-peek(ts), #"lparen"))
      stmt-method-name(s) := ts-advance(ts);
    end;
    if (is-punct?(ts-peek(ts), #"lparen"))
      stmt-params(s) := parse-parameter-list(ts);
    end;
    stmt-return(s) := parse-return-spec(ts);
    stmt-body(s) := parse-body(ts);
    // Consume the end clause for this local method.
    let dummy = make-ast-body-definition(word);
    parse-definition-tail(ts, dummy);
    stmt-end-word(s) := defn-end-word(dummy);
    stmt-end-name(s) := defn-end-name(dummy);
    s
  elseif (is-name-token?(t))
    // `name (params) => (returns) body end name`  (implicit `method` word)
    let word = ts-advance(ts);
    let s = make(<ast-statement>, word: word, body: make-ast-body());
    node-token(s) := word;
    stmt-method-name(s) := word;
    if (is-punct?(ts-peek(ts), #"lparen"))
      stmt-params(s) := parse-parameter-list(ts);
    end;
    stmt-return(s) := parse-return-spec(ts);
    stmt-body(s) := parse-body(ts);
    let dummy = make-ast-body-definition(word);
    parse-definition-tail(ts, dummy);
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
    // Parenthesised fragment: a grouped expression `(e)`, a typed binding
    // `(e :: <error>)`, or a comma list `(a, b)`.  parse-paren-fragment
    // returns the inner expression transparently for the single-untyped case.
    ts-advance(ts);
    parse-paren-fragment(ts)
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

// Parenthesised fragment (the leading `(` is already consumed).
//
//   ( expr )                  → expr               (transparent grouping)
//   ( expr :: type )          → <ast-paren-list>[ <ast-typed-name> ]
//   ( e1 , e2 , … )           → <ast-paren-list>[ … ]
//
// Each item is `expression` optionally followed by `:: type`.  A typed item
// whose expression is a plain variable reference becomes an <ast-typed-name>
// (name token + type); otherwise the bare expression is kept (type dropped —
// non-name typed heads do not occur in practice).  This lets clause heads
// like `block (return)`, `exception (e :: <error>)`, and `select (n)` parse
// without forcing the whole `(…)` to be a single bare expression.
define function parse-paren-fragment (ts :: <token-stream>) => (n :: <ast-node>)
  let items = make(<stretchy-vector>);
  let any-typed? = #f;
  let done? = #f;
  until (done? | ts-at-end?(ts))
    if (is-punct?(ts-peek(ts), #"rparen"))
      done? := #t;
    else
      let expr = parse-expression(ts);
      let item = expr;
      if (is-punct?(ts-peek(ts), #"colon-colon"))
        ts-advance(ts);   // consume `::`
        let ty = parse-type-spec(ts);
        any-typed? := #t;
        if (instance?(expr, <ast-variable-ref>))
          let tn = make(<ast-typed-name>, tok: varref-tok(expr));
          typed-name-type(tn) := ty;
          item := tn;
        end;
      end;
      add!(items, item);
      if (is-punct?(ts-peek(ts), #"comma"))
        ts-advance(ts);
      else
        done? := #t;
      end;
    end;
  end;
  ts-expect-punct(ts, #"rparen", "expected ) after parenthesised expression");
  if (size(items) = 1 & ~ any-typed?)
    items[0]                                  // transparent single grouping
  else
    make(<ast-paren-list>, items: items)
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
  // Anonymous literal: optional parameter list, then optional return spec,
  // then the body.  There is no method name in expression position.
  let params = #f;
  if (is-punct?(ts-peek(ts), #"lparen"))
    params := parse-parameter-list(ts);
  end;
  let returns = parse-return-spec(ts);
  let body = parse-body(ts);
  let s = make(<ast-statement>, word: word, body: body);
  node-token(s) := word;
  stmt-params(s) := params;
  stmt-return(s) := returns;
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
  let s = make(<ast-statement>, word: word, body: make-ast-body());
  node-token(s) := word;
  // `for (clauses)` carries an iteration header with its own micro-syntax
  // (`i from 1 to 10`, `x = init then next`, `item in c`, `until test`).  It
  // is parsed structurally here, BEFORE the body, so the connector keywords
  // never reach ordinary expression parsing.  Every other begin-word's
  // parenthesised head (if any) is just an expression in the leading body.
  if (is-keyword?(word, #"for") & is-punct?(ts-peek(ts), #"lparen"))
    stmt-for-header(s) := parse-for-header(ts);
  end;
  stmt-body(s) := parse-body(ts);   // leading clause body (stops at sep / end)
  // Collect any trailing clauses: (CLAUSE-SEP body)* up to `end`.  parse-body
  // halts on each clause separator, so each iteration consumes one separator
  // and the body that follows it.  An `elseif (c)` head keeps `(c)` as the
  // first constituent of its clause body, mirroring the leading `if`.
  let clauses = make(<stretchy-vector>);
  let done? = #f;
  until (done? | ts-at-end?(ts))
    if (is-clause-separator?(ts-peek(ts)))
      let sep = ts-advance(ts);          // consume else / elseif / cleanup / …
      let cbody = parse-body(ts);
      add!(clauses, make(<ast-statement-clause>, word: sep, body: cbody));
    else
      done? := #t;
    end;
  end;
  if (size(clauses) > 0)
    stmt-clauses(s) := clauses;
  end;
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

// for-header: `( for-clause (, for-clause)* )`  — the leading `(` is at the
// current position.  Returns a vector of <ast-for-clause>.
define function parse-for-header (ts :: <token-stream>)
 => (clauses :: <stretchy-vector>)
  ts-advance(ts);   // consume `(`
  let clauses = make(<stretchy-vector>);
  let done? = #f;
  until (done? | ts-at-end?(ts))
    if (is-punct?(ts-peek(ts), #"rparen"))
      done? := #t;
    else
      let before = ts-pos(ts);
      add!(clauses, parse-for-clause(ts));
      if (is-punct?(ts-peek(ts), #"comma"))
        ts-advance(ts);   // separator between clauses
      elseif (ts-pos(ts) = before)
        // No progress (unexpected token that starts no clause) — bail so the
        // closing-paren check below doesn't spin forever.
        done? := #t;
      end;
    end;
  end;
  ts-expect-punct(ts, #"rparen", "expected ) to close for header");
  clauses
end function;

// for-clause:
//     WHILE expr | UNTIL expr                         (end-test, no variable)
//     variable (FOR-CONNECTOR expr)+                  (step / iteration / range)
//
// Captured as an optional leading variable plus a sequence of
// `connector expr` parts; the `for` macro interprets the connectors later.
define function parse-for-clause (ts :: <token-stream>) => (c :: <ast-for-clause>)
  let c = make(<ast-for-clause>, parts: make(<stretchy-vector>));
  let t = ts-peek(ts);
  if (is-keyword?(t, #"while") | is-keyword?(t, #"until"))
    // End-test clause: the keyword is the connector, no loop variable.
    let conn = ts-advance(ts);
    node-token(c) := conn;
    add!(for-clause-parts(c),
         make(<ast-for-part>, conn: conn, expr: parse-expression(ts)));
  else
    // Variable-based clause: a loop variable then one or more parts.
    if (is-name-token?(t))
      let v = ts-advance(ts);
      for-clause-var(c) := v;
      node-token(c) := v;
    end;
    let done? = #f;
    until (done? | ts-at-end?(ts))
      if (is-for-connector?(ts-peek(ts)))
        let conn = ts-advance(ts);
        add!(for-clause-parts(c),
             make(<ast-for-part>, conn: conn, expr: parse-expression(ts)));
      else
        done? := #t;
      end;
    end;
  end;
  c
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

// Parse a single specialiser / type in a parameter or value position.
// Uses parse-operand (postfix level) rather than parse-expression so a
// trailing `=` default or `,` separator is NOT swallowed into the type.
define function parse-type-spec (ts :: <token-stream>) => (n :: <ast-node>)
  parse-operand(ts)
end function;

// ── 15b. Parsing: parameter lists ────────────────────────────────────────
//
// parameter-list:
//     LPAREN parameters-OPT RPAREN
//
// parameters:
//     required-parameter , ...
//     ... #rest NAME
//     ... #key key-spec ... [#all-keys]
//     ... #all-keys
//     ... #next NAME
//
// A required parameter is a `variable` (NAME [:: type]).
// A key-spec is NAME [:: type] [= default].
//
// Modelled after nod-reader's parse_param_list_loose.

define function parse-parameter-list (ts :: <token-stream>)
 => (p :: <ast-param-list>)
  let open-tok = ts-expect-punct(ts, #"lparen", "expected ( to open parameter list");
  let p = make-ast-param-list();
  node-token(p) := open-tok;
  // mode: #"required" → #"key" once #key is seen.
  let mode = #"required";
  let done? = #f;
  until (done? | ts-at-end?(ts))
    let t = ts-peek(ts);
    if (is-punct?(t, #"rparen"))
      done? := #t;
    elseif (is-punct?(t, #"comma"))
      ts-advance(ts);   // skip stray separators
    elseif (is-keyword?(t, #"hash-rest"))
      ts-advance(ts);
      let name-tok = ts-peek(ts);
      if (is-name-token?(name-tok))
        ts-advance(ts);
        params-rest(p) := name-tok;
      else
        parse-error("expected name after #rest in parameter list");
      end;
    elseif (is-keyword?(t, #"hash-key"))
      ts-advance(ts);
      params-key?(p) := #t;
      mode := #"key";
    elseif (is-keyword?(t, #"hash-all-keys"))
      ts-advance(ts);
      params-all-keys?(p) := #t;
    elseif (is-keyword?(t, #"hash-next"))
      ts-advance(ts);
      let name-tok = ts-peek(ts);
      if (is-name-token?(name-tok))
        ts-advance(ts);
        params-next(p) := name-tok;
      else
        parse-error("expected name after #next in parameter list");
      end;
    elseif (is-name-token?(t))
      if (mode = #"key")
        // key-spec: NAME [:: type] [= default]
        let name-tok = ts-advance(ts);
        let k = make-ast-key-spec(name-tok);
        node-token(k) := name-tok;
        if (is-punct?(ts-peek(ts), #"colon-colon"))
          ts-advance(ts);
          key-spec-type(k) := parse-type-spec(ts);
        end;
        if (is-punct?(ts-peek(ts), #"equal"))
          ts-advance(ts);
          key-spec-default(k) := parse-expression(ts);
        end;
        add!(params-keys(p), k);
      else
        // required parameter: NAME [:: type]
        add!(params-required(p), parse-variable(ts));
      end;
    else
      parse-error("unexpected token in parameter list");
    end;
    // Consume a comma separator if present.
    if (is-punct?(ts-peek(ts), #"comma"))
      ts-advance(ts);
    end;
  end;
  ts-expect-punct(ts, #"rparen", "expected ) to close parameter list");
  p
end function;

// ── 15c. Parsing: return specifications ───────────────────────────────────
//
// return-spec:
//     <empty>
//     ARROW value-name                  ← single bare value (a type)
//     ARROW LPAREN value-specs-OPT RPAREN
//
// value-specs:
//     value-spec , ...
//     ... #rest NAME [:: type]
//
// value-spec:  NAME [:: type]  |  type
//
// Modelled after nod-reader's maybe_return_sig.

define function parse-return-spec (ts :: <token-stream>)
 => (r :: <ast-return-spec>)
  let r = make-ast-return-spec();
  if (~ is-punct?(ts-peek(ts), #"arrow"))
    // No `=>`: empty/absent return spec.
    r
  else
    let arrow = ts-advance(ts);   // consume `=>`
    node-token(r) := arrow;
    ret-present?(r) := #t;
    if (is-punct?(ts-peek(ts), #"lparen"))
      // ARROW ( value-specs )
      ts-advance(ts);   // consume `(`
      let done? = #f;
      until (done? | ts-at-end?(ts))
        let t = ts-peek(ts);
        if (is-punct?(t, #"rparen"))
          done? := #t;
        elseif (is-punct?(t, #"comma"))
          ts-advance(ts);
        elseif (is-keyword?(t, #"hash-rest"))
          ts-advance(ts);
          let name-tok = ts-peek(ts);
          if (is-name-token?(name-tok))
            ts-advance(ts);
            ret-rest(r) := name-tok;
            if (is-punct?(ts-peek(ts), #"colon-colon"))
              ts-advance(ts);
              ret-rest-type(r) := parse-type-spec(ts);
            end;
          else
            parse-error("expected name after #rest in return spec");
          end;
        elseif (is-name-token?(t))
          // value-spec: NAME [:: type]
          add!(ret-values(r), parse-value-spec(ts));
        else
          parse-error("unexpected token in return spec");
        end;
        if (is-punct?(ts-peek(ts), #"comma"))
          ts-advance(ts);
        end;
      end;
      ts-expect-punct(ts, #"rparen", "expected ) to close return spec");
      r
    elseif (is-name-token?(ts-peek(ts)))
      // ARROW single-value (bare type/name, no parens)
      add!(ret-values(r), parse-value-spec(ts));
      r
    else
      // `=>` with nothing parseable after it (e.g. before `;`): leave empty.
      r
    end
  end
end function;

// value-spec: NAME [:: type].  A bare type with no name is recorded as an
// <ast-typed-name> whose name token is the type's leading token and whose
// type is the parsed type — matching the existing typed-name shape.
define function parse-value-spec (ts :: <token-stream>) => (v :: <ast-typed-name>)
  let name-tok = ts-advance(ts);
  let v = make(<ast-typed-name>, tok: name-tok);
  node-token(v) := name-tok;
  if (is-punct?(ts-peek(ts), #"colon-colon"))
    ts-advance(ts);
    typed-name-type(v) := parse-type-spec(ts);
  end;
  v
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

// Dump a TYPED-NAME-like line: a label, the name, and (if a type slot
// value is given) the type subtree on the following indented lines.
define function dump-typed-name (label :: <byte-string>, name-tok :: <token>,
                                 type-node :: <object>,
                                 acc :: <stretchy-vector>, depth :: <integer>)
 => ()
  acc-indent(acc, depth);
  acc-string(acc, label);
  acc-string(acc, " ");
  acc-string(acc, token-name(name-tok));
  acc-newline(acc);
  if (instance?(type-node, <ast-node>))
    acc-indent(acc, depth + 1);
    acc-string(acc, "TYPE");
    acc-newline(acc);
    dump-node(type-node, acc, depth + 2);
  end;
end function;

// Dump a parameter list as a PARAMS block.
define function dump-param-list (p :: <ast-param-list>, acc :: <stretchy-vector>,
                                 depth :: <integer>) => ()
  acc-indent(acc, depth);
  acc-string(acc, "PARAMS");
  acc-newline(acc);
  let req = params-required(p);
  let n = size(req);
  let i = 0;
  until (i >= n)
    let v = req[i];
    dump-typed-name("PARAM", typed-name-tok(v), typed-name-type(v),
                    acc, depth + 1);
    i := i + 1;
  end;
  if (instance?(params-rest(p), <token>))
    acc-indent(acc, depth + 1);
    acc-string(acc, "REST ");
    acc-string(acc, token-name(params-rest(p)));
    acc-newline(acc);
  end;
  if (params-key?(p))
    acc-indent(acc, depth + 1);
    acc-string(acc, "KEY");
    acc-newline(acc);
    let keys = params-keys(p);
    let m = size(keys);
    let j = 0;
    until (j >= m)
      let k = keys[j];
      dump-typed-name("KEY-PARAM", key-spec-tok(k), key-spec-type(k),
                      acc, depth + 2);
      if (instance?(key-spec-default(k), <ast-node>))
        acc-indent(acc, depth + 3);
        acc-string(acc, "DEFAULT");
        acc-newline(acc);
        dump-node(key-spec-default(k), acc, depth + 4);
      end;
      j := j + 1;
    end;
  end;
  if (params-all-keys?(p))
    acc-indent(acc, depth + 1);
    acc-string(acc, "ALL-KEYS");
    acc-newline(acc);
  end;
  if (instance?(params-next(p), <token>))
    acc-indent(acc, depth + 1);
    acc-string(acc, "NEXT ");
    acc-string(acc, token-name(params-next(p)));
    acc-newline(acc);
  end;
end function;

// Dump a return spec as a RETURNS block (only when an `=>` was present).
define function dump-return-spec (r :: <ast-return-spec>, acc :: <stretchy-vector>,
                                  depth :: <integer>) => ()
  if (ret-present?(r))
    acc-indent(acc, depth);
    acc-string(acc, "RETURNS");
    acc-newline(acc);
    let vals = ret-values(r);
    let n = size(vals);
    let i = 0;
    until (i >= n)
      let v = vals[i];
      dump-typed-name("VALUE", typed-name-tok(v), typed-name-type(v),
                      acc, depth + 1);
      i := i + 1;
    end;
    if (instance?(ret-rest(r), <token>))
      dump-typed-name("REST", ret-rest(r), ret-rest-type(r), acc, depth + 1);
    end;
  end;
end function;

// Dump one slot spec as a SLOT block:
//   SLOT <name>
//     ADJ <word> ...            (one per adjective)
//     TYPE / <type subtree>     (when :: type present)
//     INIT-KEYWORD <kw>         (init-keyword: kw:)
//     REQUIRED-INIT-KEYWORD <kw>(required-init-keyword: kw:)
//     INIT / <expr subtree>     (= default, init-value:, or init-function:)
define function dump-slot-spec (s :: <ast-slot-spec>, acc :: <stretchy-vector>,
                                depth :: <integer>) => ()
  acc-indent(acc, depth);
  acc-string(acc, "SLOT");
  if (instance?(slot-name-tok(s), <token>))
    acc-string(acc, " ");
    acc-string(acc, token-name(slot-name-tok(s)));
  end;
  acc-newline(acc);
  // Adjectives.
  let adjs = slot-adjectives(s);
  let na = size(adjs);
  let ia = 0;
  until (ia >= na)
    acc-indent(acc, depth + 1);
    acc-string(acc, "ADJ ");
    acc-string(acc, token-name(adjs[ia]));
    acc-newline(acc);
    ia := ia + 1;
  end;
  // Type.
  if (instance?(slot-type(s), <ast-node>))
    acc-indent(acc, depth + 1);
    acc-string(acc, "TYPE");
    acc-newline(acc);
    dump-node(slot-type(s), acc, depth + 2);
  end;
  // Init keyword (required or not).
  if (instance?(slot-init-kw(s), <token>))
    acc-indent(acc, depth + 1);
    if (slot-required?(s))
      acc-string(acc, "REQUIRED-INIT-KEYWORD ");
    else
      acc-string(acc, "INIT-KEYWORD ");
    end;
    acc-string(acc, keyword-name-token-name(slot-init-kw(s)));
    acc-newline(acc);
  elseif (slot-required?(s))
    // required-init-keyword whose value was not a bare keyword-name.
    acc-indent(acc, depth + 1);
    acc-string(acc, "REQUIRED-INIT-KEYWORD");
    acc-newline(acc);
  end;
  // Init value / function / `= default`.
  if (instance?(slot-init(s), <ast-node>))
    acc-indent(acc, depth + 1);
    acc-string(acc, "INIT");
    acc-newline(acc);
    dump-node(slot-init(s), acc, depth + 2);
  end;
end function;

// Dump one trailing statement clause:
//   CLAUSE <sep>          (else / elseif / cleanup / exception / …)
//     <body subtree>
define function dump-statement-clause (c :: <ast-statement-clause>,
                                       acc :: <stretchy-vector>,
                                       depth :: <integer>) => ()
  acc-indent(acc, depth);
  acc-string(acc, "CLAUSE ");
  acc-string(acc, token-name(clause-word(c)));
  acc-newline(acc);
  dump-node(clause-body(c), acc, depth + 1);
end function;

// Printable spelling of a token used as an operator/connector: punctuation
// tokens (e.g. `=`) have no name-like spelling, so use their symbolic form.
define function connector-spelling (t :: <token>) => (s :: <byte-string>)
  if (instance?(t, <punctuation-token>))
    write-to-string(punctuation-token-form(t))
  else
    token-name(t)
  end
end function;

// Dump one for-clause:
//   FOR-CLAUSE [<var>]
//     PART <conn>
//       <expr subtree>
//     ...
define function dump-for-clause (c :: <ast-for-clause>, acc :: <stretchy-vector>,
                                 depth :: <integer>) => ()
  acc-indent(acc, depth);
  acc-string(acc, "FOR-CLAUSE");
  if (instance?(for-clause-var(c), <token>))
    acc-string(acc, " ");
    acc-string(acc, token-name(for-clause-var(c)));
  end;
  acc-newline(acc);
  let parts = for-clause-parts(c);
  let n = size(parts);
  let i = 0;
  until (i >= n)
    let p = parts[i];
    acc-indent(acc, depth + 1);
    acc-string(acc, "PART ");
    acc-string(acc, connector-spelling(for-part-conn(p)));
    acc-newline(acc);
    dump-node(for-part-expr(p), acc, depth + 2);
    i := i + 1;
  end;
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
    if (instance?(defn-method-name(node), <token>))
      acc-string(acc, " ");
      acc-string(acc, token-name(defn-method-name(node)));
    end;
    acc-newline(acc);
    if (instance?(defn-params(node), <ast-param-list>))
      dump-param-list(defn-params(node), acc, depth + 1);
    end;
    if (instance?(defn-return(node), <ast-return-spec>))
      dump-return-spec(defn-return(node), acc, depth + 1);
    end;
    dump-node(defn-body(node), acc, depth + 1);
  elseif (instance?(node, <ast-class-definition>))
    acc-string(acc, "DEFINE-CLASS");
    if (instance?(class-name(node), <token>))
      acc-string(acc, " ");
      acc-string(acc, token-name(class-name(node)));
    end;
    acc-newline(acc);
    // Superclasses.
    let supers = class-supers(node);
    let ns = size(supers);
    let is = 0;
    until (is >= ns)
      acc-indent(acc, depth + 1);
      acc-string(acc, "SUPER");
      acc-newline(acc);
      dump-node(supers[is], acc, depth + 2);
      is := is + 1;
    end;
    // Slot specs.
    let slots = class-slots(node);
    let nsl = size(slots);
    let isl = 0;
    until (isl >= nsl)
      dump-slot-spec(slots[isl], acc, depth + 1);
      isl := isl + 1;
    end;
  elseif (instance?(node, <ast-generic-definition>))
    acc-string(acc, "DEFINE-GENERIC");
    if (instance?(gen-name(node), <token>))
      acc-string(acc, " ");
      acc-string(acc, token-name(gen-name(node)));
    end;
    acc-newline(acc);
    // Modifiers (open / sealed / …).
    let mods = defn-modifiers(node);
    let nm = size(mods);
    let im = 0;
    until (im >= nm)
      acc-indent(acc, depth + 1);
      acc-string(acc, "MOD ");
      acc-string(acc, token-name(mods[im]));
      acc-newline(acc);
      im := im + 1;
    end;
    if (instance?(gen-params(node), <ast-param-list>))
      dump-param-list(gen-params(node), acc, depth + 1);
    end;
    if (instance?(gen-return(node), <ast-return-spec>))
      dump-return-spec(gen-return(node), acc, depth + 1);
    end;
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
  elseif (instance?(node, <ast-paren-list>))
    acc-string(acc, "PAREN-LIST");
    acc-newline(acc);
    let items = paren-list-items(node);
    let n = size(items);
    let i = 0;
    until (i >= n)
      dump-node(items[i], acc, depth + 1);
      i := i + 1;
    end;
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
    if (instance?(stmt-method-name(node), <token>))
      acc-string(acc, " ");
      acc-string(acc, token-name(stmt-method-name(node)));
    end;
    acc-newline(acc);
    // `for` iteration header (before the body).
    if (instance?(stmt-for-header(node), <stretchy-vector>))
      let fcs = stmt-for-header(node);
      let nf = size(fcs);
      let iff = 0;
      until (iff >= nf)
        dump-for-clause(fcs[iff], acc, depth + 1);
        iff := iff + 1;
      end;
    end;
    if (instance?(stmt-params(node), <ast-param-list>))
      dump-param-list(stmt-params(node), acc, depth + 1);
    end;
    if (instance?(stmt-return(node), <ast-return-spec>))
      dump-return-spec(stmt-return(node), acc, depth + 1);
    end;
    dump-node(stmt-body(node), acc, depth + 1);
    // Trailing clauses (elseif / else / cleanup / exception / …).
    if (instance?(stmt-clauses(node), <stretchy-vector>))
      let cs = stmt-clauses(node);
      let nc = size(cs);
      let ic = 0;
      until (ic >= nc)
        dump-statement-clause(cs[ic], acc, depth + 1);
        ic := ic + 1;
      end;
    end;
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
    // Leading indent already emitted at the top of dump-node.
    acc-string(acc, "TYPED-NAME ");
    acc-string(acc, token-name(typed-name-tok(node)));
    acc-newline(acc);
    if (instance?(typed-name-type(node), <ast-node>))
      acc-indent(acc, depth + 1);
      acc-string(acc, "TYPE");
      acc-newline(acc);
      dump-node(typed-name-type(node), acc, depth + 2);
    end;
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
