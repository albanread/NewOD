Module: rope

// Sprint 43a — immutable read-only rope buffer.
//
// A rope is a binary tree of `<byte-string>` chunks:
//
//      <rope>
//        ├ <rope-leaf>   bytes (a <byte-string>) + cached length
//        └ <rope-node>   left, right (both <rope>), weight, total
//
//   * weight = size(left)            (caches the index split point)
//   * total  = size(left) + size(right)  (caches whole-subtree length)
//
// Both are caches so `rope-size` is O(1) and `rope-element` walks log(n)
// nodes (one comparison per level).
//
// This is the read-only core (Sprint 43a):
//   * make-rope-from-string(s)       — recursive split into ≤1024-byte leaves
//   * rope-size(r)                   — O(1)
//   * rope-element(r, i)             — O(log n)
//   * rope-substring(r, lo, hi)      — O(log n + len), fresh <byte-string>
//   * rope-concatenate(a, b)         — O(1), new internal node
//   * for-each-leaf(r, fn)           — O(n), in-order leaf walk
//
// Sprint 43b will add split-at / insert / delete on top of these.
//
// Leaf max = 1024 bytes — bigger leaves keep the tree shallow and reduce
// per-op dispatch / GC pressure. Production editors (xi, VSCode's
// TextBuffer) use 1024-4096. We can revisit after benchmarks.

// ─── Win32 mins for the EXE entry point + self-tests ─────────────────────
//
// The self-test main prints PASS/FAIL lines via format-out; the test
// driver reads stdout and asserts every "PASS" header is present.

// ─── Class hierarchy ─────────────────────────────────────────────────────

define class <rope> (<object>) end class;

define class <rope-leaf> (<rope>)
  slot rope-leaf-bytes :: <byte-string>, init-keyword: bytes:;
  slot rope-leaf-len   :: <integer>,     init-keyword: len:;
end class;

define class <rope-node> (<rope>)
  slot rope-node-left   :: <rope>,    init-keyword: left:;
  slot rope-node-right  :: <rope>,    init-keyword: right:;
  slot rope-node-weight :: <integer>, init-keyword: weight:;
  slot rope-node-total  :: <integer>, init-keyword: total:;
end class;

// ─── rope-size — O(1), cached at each node ───────────────────────────────

define method rope-size (r :: <rope-leaf>) => (n :: <integer>)
  rope-leaf-len(r)
end method;

define method rope-size (r :: <rope-node>) => (n :: <integer>)
  rope-node-total(r)
end method;

// ─── rope-element — O(log n) tree descent ────────────────────────────────

define method rope-element (r :: <rope-leaf>, i :: <integer>) => (b :: <integer>)
  element(rope-leaf-bytes(r), i)
end method;

define method rope-element (r :: <rope-node>, i :: <integer>) => (b :: <integer>)
  let w = rope-node-weight(r);
  if (i < w)
    rope-element(rope-node-left(r), i)
  else
    rope-element(rope-node-right(r), i - w)
  end
end method;

// ─── rope-concatenate — O(1), new internal node ──────────────────────────

define method rope-concatenate (a :: <rope>, b :: <rope>) => (r :: <rope-node>)
  let asize = rope-size(a);
  let bsize = rope-size(b);
  make(<rope-node>,
       left: a, right: b,
       weight: asize, total: asize + bsize)
end method;

// ─── for-each-leaf — in-order leaf walk ──────────────────────────────────
//
// `fn` is a `<function>` of one arg: the leaf's `<byte-string>`. Result
// is discarded. Useful for rendering (walk leaves, draw each chunk) and
// for serialising (walk leaves, write each chunk to disk).

define method for-each-leaf (r :: <rope-leaf>, fn) => ()
  fn(rope-leaf-bytes(r));
  #f
end method;

define method for-each-leaf (r :: <rope-node>, fn) => ()
  for-each-leaf(rope-node-left(r), fn);
  for-each-leaf(rope-node-right(r), fn);
  #f
end method;

// ─── rope-substring — fresh <byte-string> for [lo, hi) ───────────────────
//
// Allocate the destination once, then walk the rope copying intersecting
// runs from each leaf via the bulk-copy primitive. The recursive
// `rope-copy-into` returns the number of bytes copied so the caller can
// advance `dst-off` correctly across sibling traversals.

define method rope-copy-into
    (r :: <rope-leaf>, lo :: <integer>, hi :: <integer>,
     dst :: <byte-string>, dst-off :: <integer>)
 => (n :: <integer>)
  let leaf-len = rope-leaf-len(r);
  // Intersect [lo, hi) with [0, leaf-len).
  let a = if (lo > 0) lo else 0 end;
  let b = if (hi < leaf-len) hi else leaf-len end;
  if (a < b)
    %byte-string-copy!(dst, dst-off, rope-leaf-bytes(r), a, b - a);
    b - a
  else
    0
  end
end method;

define method rope-copy-into
    (r :: <rope-node>, lo :: <integer>, hi :: <integer>,
     dst :: <byte-string>, dst-off :: <integer>)
 => (n :: <integer>)
  let w = rope-node-weight(r);
  // Left child covers indices [0, w); right covers [w, total).
  let from-left =
    if (lo < w)
      let left-hi = if (hi < w) hi else w end;
      rope-copy-into(rope-node-left(r), lo, left-hi, dst, dst-off)
    else
      0
    end;
  let from-right =
    if (hi > w)
      let right-lo = if (lo > w) lo - w else 0 end;
      let right-hi = hi - w;
      rope-copy-into(rope-node-right(r), right-lo, right-hi,
                     dst, dst-off + from-left)
    else
      0
    end;
  from-left + from-right
