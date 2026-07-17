use crate::cli::json::JsonWarning;
use crate::domain::tree::TreeRuntimeState;
use crate::domain::{manifest, tree};
use crate::engine::config::Config;
use crate::engine::pending::{self, OpKind};

use serde::Serialize;
use serde_json::json;
use std::io::{BufRead, ErrorKind as IoErrorKind, Write};
use std::path::{Component, Path};

#[derive(Debug, Clone, Serialize, Default)]
pub struct RazeResult {
    /// Signals the user declined confirmation. All other fields are zero/false when cancelled.
    #[serde(default)]
    pub cancelled: bool,
    pub deleted_files: usize,
    pub deleted_manifest: bool,
    pub removed_output_dir: bool,
    pub output_dir_left_in_place: bool,
    pub deleted_config: bool,
    pub deleted_auth: bool,
    pub preserved_auth: bool,
    pub output_dir: String,
    pub config_path: String,
    pub auth_path: String,
}

#[derive(Debug)]
pub struct RazeOutput {
    pub result: RazeResult,
    pub warnings: Vec<JsonWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCleanup {
    Preserve,
    Delete,
}

impl AuthCleanup {
    fn deletes_auth(self) -> bool {
        matches!(self, Self::Delete)
    }
}

#[derive(Debug)]
pub enum RazeError {
    Io(String),
    Busy(String),
}

impl std::fmt::Display for RazeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "{}", msg),
            Self::Busy(msg) => write!(f, "{}", msg),
        }
    }
}

pub fn run(
    config: Option<Config>,
    config_path: &Path,
    auth_path: &Path,
    include_auth: bool,
) -> Result<Option<RazeOutput>, RazeError> {
    match config {
        Some(cfg) => {
            let Some(seeded) = cfg.into_seeded() else {
                return Ok(None);
            };
            let auth_cleanup = if include_auth {
                AuthCleanup::Delete
            } else {
                AuthCleanup::Preserve
            };
            let tree = seeded.tree();
            Ok(Some(raze_with_auth(
                tree.path(),
                config_path,
                auth_path,
                auth_cleanup,
            )?))
        }
        None => {
            if !include_auth {
                return Ok(None);
            }
            // Same hardening as the seeded-tree path: refuse non-interactive
            // invocation and let the user back out before credentials are deleted.
            #[cfg(not(test))]
            {
                if confirm_auth_only_interactive(auth_path)? {
                    return Ok(Some(RazeOutput {
                        result: RazeResult {
                            cancelled: true,
                            auth_path: auth_path.to_string_lossy().into_owned(),
                            ..Default::default()
                        },
                        warnings: Vec::new(),
                    }));
                }
            }
            match raze_auth_only(auth_path)? {
                Some(output) => Ok(Some(output)),
                None => Ok(Some(RazeOutput {
                    result: RazeResult {
                        auth_path: auth_path.to_string_lossy().into_owned(),
                        ..Default::default()
                    },
                    warnings: Vec::new(),
                })),
            }
        }
    }
}

pub fn raze(output_dir: &Path, config_path: &Path) -> Result<RazeOutput, RazeError> {
    let auth_path = config_path.with_file_name("auth.json");
    raze_with_auth(output_dir, config_path, &auth_path, AuthCleanup::Preserve)
}

