package p

type Point struct {
	X, Y  int
	Label string `json:"label"`
	*Base
	io.Reader
}

type Shape interface {
	Area() float64
	fmt.Stringer
}

type (
	A struct{ v int }
	B = A
)

func (p * Point) Move() {}

func (g G2 [T]) Gap() {}
