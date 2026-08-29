# Document ontology

`state.json` is the system-managed index for a seeded directory. It stores
one aggregate for each exact source identity:

```mermaid
erDiagram
    SOURCE_IDENTITY ||--o{ RAW_SNAPSHOT : contains
    SOURCE_IDENTITY ||--o| SUMMARY_DOCUMENT : has_current
    RAW_SNAPSHOT |o..|| SUMMARY_DOCUMENT : is_derived_from

    SOURCE_IDENTITY {
        string source_key PK
    }
    RAW_SNAPSHOT {
        string filename
        timestamp written_at
    }
    SUMMARY_DOCUMENT {
        string filename
        string derived_from FK
        timestamp created_at
        timestamp updated_at
    }
    DISTILLATION_DOCUMENT {
        string filename PK
        string kind
        timestamp created_at
        timestamp updated_at
    }
    SOURCE_DOCUMENT_INPUT {
        string source_key FK
        string kind
        string filename FK
        string content_digest
    }
    DISTILLATION_DOCUMENT }o--o{ SOURCE_DOCUMENT_INPUT : cites
```

## SourceIdentity

A source identity is the exact source key recorded by `bo snap`. HTTP and
HTTPS URLs without fragments are stored as-is. A Markdown file uses
`raw:<filename>` as its source identity. URL canonicalization is intentionally
deferred.

## SourceRecord

Each source aggregate contains zero or more immutable raw snapshots and at
most one current summary. Snapshot filenames are unique in the target
directory. A summary's `derived_from` value must identify one snapshot in the
same aggregate.

Timestamps use UTC RFC 3339 values throughout the state format:

```json
{
  "sources": [
    {
      "source_key": "https://example.com/foo",
      "snapshots": [
        {
          "filename": "foo--123456.md",
          "written_at": "2026-08-23T12:34:56.123456789Z"
        }
      ],
      "summary": {
        "filename": "foo.md",
        "derived_from": "foo--123456.md",
        "created_at": "2026-08-23T12:35:00Z",
        "updated_at": "2026-08-23T12:35:00Z"
      }
    }
  ]
}
```

State is validated when it is loaded and before it is published. Raw
snapshots remain append-only evidence. Rewriting a summary preserves its
filename and creation time, updates its timestamp, and replaces its
`derived_from` reference.

Document entries may also contain `content_digest`, `content_size`, and
`content_modified_at` inventory metadata for conditional writes. These fields
are not operation events.

`DISTILLATION_DOCUMENT` records are stored in `distillation_documents` under
`distillations/` and use
`derived_from` inputs with `source_key`, `kind`, `filename`, and a SHA-256
`content_digest`. Raw and summary inputs from one source count as one source;
each distill record must cite at least two source identities. Distillation
documents are create-only and do not provide evidence to the distill agent.
