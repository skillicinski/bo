package bo

import "context"

type Page struct {
	Title     string
	Markdown  string
	SourceURL string
}

type Source interface {
	Fetch(context.Context, string) (Page, error)
}

type Fetcher = Source
