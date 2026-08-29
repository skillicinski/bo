package domain

type DocumentKind string

const (
	DocumentKindRaw          DocumentKind = "raw"
	DocumentKindSummary      DocumentKind = "summary"
	DocumentKindDistillation DocumentKind = "distillation"
)

type DocumentRef struct {
	Kind DocumentKind
	Name string
}

func RawRef(name string) DocumentRef     { return DocumentRef{Kind: DocumentKindRaw, Name: name} }
func SummaryRef(name string) DocumentRef { return DocumentRef{Kind: DocumentKindSummary, Name: name} }
func DistillationRef(name string) DocumentRef {
	return DocumentRef{Kind: DocumentKindDistillation, Name: name}
}
