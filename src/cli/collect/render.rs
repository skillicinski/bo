// Collect stage: human-readable output only.

use std::io::Write;

use super::{BatchCollectResult, CollectItemStatus, CollectResult};

pub fn render_human<W: Write>(result: &CollectResult, stdout: &mut W) -> std::io::Result<()> {
    writeln!(stdout, "✓ collected: {} → {}", result.url, result.file)
}

pub fn render_batch_human<W: Write>(
    result: &BatchCollectResult,
    stdout: &mut W,
) -> std::io::Result<()> {
    for item in &result.items {
        let label = item.url.as_deref().unwrap_or(&item.input);
        match item.status {
            CollectItemStatus::Collected => writeln!(
                stdout,
                "✓ collected: {} → {}",
                label,
                item.file.as_deref().unwrap_or("")
            )?,
            CollectItemStatus::Skipped => writeln!(
                stdout,
                "↷ skipped: {} ({})",
                label,
                item.message.as_deref().unwrap_or("skipped")
            )?,
            CollectItemStatus::Failed => writeln!(
                stdout,
                "✗ failed: {} ({})",
                label,
                item.message.as_deref().unwrap_or("failed")
            )?,
        }
    }

    writeln!(
        stdout,
        "collect summary: {} collected, {} skipped, {} failed",
        result.summary.collected, result.summary.skipped, result.summary.failed
    )
}
