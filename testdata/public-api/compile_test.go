package main

import "github.com/skillicinski/bo"

var (
	_ bo.Workspace        = workspace{}
	_ bo.WorkspaceEvents  = workspace{}
	_ bo.WorkspaceCreator = creator{}

	_ = bo.DefaultSynthesisOptions
	_ = bo.Distill
	_ = bo.NewDeepSeekProvider
	_ = bo.NewError
	_ = bo.NewLocalManager
	_ = bo.NewRevision
	_ = bo.RawRef
	_ = bo.ReadState
	_ = bo.Seed
	_ = bo.Snap
	_ = bo.SummaryRef
	_ = bo.Synth
	_ = bo.WrapError
	_ = bo.IsAlreadyExists
	_ = bo.IsKind
)
