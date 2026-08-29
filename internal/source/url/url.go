package url

import (
	"context"
	"net/http"
	"net/url"
	"strings"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

const DefaultUserAgent = "bo/0.1"

type Requester interface {
	Do(*http.Request) (*http.Response, error)
}

type Transport struct{}

func NewTransport() *Transport { return &Transport{} }

func sourceFailure(detail string, err error) error {
	if contextErr := internalerrors.Context(err); contextErr != nil {
		return contextErr
	}
	return internalerrors.Wrap(internalerrors.KindSource, detail, err)
}

func (t *Transport) Route(ctx context.Context, input string) (source.Origin, error) {
	if err := ctx.Err(); err != nil {
		return source.Origin{}, internalerrors.Context(err)
	}
	parsed, err := url.Parse(input)
	if err != nil {
		if strings.Contains(input, "://") || strings.HasPrefix(input, "http:") || strings.HasPrefix(input, "https:") {
			return source.Origin{}, internalerrors.Wrap(internalerrors.KindValidation, "invalid URL", err)
		}
		return source.Origin{}, source.ErrNotHandled
	}
	if (parsed.Scheme == "http" || parsed.Scheme == "https") && (parsed.Fragment != "" || strings.Contains(input, "#")) {
		return source.Origin{}, internalerrors.Validation("URL must not contain a fragment")
	}
	match := ClassifyYouTubeURL(input)
	if parsed.Scheme == "http" || parsed.Scheme == "https" {
		if parsed.Host == "" {
			return source.Origin{}, internalerrors.Validation("URL must include a host")
		}
		if err := domain.ValidateSourceKey(input); err != nil {
			return source.Origin{}, err
		}
	}
	if match.Kind == YouTubeSupported {
		return source.NewOrigin(source.OriginYouTube, input, input), nil
	}
	if match.Kind == YouTubeUnsupported {
		return source.Origin{}, internalerrors.Source("YouTube URL: " + match.Reason)
	}
	if parsed.Scheme == "" {
		return source.Origin{}, source.ErrNotHandled
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return source.Origin{}, internalerrors.Validation("URL scheme must be http or https")
	}
	if parsed.Host == "" {
		return source.Origin{}, internalerrors.Validation("URL must include a host")
	}
	return source.NewOrigin(source.OriginHTML, input, parsed.String()), nil
}
