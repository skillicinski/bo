package url

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"

	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

const DefaultUserAgent = "bo/0.1"

type Requester interface {
	Do(*http.Request) (*http.Response, error)
}

type Transport struct{}

func NewTransport() *Transport { return &Transport{} }

func (t *Transport) Route(_ context.Context, input string) (source.Origin, error) {
	match := ClassifyYouTubeURL(input)
	if match.Kind == YouTubeSupported {
		return source.NewOrigin(source.OriginYouTube, input, input), nil
	}
	if match.Kind == YouTubeUnsupported {
		return source.Origin{}, internalerrors.Unsupported("YouTube URL: " + match.Reason)
	}
	parsed, err := url.Parse(input)
	if err != nil {
		if strings.Contains(input, "://") || strings.HasPrefix(input, "http:") || strings.HasPrefix(input, "https:") {
			return source.Origin{}, internalerrors.Input(fmt.Sprintf("invalid URL: %v", err))
		}
		return source.Origin{}, source.ErrNotHandled
	}
	if parsed.Scheme == "" {
		return source.Origin{}, source.ErrNotHandled
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return source.Origin{}, internalerrors.Input("URL scheme must be http or https")
	}
	if parsed.Host == "" {
		return source.Origin{}, internalerrors.Input("URL must include a host")
	}
	return source.NewOrigin(source.OriginHTML, input, parsed.String()), nil
}
