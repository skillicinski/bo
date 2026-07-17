// bo compile — validated deterministic and agent-assisted pipelines.
//
// Default: read leaves → LLM call(s) → parse/validate → write. With
// `--agent --dry-run`: bounded tool loop → validated plan → read-only preview.
// Both paths reject an invalid plan before mutation; the agent path is currently
// preview-only.

mod agent;
mod cluster;
mod dry_run;
mod execute;
mod journal;
mod orchestrate;
mod parse;
mod plan;
mod prompt;
mod render;
mod repair;
mod types;
mod validation;

// ── re-exports: public API ───────────────────────────────────────────────────

pub use dry_run::{run_compile_dry_run, run_compile_dry_run_with_provider};
pub use orchestrate::{
    degenerate_result_warning, run_compile_with_options, run_compile_with_provider_started_at,
};
pub use render::{render_diagnostics, render_human, render_preview_human};
pub use types::{
    BranchResult, CompileDryRunOutcome, CompileError, CompileOptions, CompileOutcome,
    CompilePreview, CompileResult, CompileRunMode, CompileSummary, PreviewBranch,
    VALIDATION_NEXT_STEP,
};

#[cfg(test)]
#[path = "../../tests/cli_compile_tests.rs"]
mod tests;
