package domain

import (
	"encoding/json"
)

type State struct {
	Raw       []RawRecord     `json:"raw"`
	Summaries []SummaryRecord `json:"summaries"`
}

func (s State) MarshalJSON() ([]byte, error) {
	raw := s.Raw
	if raw == nil {
		raw = []RawRecord{}
	}
	summaries := s.Summaries
	if summaries == nil {
		summaries = []SummaryRecord{}
	}
	type state State
	return json.Marshal(state{Raw: raw, Summaries: summaries})
}

type RawRecord struct {
	Filename  string `json:"filename"`
	URL       string `json:"url"`
	WrittenAt uint64 `json:"written_at"`
}

type SummaryRecord struct {
	Filename    string `json:"filename"`
	SourceKey   string `json:"source_key"`
	DerivedFrom string `json:"derived_from"`
	CreatedAt   uint64 `json:"created_at"`
	UpdatedAt   uint64 `json:"updated_at"`
}

func MarshalState(state State) ([]byte, error) {
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(data, '\n'), nil
}

func UnmarshalState(data []byte) (State, error) {
	var state State
	if err := json.Unmarshal(data, &state); err != nil {
		return State{}, err
	}
	if state.Raw == nil {
		state.Raw = []RawRecord{}
	}
	if state.Summaries == nil {
		state.Summaries = []SummaryRecord{}
	}
	return state, nil
}
