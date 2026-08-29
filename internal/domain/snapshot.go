package domain

type RawSnapshot struct {
	SourceKey string
	Title     string
	Markdown  []byte
}

func NewRawSnapshot(sourceKey, title string, markdown []byte) RawSnapshot {
	return RawSnapshot{SourceKey: sourceKey, Title: title, Markdown: append([]byte(nil), markdown...)}
}
