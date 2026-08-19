package domain_test

import (
	"testing"

	"github.com/skillicinski/bo/internal/domain"
)

func TestStateJSONIsStable(t *testing.T) {
	data, err := domain.MarshalState(domain.State{})
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "{\n  \"raw\": [],\n  \"summaries\": []\n}\n" {
		t.Fatalf("unexpected state: %q", data)
	}
}
