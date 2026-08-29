package domain

type DocumentKind string

const (
	DocumentKindRaw         DocumentKind = "raw"
	DocumentKindSummary     DocumentKind = "summary"
	DocumentKindSynthesized DocumentKind = "synthesized"
)

type DocumentRef struct {
	Kind DocumentKind
	Name string
}

func RawRef(name string) DocumentRef     { return DocumentRef{Kind: DocumentKindRaw, Name: name} }
func SummaryRef(name string) DocumentRef { return DocumentRef{Kind: DocumentKindSummary, Name: name} }
func SynthesizedRef(name string) DocumentRef {
	return DocumentRef{Kind: DocumentKindSynthesized, Name: name}
}

type SynthesizedKind string

const SynthesizedKindDistill SynthesizedKind = "distill"
