Module: gc-rope-file-load

// GC rope file-load test.
//
// Loads real files from f:\scratch into rope buffers and exercises
// every rope operation: make-rope-from-string, rope-size, rope-element,
// rope-line-count, rope-line-to-offset, rope-offset-to-line,
// rope-concatenate, rope-split-at, rope-insert, rope-delete,
// for-each-leaf, rope->string, rope-substring.
//
// Expected file: f:\scratch\sample-tall-wide.txt
//   86296 bytes, 2220 LF bytes → rope-line-count = 2221, first byte = '/' (47)
//
// main() returns rope-line-count(r) = 2221 if every assertion passes,
// 0 if any assertion fails.  The Rust test asserts the return value is 2221.

// ─── Rope class hierarchy ─────────────────────────────────────────────────

define class <rope> (<object>) end class;

define class <rope-leaf> (<rope>)
  slot rope-leaf-bytes    :: <byte-string>, init-keyword: bytes:;
  slot rope-leaf-len      :: <integer>,     init-keyword: len:;
  slot rope-leaf-newlines :: <integer>,     init-keyword: newlines:;
end class;

define class <rope-node> (<rope>)
  slot rope-node-left     :: <rope>,    init-keyword: left:;
  slot rope-node-right    :: <rope>,    init-keyword: right:;
  slot rope-node-weight   :: <integer>, init-keyword: weight:;
  slot rope-node-total    :: <integer>, init-keyword: total:;
  slot rope-node-newlines :: <integer>, init-keyword: newlines:;
end class;

// ─── Newline-count helper ──────────────────────────────────────────────────

define function count-newlines-in (s) => (n :: <integer>)
  let len = size(s);
  let count = 0;
  let i = 0;
  until (i = len)
    if (element(s, i) = 10)
      count := count + 1;
    else
      #f
    end;
    i := i + 1;
  end;
  count
end function;

// ─── rope-size — O(1) ─────────────────────────────────────────────────────

define method rope-size (r :: <rope-leaf>) => (n :: <integer>)
  rope-leaf-len(r)
end method;

define method rope-size (r :: <rope-node>) => (n :: <integer>)
  rope-node-total(r)
end method;

define method rope-newlines (r :: <rope-leaf>) => (n :: <integer>)
  rope-leaf-newlines(r)
end method;

define method rope-newlines (r :: <rope-node>) => (n :: <integer>)
  rope-node-newlines(r)
end method;

// ─── rope-element — O(log n) ──────────────────────────────────────────────

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

// ─── rope-concatenate — O(1) ──────────────────────────────────────────────

define method rope-concatenate (a :: <rope>, b :: <rope>) => (r :: <rope>)
  let asize = rope-size(a);
  let bsize = rope-size(b);
  if (asize = 0)
    b
  elseif (bsize = 0)
    a
  else
    make(<rope-node>,
         left: a, right: b,
         weight: asize, total: asize + bsize,
         newlines: rope-newlines(a) + rope-newlines(b))
  end
end method;

define function empty-rope () => (r :: <rope-leaf>)
  make(<rope-leaf>, bytes: "", len: 0, newlines: 0)
end function;

// ─── for-each-leaf — in-order walk ────────────────────────────────────────

define method for-each-leaf (r :: <rope-leaf>, fn) => ()
  fn(rope-leaf-bytes(r));
  #f
end method;

define method for-each-leaf (r :: <rope-node>, fn) => ()
  for-each-leaf(rope-node-left(r), fn);
  for-each-leaf(rope-node-right(r), fn);
  #f
end method;

// ─── rope-copy-into ───────────────────────────────────────────────────────

define method rope-copy-into
    (r :: <rope-leaf>, lo :: <integer>, hi :: <integer>,
     dst :: <byte-string>, dst-off :: <integer>)
 => (n :: <integer>)
  let leaf-len = rope-leaf-len(r);
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

// ─── make-rope-from-string ────────────────────────────────────────────────

define function make-rope-from-string (s) => (r)
  let n = size(s);
  if (n <= 1024)
    make(<rope-leaf>,
         bytes: s, len: n,
         newlines: count-newlines-in(s))
  else
    let mid = n / 2;
    let left  = make-rope-from-string(copy-sequence(s, 0, mid));
    let right = make-rope-from-string(copy-sequence(s, mid, n));
    rope-concatenate(left, right)
  end
end function;

// ─── rope-split-at ────────────────────────────────────────────────────────

define method rope-split-at (r :: <rope-leaf>, i :: <integer>) => (split)
  let n = rope-leaf-len(r);
  if (i <= 0)
    pair(empty-rope(), r)
  elseif (i >= n)
    pair(r, empty-rope())
  else
    let bytes = rope-leaf-bytes(r);
    let left-bytes  = copy-sequence(bytes, 0, i);
    let right-bytes = copy-sequence(bytes, i, n);
    pair(make(<rope-leaf>,
              bytes: left-bytes,  len: i,
              newlines: count-newlines-in(left-bytes)),
         make(<rope-leaf>,
              bytes: right-bytes, len: n - i,
              newlines: count-newlines-in(right-bytes)))
  end
end method;

define method rope-split-at (r :: <rope-node>, i :: <integer>) => (split)
  let total = rope-node-total(r);
  if (i <= 0)
    pair(empty-rope(), r)
  elseif (i >= total)
    pair(r, empty-rope())
  else
    let w = rope-node-weight(r);
    if (i = w)
      pair(rope-node-left(r), rope-node-right(r))
    elseif (i < w)
      let inner = rope-split-at(rope-node-left(r), i);
      pair(head(inner),
           rope-concatenate(tail(inner), rope-node-right(r)))
    else
      let inner = rope-split-at(rope-node-right(r), i - w);
      pair(rope-concatenate(rope-node-left(r), head(inner)),
           tail(inner))
    end
  end
