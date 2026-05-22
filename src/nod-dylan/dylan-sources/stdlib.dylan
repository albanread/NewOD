Module: dylan
Author: NewOpenDylan stdlib (Sprint 20b)

// ── stdlib.dylan — collection ops, FIP wrappers, for-each macro ───────────
//
// Sprint 20b: this file is auto-loaded by `nod_sema::stdlib::ensure_loaded()`
// before user code lowers. Every `define function` here is rewritten to
// `define method <name> (params... :: <object>)` by the loader so that
// user code's call-site `Dispatch` IR resolves through the process-global
// dispatch table (cross-module symbol linkage is deferred — see
// DEFERRED.md → Sprint 20b residue).
//
// What lives here:
//   * `size(c)` — collection size, via `%collection-size`.
//   * `concatenate(c1, c2)` — binary concat, via `%collection-concatenate`.
//   * `for-each` macro — sugar over the FIP primitives.
//   * `nod-stdlib-marker` — sentinel used by tests to confirm the loader
//     parsed + JIT'd the stdlib.
//
// Deferred (DEFERRED.md → "Sprint 20b residue — full collections in
// Dylan"):
//   * `reduce`, `map`, `do` as Dylan functions accepting a first-class
//     function argument. Requires the function-Word ABI (Sprint 21
//     sub-goal) to thread function references through the JIT call shape.
//     The Rust-side `collection_reduce`/`collection_map`/`collection_do`
//     stay in `nod_runtime::collections` until then; the FIP primitives
//     wired in this sprint make the migration mechanical when first-
//     class functions land.
//   * Slot-accessor-based FIP (`define sealed method
//     forward-iteration-protocol …` per concrete class). Requires the
//     `%`-prefix lexer carve-out for `<iteration-state>`'s slot names
//     and slot-accessor generation for pre-registered Rust classes. Both
//     are Sprint 21 follow-ups; the Rust-side `forward_iteration_protocol`
//     + the `%fip-*` primitives here cover the protocol surface in the
//     meantime.

// ─── nod-stdlib-marker — loader-sanity sentinel ───────────────────────────
//
// Echoes its argument back unchanged + 1. Used by tests to confirm the
// loader registered stdlib methods into the process-global dispatch
// table. The single argument is required so the loader rewrites this
// as `define method nod-stdlib-marker (x :: <object>)` — 0-arg generics
// aren't allowed in Dylan.

define function nod-stdlib-marker (x) => (n)
  x + 1
end function;

// ─── size ──────────────────────────────────────────────────────────────────
//
// A thin wrapper around the `%collection-size` primitive. The primitive
// dispatches on the concrete class internally (it's the existing Rust
// `collection_size` exposed via the `nod_collection_size` extern). The
// loader rewrites this to a method on `(c :: <object>)`, registered as
// the sole entry under the `size` generic; user code's call to `size(c)`
// resolves through the process-global dispatch table.

define function size (c) => (n)
  %collection-size(c)
end function;

// ─── concatenate ───────────────────────────────────────────────────────────
//
// Binary concatenate. Delegates to the `%collection-concatenate`
// primitive (Rust `collection_concatenate`). Preserves shape when both
// inputs share a class; widens to `<simple-object-vector>` otherwise.

define function concatenate (c1, c2) => (result)
  %collection-concatenate(c1, c2)
end function;

// ─── reduce ────────────────────────────────────────────────────────────────
//
// Sprint 21: now Dylan-defined. `fn` is a `<function>` first-class
// value; the inner combiner call lowers to `nod_funcall2(fn, acc, x)`
// because `fn` is an env-bound name that isn't a top-level function or
// generic. FIP-driven so this body is identical for every concrete
// collection class registered with `forward-iteration-protocol`.

define function reduce (fn, init, c) => (result)
  let state = %fip-init(c);
  let acc = init;
  until (%fip-finished?(state))
    acc := %funcall2(fn, acc, %fip-current-element(state));
    %fip-advance!(state)
  end;
  acc
end function;

// ─── map ───────────────────────────────────────────────────────────────────
//
// Sprint 21: returns a fresh `<simple-object-vector>` of length
// `size(c)`. Shape-preserving variants (return a `<list>` when input
// is a `<list>`, etc.) land alongside the rest of the stdlib
// collection methods in Sprint 22+.

define function map (fn, c) => (result)
  let n = %collection-size(c);
  let result = %make-sov(n);
  let state = %fip-init(c);
  let i = 0;
  until (%fip-finished?(state))
    %vector-element-setter(%funcall1(fn, %fip-current-element(state)), result, i);
    i := i + 1;
    %fip-advance!(state)
  end;
  result
