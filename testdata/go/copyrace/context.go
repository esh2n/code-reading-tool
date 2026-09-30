// Package copyrace is a small reproduction of the data race gin fixed in
// PR #1841: Copy duplicates the struct but shares the Params backing array.
package copyrace

import "sync"

type Param struct {
	Key   string
	Value string
}

type Context struct {
	Params []Param
	Keys   map[string]any
}

var pool = sync.Pool{New: func() any { return &Context{Params: make([]Param, 0, 4)} }}

// Acquire takes a context from the pool, the way a router does per request.
func Acquire(params ...Param) *Context {
	c := pool.Get().(*Context)
	c.Params = append(c.Params, params...)
	return c
}

// Release resets a context and returns it to the pool.
func Release(c *Context) {
	c.reset()
	pool.Put(c)
}

func (c *Context) reset() {
	for i := range c.Params {
		c.Params[i] = Param{}
	}
	c.Params = c.Params[:0]
	c.Keys = nil
}

// Copy returns a copy that is meant to be safe to use in a goroutine.
func (c *Context) Copy() *Context {
	cp := *c
	cp.Keys = map[string]any{}
	for k, v := range c.Keys {
		cp.Keys[k] = v
	}
	return &cp
}

// Param returns the value of the named parameter, or "" if absent.
func (c *Context) Param(key string) string {
	for _, p := range c.Params {
		if p.Key == key {
			return p.Value
		}
	}
	return ""
}
