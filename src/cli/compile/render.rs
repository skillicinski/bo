// ── compile output rendering ──────────────────────────────────────────────────

use super::{BranchResult, CompileResult, NO_NEW_LEAVES_REASON};

pub(super) fn print_result(result: &CompileResult) {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => println!("bo is empty!"),
            Some("single_leaf") => println!("bo only has 1 leaf!"),
            Some(NO_NEW_LEAVES_REASON) => println!("nothing new to compile"),
            _ => println!("compiled: no work to do"),
        }
        return;
    }

    print_summary_parts(
        &result.branches,
        result.leaves_processed,
        &result.leaves_skipped,
    );
}

fn print_summary_parts(
    branches: &[BranchResult],
    leaves_processed: usize,
    leaves_skipped: &[String],
) {
    if branches.is_empty() {
        println!("compiled: no branches found");
    } else {
        println!(
            "compiled: {} {} from {} processed leaves",
            branches.len(),
            if branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            leaves_processed
        );
        for b in branches {
            println!(
                "  ✓ {} ({} {})",
                b.slug,
                b.leaf_count,
                if b.leaf_count == 1 { "leaf" } else { "leaves" }
            );
        }
    }

    if !leaves_skipped.is_empty() {
        println!();
        println!(
            "  ⚠ skipped {} {} (unparseable frontmatter):",
            leaves_skipped.len(),
            if leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        );
        for f in leaves_skipped {
            println!("    - {}", f);
        }
    }
}