pub fn raze_with_auth(
    output_dir: &Path,
    config_path: &Path,
    auth_path: &Path,
    auth_cleanup: AuthCleanup,
) -> Result<RazeOutput, RazeError> {
    recover_pending_if_needed(output_dir)?;

    let manifest_path = tree::manifest_path(output_dir);
    let manifest = match crate::engine::manifest::runtime_state(output_dir) {
        Ok(TreeRuntimeState::Initialized(manifest)) => Some(manifest),
        Ok(TreeRuntimeState::FreshSeeded | TreeRuntimeState::MissingManifest) => None,
        Err(error) => return Err(RazeError::Io(format!("failed to read manifest: {error}"))),
    };

    // There is no non-interactive bypass; integration tests verify the refusal.
    #[cfg(not(test))]
    {
        let include_auth = auth_cleanup.deletes_auth();
        if confirm_raze_interactive(output_dir, manifest.as_ref(), include_auth)? {
            return Ok(RazeOutput {
                result: RazeResult {
                    cancelled: true,
                    deleted_files: 0,
                    deleted_manifest: false,
                    removed_output_dir: false,
                    output_dir_left_in_place: false,
                    deleted_config: false,
                    deleted_auth: false,
                    preserved_auth: !include_auth,
                    output_dir: path_string(output_dir),
                    config_path: path_string(config_path),
                    auth_path: path_string(auth_path),
                },
                warnings: vec![],
            });
        }
    }

    let mut warnings = Vec::new();
    let mut deletes: Vec<String> = Vec::new();

    if let Some(manifest) = &manifest {
        for leaf in &manifest.leaves {
            push_manifest_delete(&mut deletes, &mut warnings, &leaf.file);
        }
        for branch in &manifest.branches {
            push_manifest_delete(&mut deletes, &mut warnings, &branch.file);
        }
    }

    let deleted_files = deletes
        .iter()
        .filter(|relative| output_dir.join(relative).is_file())
        .count();

    if output_dir.join("branch").exists() {
        deletes.push("branch".to_string());
    }
    if output_dir.join("leaf").exists() {
        deletes.push("leaf".to_string());
    }
    deletes.push(".bo".to_string());

    let operation = pending::new_operation(
        output_dir,
        OpKind::Raze {
            include_auth: auth_cleanup.deletes_auth(),
        },
        Vec::new(),
        deletes.clone(),
    )
    .map_err(map_pending_error)?;
    let pending_path = pending::pending_path(output_dir);
    pending::write(&pending_path, &operation).map_err(map_pending_error)?;

    match std::fs::remove_file(&manifest_path) {
        Ok(()) => {}
        Err(error) if error.kind() == IoErrorKind::NotFound => {}
        Err(error) => {
            return Err(RazeError::Io(format!(
                "failed to delete manifest: {}",
                error
            )));
        }
    }

    pending::apply_deletes(output_dir, &deletes).map_err(map_pending_error)?;
    pending::clear(&pending_path).map_err(map_pending_error)?;
    let deleted_manifest = true;

    let (removed_output_dir, output_dir_left_in_place) = match std::fs::remove_dir(output_dir) {
        Ok(()) => (true, false),
        Err(error)
            if error.kind() == IoErrorKind::DirectoryNotEmpty
                || error.kind() == IoErrorKind::NotFound =>
        {
            (false, true)
        }
        Err(error) => {
            return Err(RazeError::Io(format!(
                "failed to remove output directory: {}",
                error
            )));
        }
    };

    let delete_auth = auth_cleanup.deletes_auth();
    let deleted_config = delete_optional_file(config_path, "config")?;
    let preserved_auth = !delete_auth && auth_path.exists();
    let deleted_auth = if delete_auth {
        delete_optional_file(auth_path, "auth")?
    } else {
        false
    };

    Ok(RazeOutput {
        result: RazeResult {
            cancelled: false,
            deleted_files,
            deleted_manifest,
            removed_output_dir,
            output_dir_left_in_place,
            deleted_config,
            deleted_auth,
            preserved_auth,
            output_dir: path_string(output_dir),
            config_path: path_string(config_path),
            auth_path: path_string(auth_path),
        },
        warnings,
    })
}

pub fn raze_auth_only(auth_path: &Path) -> Result<Option<RazeOutput>, RazeError> {
    let deleted_auth = delete_optional_file(auth_path, "auth")?;
    if !deleted_auth {
        return Ok(None);
    }

    Ok(Some(RazeOutput {
        result: RazeResult {
            cancelled: false,
            deleted_files: 0,
            deleted_manifest: false,
            removed_output_dir: false,
            output_dir_left_in_place: false,
            deleted_config: false,
            deleted_auth,
            preserved_auth: false,
            output_dir: String::new(),
            config_path: String::new(),
            auth_path: path_string(auth_path),
        },
        warnings: Vec::new(),
    }))
}

/// Run the confirmation gate with real stdin/stderr. Returns `Ok(true)` if the
/// user cancelled (caller should return early with no mutation). Returns
/// `Ok(false)` if confirmed. Returns `Err` if stdin is not a TTY, refusing to
/// run non-interactively.
#[cfg(not(test))]
fn confirm_raze_interactive(
    tree_root: &Path,
    manifest: Option<&manifest::Manifest>,
    include_auth: bool,
) -> Result<bool, RazeError> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(RazeError::Io(
            "raze requires an interactive terminal for confirmation. Refusing to run non-interactively."
                .into(),
        ));
    }
    let confirmed = confirm_raze(
        tree_root,
        manifest,
        include_auth,
        &mut std::io::BufReader::new(std::io::stdin()),
        &mut std::io::stderr(),
    )?;
    Ok(!confirmed)
}

/// Credential-only confirmation gate. Same contract as `confirm_raze_interactive`:
/// `Ok(true)` if the user cancelled, `Ok(false)` if confirmed, `Err` if stdin is
/// not a TTY.
#[cfg(not(test))]
fn confirm_auth_only_interactive(auth_path: &Path) -> Result<bool, RazeError> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(RazeError::Io(
            "raze requires an interactive terminal for confirmation. Refusing to run non-interactively."
                .into(),
        ));
    }
    let confirmed = confirm_auth_only(
        auth_path,
        &mut std::io::BufReader::new(std::io::stdin()),
        &mut std::io::stderr(),
    )?;
    Ok(!confirmed)
}

