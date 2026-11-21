package p

type Number interface {
	~int | ~string
	String() string
}

type G[T any] struct {
	v T
}

type Box struct {
	G[int]
	*H[string]
	pkg.K[bool]
	n int
}
