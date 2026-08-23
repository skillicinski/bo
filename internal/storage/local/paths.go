package local

import (
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"unicode"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const stateFile = "state.json"

var adjectives = []string{
	"amber", "brisk", "calm", "clever", "crisp", "eager", "gentle", "hidden", "mellow", "quiet",
	"rapid", "silver", "steady", "tidy", "vivid",
}

var nouns = []string{
	"badger", "cedar", "comet", "falcon", "meadow", "otter", "panda", "quartz", "river", "sparrow",
	"sunset", "thicket", "willow", "wren", "zephyr",
}

func HomeDir() (string, error) {
	for _, name := range []string{"HOME", "USERPROFILE"} {
		if value := os.Getenv(name); value != "" {
			return value, nil
		}
	}
	return "", internalerrors.Validation("HOME or USERPROFILE is not set")
}

func ValidateName(name string) error {
	if name == "" {
		return internalerrors.Validation("name must not be empty")
	}
	if name == "." || name == ".." || strings.ContainsAny(name, `/\`) {
		return internalerrors.Validation("name must be a single directory component")
	}
	if strings.HasSuffix(name, ".") || strings.HasSuffix(name, " ") {
		return internalerrors.Validation("name must not end with a dot or space")
	}
	for _, r := range name {
		if unicode.IsControl(r) || strings.ContainsRune(`<>:"|?*`, r) {
			return internalerrors.Validation("name contains an invalid character")
		}
	}
	if reservedDeviceName(name) {
		return internalerrors.Validation("name is reserved on Windows")
	}
	return nil
}

func reservedDeviceName(name string) bool {
	stem := name
	if index := strings.IndexByte(stem, '.'); index >= 0 {
		stem = stem[:index]
	}
	if strings.EqualFold(stem, "CON") || strings.EqualFold(stem, "PRN") ||
		strings.EqualFold(stem, "AUX") || strings.EqualFold(stem, "NUL") {
		return true
	}
	return len(stem) == 4 && (strings.EqualFold(stem[:3], "COM") || strings.EqualFold(stem[:3], "LPT")) &&
		stem[3] >= '1' && stem[3] <= '9'
}

func RandomName() (string, error) {
	var bytes [2]byte
	if _, err := io.ReadFull(rand.Reader, bytes[:]); err != nil {
		return "", err
	}
	return adjectives[int(bytes[0])%len(adjectives)] + "-" + nouns[int(bytes[1])%len(nouns)], nil
}

func Seed(home string, requestedName *string) (string, error) {
	name := ""
	if requestedName == nil {
		var err error
		name, err = RandomName()
		if err != nil {
			return "", internalerrors.Wrap(internalerrors.KindFilesystem, "generating workspace name failed", err)
		}
	} else {
		name = *requestedName
	}
	if err := ValidateName(name); err != nil {
		return "", err
	}

	rootPath := filepath.Join(home, ".bo")
	if err := os.MkdirAll(rootPath, 0o755); err != nil {
		return "", filesystem(rootPath, err)
	}
	target := filepath.Join(rootPath, name)
	if err := os.Mkdir(target, 0o755); err != nil {
		if errors.Is(err, os.ErrExist) {
			return "", internalerrors.Wrap(internalerrors.KindAlreadyExists, "workspace already exists", internalerrors.ErrAlreadyExists)
		}
		return "", filesystem(target, err)
	}
	if err := initializeState(target, domain.State{Sources: []domain.SourceRecord{}}); err != nil {
		return "", err
	}
	return target, nil
}

func initializeState(target string, state domain.State) error {
	root, err := os.OpenRoot(target)
	if err != nil {
		return filesystem(target, err)
	}
	defer root.Close()
	if _, err := root.Lstat(stateFile); err == nil {
		return internalerrors.AlreadyExists("state file already exists")
	} else if !os.IsNotExist(err) {
		return filesystem(filepath.Join(target, stateFile), err)
	}
	data, err := domain.MarshalState(state)
	if err != nil {
		return err
	}
	temporary, err := root.OpenFile("."+stateFile+".tmp", os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return filesystem(filepath.Join(target, "."+stateFile+".tmp"), err)
	}
	if _, err = temporary.Write(data); err == nil {
		err = temporary.Sync()
	}
	closeErr := temporary.Close()
	if err == nil {
		err = closeErr
	}
	if err == nil {
		err = root.Rename("."+stateFile+".tmp", stateFile)
	}
	if err != nil {
		_ = root.Remove("." + stateFile + ".tmp")
		return filesystem(filepath.Join(target, stateFile), err)
	}
	return syncRoot(root)
}

func ResolveTarget(home, name string) (string, error) {
	if err := ValidateName(name); err != nil {
		return "", err
	}
	root := filepath.Join(home, ".bo")
	target := filepath.Join(root, name)
	info, err := os.Stat(target)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return "", internalerrors.Wrap(internalerrors.KindMissingResource, fmt.Sprintf("target directory does not exist: %s", target), err)
		}
		return "", filesystem(target, err)
	}
	if !info.IsDir() {
		return "", internalerrors.Validation(fmt.Sprintf("target is not a directory: %s", target))
	}
	canonicalRoot, err := filepath.EvalSymlinks(root)
	if err != nil {
		return "", filesystem(root, err)
	}
	canonicalTarget, err := filepath.EvalSymlinks(target)
	if err != nil {
		return "", filesystem(target, err)
	}
	if err := ensureInside(canonicalTarget, canonicalRoot); err != nil {
		return "", err
	}
	return canonicalTarget, nil
}

func ensureInside(path, root string) error {
	relative, err := filepath.Rel(root, path)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) || filepath.IsAbs(relative) {
		return internalerrors.Validation(fmt.Sprintf("path escapes %s: %s", root, path))
	}
	return nil
}
