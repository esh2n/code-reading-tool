package copyrace

import "testing"

// S1 normal: a copy reads the parameter of its request.
func TestCrtS1(t *testing.T) {
	c := Acquire(Param{Key: "id", Value: "42"})
	cp := c.Copy()
	got := cp.Param("id")
	t.Logf("cp.Param(\"id\") = %q", got)
	if got != "42" {
		t.Fatalf("want 42, got %q", got)
	}
	Release(c)
}