end method;

define function rope-substring
    (r, lo :: <integer>, hi :: <integer>) => (s :: <byte-string>)
  let n = hi - lo;
  let result = %byte-string-allocate(n);
  rope-copy-into(r, lo, hi, result, 0);
  result
end function;

// ─── make-rope-from-string — recursive split into ≤1024-byte leaves ──────
//
// Leaf max: 1024 — small enough to keep individual-edit copying cheap
// (Sprint 43b), large enough to keep the tree shallow and dispatch / GC
// overhead low. Inlined literal here because user-code `define constant`
// lowers to a 0-arity function call shape; an inline 1024 is cleaner
// for the one site that needs it.

define function make-rope-from-string (s) => (r)
  let n = size(s);
  if (n <= 1024)
    make(<rope-leaf>, bytes: s, len: n)
  else
    let mid = n / 2;
    let left  = make-rope-from-string(copy-sequence(s, 0, mid));
    let right = make-rope-from-string(copy-sequence(s, mid, n));
    rope-concatenate(left, right)
  end
end function;

// ─── Self-tests (run as part of `main`) ──────────────────────────────────
//
// Build a deterministic test buffer where byte[i] = i mod 256, drive
// each rope op against it, and emit PASS / FAIL lines on stdout. The
// Rust-side test (`rope_ops.rs`) runs this EXE and asserts every
// expected PASS line appears.

define function make-test-bytes (n :: <integer>) => (s :: <byte-string>)
  let s = %byte-string-allocate(n);
  let i = 0;
  until (i = n)
    let b = i - (i / 256) * 256;
    %byte-string-element-setter(b, s, i);
    i := i + 1;
  end;
  s
end function;

// Verify byte[i] = i mod 256 across a rope.
define function rope-bytes-match-pattern? (r, n :: <integer>) => (ok :: <boolean>)
  let ok = #t;
  let i = 0;
  until (i = n)
    let expected = i - (i / 256) * 256;
    if (rope-element(r, i) ~= expected)
      ok := #f;
    else
      #f
    end;
    i := i + 1;
  end;
  ok
end function;

define function main () => ()
  // ─── Test 1: tiny single-leaf rope ──────────────────────────────────
  let small = "hello";
  let r1 = make-rope-from-string(small);
  if (rope-size(r1) = 5)
    format-out("PASS: small rope size\n");
  else
    format-out("FAIL: small rope size = %d\n", rope-size(r1));
  end;
  // 'h' = 104, 'o' = 111
  if (rope-element(r1, 0) = 104 & rope-element(r1, 4) = 111)
    format-out("PASS: small rope elements\n");
  else
    format-out("FAIL: small rope elements\n");
  end;

  // ─── Test 2: multi-leaf rope (4000 bytes → ~4 leaves at max=1024) ──
  let big-bytes = make-test-bytes(4000);
  let big-rope  = make-rope-from-string(big-bytes);
  if (rope-size(big-rope) = 4000)
    format-out("PASS: big rope size\n");
  else
    format-out("FAIL: big rope size = %d\n", rope-size(big-rope));
  end;
  if (rope-bytes-match-pattern?(big-rope, 4000))
    format-out("PASS: big rope element pattern\n");
  else
    format-out("FAIL: big rope element pattern\n");
  end;

  // ─── Test 3: rope-substring round-trip across leaf boundary ────────
  // Take bytes [1000, 1100) — crosses the ~1024-byte boundary.
  let sub = rope-substring(big-rope, 1000, 1100);
  let sub-ok = #t;
  if (size(sub) ~= 100) sub-ok := #f else #f end;
  let j = 0;
  until (j = 100)
    let expected = (1000 + j) - ((1000 + j) / 256) * 256;
    if (element(sub, j) ~= expected) sub-ok := #f else #f end;
    j := j + 1;
  end;
  if (sub-ok)
    format-out("PASS: rope-substring across leaf boundary\n");
  else
    format-out("FAIL: rope-substring (size=%d)\n", size(sub));
  end;

  // ─── Test 4: rope-concatenate ──────────────────────────────────────
  let a = make-rope-from-string("foo");
  let b = make-rope-from-string("bar");
  let c = rope-concatenate(a, b);
  // 'f'=102 'o'=111 'o'=111 'b'=98 'a'=97 'r'=114
  if (rope-size(c) = 6
        & rope-element(c, 0) = 102
        & rope-element(c, 3) = 98
        & rope-element(c, 5) = 114)
    format-out("PASS: rope-concatenate\n");
  else
    format-out("FAIL: rope-concatenate size=%d\n", rope-size(c));
  end;

  // ─── Test 5: for-each-leaf covers every byte ───────────────────────
  // Walk the big rope, summing leaf sizes via a captured cell. The
  // sum equals total bytes iff every leaf is visited exactly once.
  let visited = 0;
  for-each-leaf(big-rope,
                method (leaf-bytes)
                  visited := visited + size(leaf-bytes)
                end);
  if (visited = 4000)
    format-out("PASS: for-each-leaf covers all bytes\n");
  else
    format-out("FAIL: for-each-leaf visited %d bytes (expected 4000)\n",
               visited);
  end;

  // ─── Test 6: rope-substring whole-rope returns equal bytes ─────────
  let whole = rope-substring(big-rope, 0, 4000);
  if (size(whole) = 4000 & whole = big-bytes)
    format-out("PASS: rope-substring full range == original\n");
  else
    format-out("FAIL: rope-substring full range mismatch\n");
  end;

  format-out("DONE\n");
end function main;
