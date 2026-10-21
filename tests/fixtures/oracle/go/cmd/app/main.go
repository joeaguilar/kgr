package main

import (
	"example.com/demo/internal/store"
	"github.com/pkg/errors"
)

func main() {
	s := store.New()
	_ = s.Get("k")
	_ = errors.New("x")
}
