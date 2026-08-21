package local

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
)

const operationsFile = "log.jsonl"

// ponytail: process-local mutex; add a cross-process lock or database adapter if multiple writers need coordination.
var operationsMu sync.Mutex

type OperationLog struct {
	path string
}

func NewOperationLog(home string) *OperationLog {
	return &OperationLog{path: filepath.Join(home, ".bo", operationsFile)}
}

func (l *OperationLog) Append(ctx context.Context, operation domain.Operation) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	if operation.Timestamp == "" {
		operation.Timestamp = time.Now().UTC().Format(time.RFC3339Nano)
	}
	if operation.Actor == "" {
		operation.Actor = "system"
	}
	if operation.Details == nil {
		operation.Details = map[string]any{}
	}
	data, err := json.Marshal(operation)
	if err != nil {
		return fmt.Errorf("serializing operation failed: %w", err)
	}
	data = append(data, '\n')

	operationsMu.Lock()
	defer operationsMu.Unlock()
	if err := os.MkdirAll(filepath.Dir(l.path), 0o700); err != nil {
		return fmt.Errorf("creating operation log directory failed: %w", err)
	}
	file, err := os.OpenFile(l.path, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o600)
	if err != nil {
		return fmt.Errorf("opening operation log failed: %w", err)
	}
	if err := file.Chmod(0o600); err != nil {
		_ = file.Close()
		return fmt.Errorf("setting operation log permissions failed: %w", err)
	}
	if written, err := file.Write(data); err != nil {
		_ = file.Close()
		return fmt.Errorf("appending operation failed: %w", err)
	} else if written != len(data) {
		_ = file.Close()
		return io.ErrShortWrite
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return fmt.Errorf("syncing operation log failed: %w", err)
	}
	if err := file.Close(); err != nil {
		return fmt.Errorf("closing operation log failed: %w", err)
	}
	return nil
}

func (l *OperationLog) Read(ctx context.Context, directory string, offset, limit int) (application.OperationPage, error) {
	if err := contextErr(ctx); err != nil {
		return application.OperationPage{}, err
	}
	if offset < 0 {
		return application.OperationPage{}, fmt.Errorf("operation log offset must not be negative")
	}
	if limit <= 0 {
		limit = 20
	}
	page := application.OperationPage{Directory: directory, Entries: []domain.Operation{}, Offset: offset, Limit: limit, NextOffset: offset}

	operationsMu.Lock()
	defer operationsMu.Unlock()
	file, err := os.Open(l.path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return page, nil
		}
		return application.OperationPage{}, fmt.Errorf("opening operation log failed: %w", err)
	}
	defer file.Close()

	reader := bufio.NewReader(file)
	matched := 0
	for {
		line, readErr := reader.ReadString('\n')
		line = strings.TrimSpace(line)
		if line != "" {
			var operation domain.Operation
			if json.Unmarshal([]byte(line), &operation) == nil && operation.Directory == directory {
				if matched >= offset && len(page.Entries) < limit {
					page.Entries = append(page.Entries, operation)
				}
				matched++
				if len(page.Entries) == limit && matched > offset+limit {
					page.HasMore = true
					break
				}
			}
		}
		if errors.Is(readErr, io.EOF) {
			break
		}
		if readErr != nil {
			return application.OperationPage{}, fmt.Errorf("reading operation log failed: %w", readErr)
		}
	}
	page.NextOffset = offset + len(page.Entries)
	return page, nil
}
