package application

import (
	"errors"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func normalizeError(err error, kind internalerrors.Kind, detail string) error {
	if err == nil {
		return nil
	}
	var categorized *internalerrors.Error
	if errors.As(err, &categorized) {
		return err
	}
	if contextErr := internalerrors.Context(err); contextErr != nil {
		return contextErr
	}
	return internalerrors.Wrap(kind, detail, err)
}