end function;

// ─── do ────────────────────────────────────────────────────────────────────
//
// Sprint 21: invoke `fn` on each element of `c` for side effects.
// Returns `#f`.

define function do (fn, c) => (result)
  let state = %fip-init(c);
  until (%fip-finished?(state))
    %funcall1(fn, %fip-current-element(state));
    %fip-advance!(state)
  end;
  #f
end function;

// ─── <table> generics (Sprint 22) ──────────────────────────────────────────
//
// `<table>` is a `<explicit-key-collection>` registered as a seed class
// by `nod_runtime::tables`. The runtime owns the heap layout + the
// open-addressing hash machinery + the `object-hash` /
// `object-equal?` fast path; this file owns the user-visible generic
// surface.
//
// The methods below specialise on `<table>` so they outrank the
// `<object>` rewrites of `size` / `concatenate` / etc. for tables.

define method size (t :: <table>) => (n :: <integer>)
  %table-size(t)
end method;

define method element (t :: <table>, key) => (value)
  %table-element-or-default(t, key, #f)
end method;

define method element-setter (value, t :: <table>, key) => (value)
  %table-element-setter(value, t, key)
end method;

define method remove-key! (t :: <table>, key) => (t)
  %table-remove-key(t, key)
end method;

define method keys (t :: <table>) => (ks)
  %table-keys(t)
end method;

define method values (t :: <table>) => (vs)
  %table-values(t)
end method;

// `object-hash` and `object-equal?` are exposed as Dylan-side generics
// so user code can call them and (eventually) add methods for new key
// types. The Rust fast path still drives table probes — these methods
// just surface the primitive to user code.

define method object-hash (x) => (h :: <integer>)
  %object-hash(x)
end method;

define method object-equal? (a, b) => (eq :: <boolean>)
  %object-equal?(a, b)
end method;

// ─── for-each macro ────────────────────────────────────────────────────────
//
// Sugar over the FIP primitives. Expands to a `let state = %fip-init(c);
// until (%fip-finished?(state)) ... %fip-advance!(state) end` loop. The
// `?var:name` binding is rebound on each iteration to the current
// element.
//
// Sprint 25: the body-shaped surface `for-each (x in c) body end` is
// recognised by the parser now (see `Expr::MacroCall` + the
// `known_macros` plumbing in `nod-reader/src/parser.rs`). Sprint 20b
// shipped the macro definition but couldn't call it from a separate
// file because the parser didn't know body-shaped macro syntax.

define macro for-each
  { for-each (?var:name in ?coll:expression) ?body:body end }
    => { begin
           let %fip-state = %fip-init(?coll);
           until (%fip-finished?(%fip-state))
             let ?var = %fip-current-element(%fip-state);
             ?body;
             %fip-advance!(%fip-state)
           end
         end }
end macro;

// ─── unless macro ──────────────────────────────────────────────────────────
//
// Sprint 25: retired the hardcoded `Expr::Unless` AST variant. The
// parser now treats `unless (cond) body end` as a body-shaped macro
// call (because `unless` is in the parser's known-macro set, seeded
// from this stdlib), and the rule below expands it to `if (~ cond)
// body end`. Identical compile-time output to the old hardcoded
// lowering — `if` remains the kernel primitive.

define macro unless
  { unless ?cond:expression ?body:body end }
    => { if (~ ?cond) ?body else #f end }
end macro;

// ─── Sprint 32: closure → C callback pointer ──────────────────────────────
//
// `as-wndproc-callback(cb)` and `as-wndenumproc-callback(cb)` register
// a Dylan closure as a Win32-callable function pointer for the named
// signature, returning a `<c-pointer>` value (fixnum-tagged raw
// address — the FFI ABI Sprint 28 adopted for `<c-pointer>` values).
//
// Sprint 32 ships two signatures: `WNDPROC` (window procedure, used by
// `RegisterClass(W)`) and `WNDENUMPROC` (passed to `EnumWindows`).
// Later sprints add TIMERPROC, THREADPROC, DLGPROC, hook procs, etc.
// A unified `as-c-callback(cb, signature-symbol)` form is deferred
// until `select` lowers.
//
// Registrations are leak-by-design in Sprint 32 — the pool of 32
// slots per signature is allocated once and never freed. A later
// sprint adds release semantics.

define function as-wndproc-callback (closure) => (ptr)
  %register-wndproc(closure)
end function;

define function as-wndenumproc-callback (closure) => (ptr)
  %register-wndenumproc(closure)
end function;
