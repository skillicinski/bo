use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct TempHome(PathBuf);

impl TempHome {
    pub fn new(label: &str, seeded: bool) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "bo-integration-{label}-{}-{suffix}",
            std::process::id()
        ));
        if seeded {
            let target = path.join(".bo/notes");
            fs::create_dir_all(&target).unwrap();
            fs::write(
                target.join("state.json"),
                "{\n  \"raw\": [],\n  \"summaries\": []\n}\n",
            )
            .unwrap();
        } else {
            fs::create_dir(&path).unwrap();
        }
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bo"));
    command
        .env("HOME", home)
        .env_remove("USERPROFILE")
        .env_remove("BO_API_URL");
    command
}
