package application

import (
	"strings"
	"unicode"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func KebabCase(value string) (string, error) {
	var builder strings.Builder
	lastDash := false
	for _, r := range value {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			builder.WriteRune(unicode.ToLower(r))
			lastDash = false
		} else if builder.Len() > 0 && !lastDash {
			builder.WriteByte('-')
			lastDash = true
		}
	}
	result := strings.TrimRight(builder.String(), "-")
	if result == "" {
		return "", internalerrors.Validation("title cannot produce a filename")
	}
	return result, nil
}
