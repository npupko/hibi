// Package main is a fixture; this comment is not a unit.
package main

import (
	"fmt"
	"strings"
)

const Greeting = "héllo"

const (
	A = 1
	B = 2
)

var x, y = 1, 2

var (
	p = "p"
	q = "q"
)

var (
	r = "r"
)

type C int

type (
	D int
	E = string
)

type Cache[K comparable] struct {
	items map[K]string
}

func (c *Cache[K]) Get(k K) string {
	return c.items[k]
}

func (c Cache[K]) Len() int {
	return len(c.items)
}

func init() {}

func init() {}

func main() {
	fmt.Println(strings.ToUpper(Greeting), x, y, p, q, r, A, B)
}
