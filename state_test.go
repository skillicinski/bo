package bo_test

import (
	"testing"

	"github.com/skillicinski/bo"
)

func TestStateJSONIsStable(t *testing.T) {
	data, err := bo.MarshalState(bo.State{})
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "{\n  \"raw\": [],\n  \"summaries\": []\n}\n" {
		t.Fatalf("unexpected state: %q", data)
	}
}
