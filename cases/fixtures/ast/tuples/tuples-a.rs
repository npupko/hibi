type A = (u8,);
fn p() { let (a,) = t; }
fn c() { f(a,); }
type D = (u8, u16,);
struct S(u8,);
fn e() { let v = (1,); }
