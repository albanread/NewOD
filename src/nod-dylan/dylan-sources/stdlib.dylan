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

// ─── for-each macro ────────────────────────────────────────────────────────
//
// Sugar over the FIP primitives. Expands to a `let state = %fip-init(c);
// until (%fip-finished?(state)) ... %fip-advance!(state) end` loop. The
// `?var:name` binding is rebound on each iteration to the current
// element.
//
// Sprint 20b deviation: the standard Dylan `for ... in ...` clause is a
// `for` macro with many clause types; `for-each` is a single-clause
// subset chosen here because Sprint 20b's macro engine handles
// single-pattern macros cleanly. The full `for` macro is Sprint 21
// work — see DEFERRED.md.

define macro for-each
  { for-each (?var:name in ?coll:expression) ?body:expression end }
    => { let %fip-state = %fip-init(?coll);
         until (%fip-finished?(%fip-state))
           let ?var = %fip-current-element(%fip-state);
           ?body;
           %fip-advance!(%fip-state)
         end }
end macro;
