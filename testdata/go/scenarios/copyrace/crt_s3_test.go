package copyrace

import (
	"sync"
	"testing"
)

// S3 concurrent: the copy goes to a goroutine while the original is
// released and reused by the next request.
func TestCrtS3(t *testing.T) {
	c := Acquire(Param{Key: "id", Value: "42"})
	cp := c.Copy()
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		t.Logf("cp.Param(\"id\") = %q", cp.Param("id"))
	}()
	Release(c)
	next := Acquire(Param{Key: "id", Value: "7"})
	wg.Wait()
	Release(next)
}
