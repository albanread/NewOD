Module: kernel-arith

// Sprint 55a — short-circuit `|` / `&` coverage for the lowering gate.

define function sc-or (a :: <integer>, b :: <integer>) => (r :: <integer>)
  a | b
end function;

define function sc-and (a :: <integer>, b :: <integer>) => (r :: <integer>)
  a & b
end function;

define function sc-in-if (a :: <integer>, b :: <integer>, c :: <integer>) => (r :: <integer>)
  if (a | b) c + 1 else c - 1 end
end function;