end method;

define function rope-insert (r, i :: <integer>, s) => (out)
  let split = rope-split-at(r, i);
  let middle = make-rope-from-string(s);
  rope-concatenate(rope-concatenate(head(split), middle), tail(split))
end function;

define function rope-delete (r, lo :: <integer>, hi :: <integer>) => (out)
  let first-split  = rope-split-at(r, lo);
  let second-split = rope-split-at(tail(first-split), hi - lo);
  rope-concatenate(head(first-split), tail(second-split))
end function;

define function rope->string (r) => (s)
  rope-substring(r, 0, rope-size(r))
end function;

// ─── Line-indexing ────────────────────────────────────────────────────────

define method rope-line-count (r :: <rope>) => (n :: <integer>)
  rope-newlines(r) + 1
end method;

define method rope-line-to-offset
    (r :: <rope-leaf>, ln :: <integer>) => (off :: <integer>)
  if (ln <= 0)
    0
  else
    let bytes = rope-leaf-bytes(r);
    let n     = rope-leaf-len(r);
    let seen  = 0;
    let pos   = 0;
    let found = -1;
    until (pos = n | found >= 0)
      if (element(bytes, pos) = 10)
        seen := seen + 1;
        if (seen = ln)
          found := pos + 1;
        else
          #f
        end;
      else
        #f
      end;
      pos := pos + 1;
    end;
    if (found < 0) n else found end
  end
end method;

define method rope-line-to-offset
    (r :: <rope-node>, ln :: <integer>) => (off :: <integer>)
  if (ln <= 0)
    0
  else
    let left-newlines = rope-newlines(rope-node-left(r));
    if (ln <= left-newlines)
      rope-line-to-offset(rope-node-left(r), ln)
    else
      rope-node-weight(r)
        + rope-line-to-offset(rope-node-right(r), ln - left-newlines)
    end
  end
end method;

define method rope-offset-to-line
    (r :: <rope-leaf>, off :: <integer>) => (ln :: <integer>)
  let bytes = rope-leaf-bytes(r);
  let n     = rope-leaf-len(r);
  let limit = if (off < 0) 0 elseif (off > n) n else off end;
  let count = 0;
  let i = 0;
  until (i = limit)
    if (element(bytes, i) = 10)
      count := count + 1;
    else
      #f
    end;
    i := i + 1;
  end;
  count
end method;

define method rope-offset-to-line
    (r :: <rope-node>, off :: <integer>) => (ln :: <integer>)
  let w = rope-node-weight(r);
  if (off <= w)
    rope-offset-to-line(rope-node-left(r), off)
  else
    rope-newlines(rope-node-left(r))
      + rope-offset-to-line(rope-node-right(r), off - w)
  end
end method;

// ─── Single pass: load file, run all 12 assertions ───────────────────────
//
// Returns 2221 (rope-line-count) if every assertion passes, 0 on any failure.

define function run-one-pass () => (<integer>)
  let content = %read-file("f:\\scratch\\sample-tall-wide.txt");
  let r = make-rope-from-string(content);

  // T1: size matches the known file size (86296 bytes, LF-only)
  if (~(rope-size(r) = 86296))
    0

  // T2: first byte is '/' (47) — the file starts with "//"
  elseif (~(rope-element(r, 0) = 47))
    0

  // T3: line count = 2220 LF bytes + 1 = 2221
  elseif (~(rope-line-count(r) = 2221))
    0

  // T4: offset of line 0 is always 0
  elseif (~(rope-line-to-offset(r, 0) = 0))
    0

  // T5: byte 0 is on line 0
  elseif (~(rope-offset-to-line(r, 0) = 0))
    0

  // T6: for-each-leaf sums to full file size
  elseif (begin
            let visited = 0;
            for-each-leaf(r, method (leaf-bytes)
                               visited := visited + size(leaf-bytes)
                             end);
            ~(visited = 86296)
          end)
    0

  // T7: rope-substring across leaf boundaries — 512 bytes at offset 512
  elseif (~(size(rope-substring(r, 512, 1024)) = 512))
    0

  // T8: concatenate r with itself → double size and double line count
  elseif (begin
            let r2 = rope-concatenate(r, r);
            ~(rope-size(r2) = 172592) | ~(rope-line-count(r2) = 4441)
          end)
    0

  // T9: split-at midpoint — halves sum back to original
  elseif (begin
            let sp = rope-split-at(r, 43148);
            ~(rope-size(head(sp)) + rope-size(tail(sp)) = 86296)
          end)
    0

  // T10: insert + delete round-trip preserves size
  elseif (begin
            let r3 = rope-insert(r, 1000, "ROUNDTRIP");
            let r4 = rope-delete(r3, 1000, 1009);
            ~(rope-size(r4) = 86296)
          end)
    0

  // T11: line-to-offset / offset-to-line round-trip on line 100
  elseif (begin
            let off100 = rope-line-to-offset(r, 100);
            ~(rope-offset-to-line(r, off100) = 100)
          end)
    0

  // T12: rope->string on a small insert produces correct length
  elseif (begin
            let snippet = rope-insert(make-rope-from-string("abcde"), 2, "XY");
            ~(size(rope->string(snippet)) = 7)
          end)
    0

  else
    rope-line-count(r)
  end
end function run-one-pass;

// ─── Main: repeat run-one-pass 50 times to exercise GC pressure ──────────
//
// Returns 2221 if all 50 passes succeed; returns 0 on any failure.

define function main () => (<integer>)
  let i  = 0;
  let ok = #t;
  while (i < 150 & ok)
    if (run-one-pass() = 0)
      ok := #f
    else
      #f
    end;
    i := i + 1;
  end;
  if (ok) 2221 else 0 end
end function main;