/// Print a credential-deletion prompt to `writer`, read response from `reader`.
/// Returns `true` for exact "yes\n", `false` for anything else.
fn confirm_auth_only(
    auth_path: &Path,
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> Result<bool, RazeError> {
    writeln!(
        writer,
        "This will permanently delete your bo credentials at {}:",
        auth_path.display()
    )
    .map_err(map_io_error)?;
    write!(writer, "Type 'yes' to confirm: ").map_err(map_io_error)?;
    writer.flush().map_err(map_io_error)?;

    let mut buf = String::new();
    reader.read_line(&mut buf).map_err(map_io_error)?;
    Ok(buf == "yes\n")
}

fn map_io_error(error: std::io::Error) -> RazeError {
    RazeError::Io(error.to_string())
}

/// Print confirmation prompt to `writer`, read response from `reader`.
/// Returns `true` for exact "yes\n", `false` for anything else.
fn confirm_raze(
    tree_root: &Path,
    manifest: Option<&manifest::Manifest>,
    include_auth: bool,
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> Result<bool, RazeError> {
    writeln!(
        writer,
        "This will permanently delete the tree at {}:",
        tree_root.display()
    )
    .map_err(map_io_error)?;
    if let Some(m) = manifest {
        writeln!(
            writer,
            "  {} leaves, {} branches",
            m.leaves.len(),
            m.branches.len()
        )
        .map_err(map_io_error)?;
    } else {
        writeln!(
            writer,
            "  unable to read manifest \u{2014} cannot determine leaf/branch count"
        )
        .map_err(map_io_error)?;
    }
    if include_auth {
        writeln!(writer, "  Auth credentials: will be deleted").map_err(map_io_error)?;
    } else {
        writeln!(
            writer,
            "  Auth credentials: preserved (use --include-auth to also delete)"
        )
        .map_err(map_io_error)?;
    }
    write!(writer, "Type 'yes' to confirm: ").map_err(map_io_error)?;
    writer.flush().map_err(map_io_error)?;

    let mut buf = String::new();
    reader.read_line(&mut buf).map_err(map_io_error)?;
    Ok(buf == "yes\n")
}

fn recover_pending_if_needed(output_dir: &Path) -> Result<(), RazeError> {
    if let Some(report) = pending::recover_or_refuse(output_dir).map_err(map_pending_error)? {
        eprintln!(
            "recovered {} changes from interrupted {}",
            report.changes, report.op
        );
    }
    Ok(())
}

fn map_pending_error(error: pending::PendingError) -> RazeError {
    match error {
        pending::PendingError::Busy { .. } => RazeError::Busy(error.to_string()),
        other => RazeError::Io(other.to_string()),
    }
}

fn push_manifest_delete(deletes: &mut Vec<String>, warnings: &mut Vec<JsonWarning>, file: &str) {
    if is_suspicious_relative_path(file) {
        warnings.push(JsonWarning::with_details(
            "suspicious_manifest_entry",
            format!("skipping manifest entry with suspicious path: {file}"),
            json!({ "file": file }),
        ));
        return;
    }
    deletes.push(file.to_string());
}

fn delete_optional_file(path: &Path, label: &str) -> Result<bool, RazeError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == IoErrorKind::NotFound => Ok(false),
        Err(error) => Err(RazeError::Io(format!(
            "failed to delete {}: {}",
            label, error
        ))),
    }
}

pub fn render_human(result: &RazeResult) -> String {
    if result.cancelled {
        return "raze cancelled\n".into();
    }

    let mut out = String::new();

    if !result.output_dir.is_empty() {
        out.push_str(&format!(
            "deleted {} markdown file(s)\n",
            result.deleted_files
        ));
    }

    if result.deleted_manifest {
        out.push_str("deleted manifest\n");
    }

    if result.removed_output_dir {
        out.push_str(&format!("removed output directory {}\n", result.output_dir));
    } else if result.output_dir_left_in_place {
        out.push_str(&format!(
            "output directory left in place (not empty or already absent): {}\n",
            result.output_dir
        ));
    }

    if result.deleted_config {
        out.push_str("deleted config\n");
    }

    if result.deleted_auth {
        out.push_str("deleted auth\n");
    } else if result.preserved_auth {
        out.push_str(&format!("preserved auth: {}\n", result.auth_path));
    }

    out
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn is_suspicious_relative_path(file: &str) -> bool {
    let relative = Path::new(file);
    relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
}

#[cfg(test)]
#[path = "../tests/cli_raze_tests.rs"]
mod tests;
