Module: point

// Sprint 34: renamed from <point> to <user-point> because `<point>` is
// now a seed `<c-struct>` registered at process boot (see
// `nod-runtime/src/structs.rs`). This fixture exercises user-class
// `define class` lowering — no relation to the Sprint 34 struct.
define class <user-point> (<object>)
  slot x :: <integer>, init-keyword: x:;
  slot y :: <integer>, init-keyword: y:;
end class;

define function distance-squared (p :: <user-point>) => (<integer>)
  let xx = x(p);
  let yy = y(p);
  xx * xx + yy * yy
end function distance-squared;

define function main () => (<integer>)
  distance-squared(make(<user-point>, x: 3, y: 4))
end function main;
