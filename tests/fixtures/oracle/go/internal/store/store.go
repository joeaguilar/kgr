package store

import (
	u "example.com/demo/pkg/util"
	"fmt"
)

type Store struct {
	name string
}

func (s *Store) Get(k string) string {
	return fmt.Sprint(u.Clean(k))
}

func New() *Store {
	return &Store{}
}
