Module: point

define class <point> (<object>)
  slot x :: <integer>, init-keyword: x:;
  slot y :: <integer>, init-keyword: y:;
end class;

define function distance-squared (p :: <point>) => (<integer>)
  let xx = x(p);
  let yy = y(p);
  xx * xx + yy * yy
end function distance-squared;

define function main () => (<integer>)
  distance-squared(make(<point>, x: 3, y: 4))
end function main;
