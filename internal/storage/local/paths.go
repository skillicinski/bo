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

const (
	stateFile = "state.json"
	eventFile = "log.jsonl"
	// ponytail: cap generated-name retries at 8; use coordinated allocation if collisions become common.
	maxGeneratedWorkspaceAttempts = 8
)

var adjectives = []string{
	"amber", "brisk", "calm", "clever", "crisp", "eager", "gentle", "hidden", "mellow", "quiet",
	"rapid", "silver", "steady", "tidy", "vivid",
}

var nouns = []string{
	"badger", "cedar", "comet", "falcon", "meadow", "otter", "panda", "quartz", "river", "sparrow",
	"sunset", "thicket", "willow", "wren", "zephyr",
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

var randomWorkspaceName = RandomName

func Seed(home string, requestedName *string) (string, error) {
	return seed(home, requestedName, nil)
}

func SeedWithEvent(home string, requestedName *string, event domain.Operation) (string, error) {
	return seed(home, requestedName, &event)
}

func seed(home string, requestedName *string, event *domain.Operation) (string, error) {
	rootPath := filepath.Join(home, ".bo")
	attempts := 1
	if requestedName == nil {
		attempts = maxGeneratedWorkspaceAttempts
	}
	for attempt := 0; attempt < attempts; attempt++ {
		name := ""
		if requestedName == nil {
			var err error
			name, err = randomWorkspaceName()
			if err != nil {
				return "", internalerrors.Wrap(internalerrors.KindFilesystem, "generating workspace name failed", err)
			}
		} else {
			name = *requestedName
		}
		if err := ValidateName(name); err != nil {
			return "", err
		}
		if err := os.MkdirAll(rootPath, 0o700); err != nil {
			return "", filesystem(rootPath, err)
		}
		target := filepath.Join(rootPath, name)
		if _, err := os.Lstat(target); err == nil {
			if requestedName != nil {
				return "", internalerrors.Wrap(internalerrors.KindAlreadyExists, "workspace already exists", internalerrors.ErrAlreadyExists)
			}
			continue
		} else if !os.IsNotExist(err) {
			return "", filesystem(target, err)
		}
		collision, err := seedWorkspaceAttempt(rootPath, target, event)
		if err != nil {
			return "", err
		}
		if collision {
			continue
		}
		return target, nil
	}
	return "", internalerrors.Wrap(internalerrors.KindAlreadyExists, "workspace already exists", internalerrors.ErrAlreadyExists)
}

func seedWorkspaceAttempt(rootPath, target string, event *domain.Operation) (bool, error) {
	temporary, err := os.MkdirTemp(rootPath, ".bo-workspace-")
	if err != nil {
		return false, filesystem(rootPath, err)
	}
	renamed := false
	defer func() {
		if !renamed {
			_ = os.RemoveAll(temporary)
		}
	}()
	state := domain.State{Sources: []domain.SourceRecord{}}
	if event != nil {
		event.Normalize()
		if err := event.ValidateSeed(); err != nil {
			return false, err
		}
	}
	if err := initializeState(temporary, state); err != nil {
		return false, err
	}
	if err := initializeEvents(temporary, event); err != nil {
		return false, err
	}
	if err := syncDirectory(temporary); err != nil {
		return false, err
	}
	if err := os.Rename(temporary, target); err != nil {
		if errors.Is(err, os.ErrExist) {
			return true, nil
		}
		return false, filesystem(target, err)
	}
	renamed = true
	if err := syncDirectory(rootPath); err != nil {
		return false, err
	}
	return false, nil
}

func initializeEvents(target string, event *domain.Operation) error {
	var data []byte
	if event != nil {
		var err error
		data, err = marshalEventLine(*event)
		if err != nil {
			return err
		}
	}
	root, err := os.OpenRoot(target)
	if err != nil {
		return filesystem(target, err)
	}
	defer root.Close()
	file, err := root.OpenFile(eventFile, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return filesystem(filepath.Join(target, eventFile), err)
	}
	var written int
	if written, err = file.Write(data); err == nil && written != len(data) {
		err = io.ErrShortWrite
	}
	if err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		_ = root.Remove(eventFile)
		return filesystem(filepath.Join(target, eventFile), err)
	}
	return syncRoot(root)
}

func syncDirectory(path string) error {
	directory, err := os.Open(path)
	if err != nil {
		return filesystem(path, err)
	}
	defer directory.Close()
	if err := directory.Sync(); err != nil {
		return filesystem(path, err)
	}
	return nil
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
