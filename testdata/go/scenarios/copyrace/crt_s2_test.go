package copyrace

import "testing"

// S2 boundary: the route has no :id, so the copy has no params.
func TestCrtS2(t *testing.T) {
	c := Acquire()
	cp := c.Copy()
	got := cp.Param("id")
	t.Logf("cp.Param(\"id\") = %q", got)
	if got != "" {
		t.Fatalf("want empty, got %q", got)
	}
	Release(c)
}
