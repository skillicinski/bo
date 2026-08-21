package domain

import "encoding/json"

type OperationCommand string

const (
	CommandSeed         OperationCommand = "seed"
	CommandSnap         OperationCommand = "snap"
	CommandState        OperationCommand = "state"
	CommandSynth        OperationCommand = "synth"
	CommandWriteSummary OperationCommand = "write_summary"
)

type Operation struct {
	Timestamp string           `json:"timestamp"`
	Actor     string           `json:"actor"`
	Directory string           `json:"directory"`
	Command   OperationCommand `json:"command"`
	Success   bool             `json:"success"`
	Details   map[string]any   `json:"details"`
}

func (o Operation) MarshalJSON() ([]byte, error) {
	details := o.Details
	if details == nil {
		details = map[string]any{}
	}
	type operation Operation
	return json.Marshal(operation{Timestamp: o.Timestamp, Actor: o.Actor, Directory: o.Directory, Command: o.Command, Success: o.Success, Details: details})
}

func (o *Operation) UnmarshalJSON(data []byte) error {
	type operation Operation
	var value operation
	if err := json.Unmarshal(data, &value); err != nil {
		return err
	}
	if value.Details == nil {
		value.Details = map[string]any{}
	}
	*o = Operation(value)
	return nil
}
