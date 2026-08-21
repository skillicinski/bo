package local_test

import (
	"context"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/storage/local"
)

func operation(timestamp, directory string, command domain.OperationCommand) domain.Operation {
	return domain.Operation{Timestamp: timestamp, Actor: "test", Directory: directory, Command: command, Success: true, Details: map[string]any{"value": directory}}
}

func TestOperationLogCreatesAndReadsJSONL(t *testing.T) {
	home := t.TempDir()
	log := local.NewOperationLog(home)
	first := operation("2026-08-21T10:00:00.123456789Z", "notes", domain.CommandSeed)
	second := operation("2026-08-21T10:00:01Z", "other", domain.CommandState)
	if err := log.Append(context.Background(), first); err != nil {
		t.Fatal(err)
	}
	if err := log.Append(context.Background(), second); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(home, ".bo", "log.jsonl")
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if mode := info.Mode().Perm(); mode != 0o600 {
		t.Fatalf("operation log mode = %o", mode)
	}
	parsed, err := time.Parse(time.RFC3339Nano, first.Timestamp)
	if err != nil || parsed.Location() != time.UTC {
		t.Fatalf("timestamp = %q, %v", first.Timestamp, err)
	}

	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil {
		t.Fatal(err)
	}
	if page.Directory != "notes" || page.Offset != 0 || page.Limit != 20 || page.NextOffset != 1 || page.HasMore || len(page.Entries) != 1 {
		t.Fatalf("page = %#v", page)
	}
	if page.Entries[0].Command != domain.CommandSeed {
		t.Fatalf("entries = %#v", page.Entries)
	}
}

func TestOperationLogMissingFileAndPagination(t *testing.T) {
	log := local.NewOperationLog(t.TempDir())
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil || len(page.Entries) != 0 || page.HasMore || page.NextOffset != 0 {
		t.Fatalf("missing page = %#v, %v", page, err)
	}
	for index := 0; index < 3; index++ {
		if err := log.Append(context.Background(), operation("2026-08-21T10:00:0"+string(rune('0'+index))+"Z", "notes", domain.CommandSnap)); err != nil {
			t.Fatal(err)
		}
	}
	first, err := log.Read(context.Background(), "notes", 0, 2)
	if err != nil || len(first.Entries) != 2 || first.NextOffset != 2 || !first.HasMore {
		t.Fatalf("first page = %#v, %v", first, err)
	}
	second, err := log.Read(context.Background(), "notes", first.NextOffset, 2)
	if err != nil || len(second.Entries) != 1 || second.NextOffset != 3 || second.HasMore {
		t.Fatalf("second page = %#v, %v", second, err)
	}
}

func TestOperationLogSkipsMalformedLines(t *testing.T) {
	home := t.TempDir()
	log := local.NewOperationLog(home)
	if err := log.Append(context.Background(), operation("2026-08-21T10:00:00Z", "notes", domain.CommandSeed)); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(home, ".bo", "log.jsonl")
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteString("\nnot-json"); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	before, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil || len(page.Entries) != 1 {
		t.Fatalf("page = %#v, %v", page, err)
	}
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(before) != string(after) {
		t.Fatal("read modified malformed log")
	}
}

func TestOperationLogConcurrentAppends(t *testing.T) {
	log := local.NewOperationLog(t.TempDir())
	const count = 32
	var group sync.WaitGroup
	group.Add(count)
	for index := 0; index < count; index++ {
		go func(index int) {
			defer group.Done()
			if err := log.Append(context.Background(), operation("2026-08-21T10:00:00Z", "notes", domain.OperationCommand("test"+string(rune('a'+index))))); err != nil {
				t.Errorf("append: %v", err)
			}
		}(index)
	}
	group.Wait()
	page, err := log.Read(context.Background(), "notes", 0, count)
	if err != nil || len(page.Entries) != count || page.HasMore {
		t.Fatalf("page = %#v, %v", page, err)
	}
}
