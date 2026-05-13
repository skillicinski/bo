use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub const MISSING_OPENAI_AUTH_MESSAGE: &str =
    "OpenAI API key not configured. Run: bo config auth --provider openai";

#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiApiKey(String);

impl OpenAiApiKey {
    pub fn new(raw: impl Into<String>) -> Result<Self, AuthError> {
        let key = raw.into().trim().to_string();
        if key.is_empty() {
            return Err(AuthError::EmptyApiKey);
        }
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OpenAiApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpenAiApiKey(<redacted>)")
    }
}

impl fmt::Display for OpenAiApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for OpenAiApiKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for OpenAiApiKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthFile {
    #[serde(default)]
    pub providers: AuthProviders,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthProviders {
    #[serde(default)]
    pub openai: Option<OpenAiAuth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiAuth {
    #[serde(default)]
    pub api_key: Option<OpenAiApiKey>,
}

#[derive(Debug)]
pub enum AuthError {
    NotFound,
    EmptyApiKey,
    Io(io::Error),
    Parse(serde_json::Error),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::NotFound => write!(f, "auth file not found"),
            AuthError::EmptyApiKey => write!(f, "OpenAI API key cannot be empty"),
            AuthError::Io(error) => write!(f, "auth I/O error: {error}"),
            AuthError::Parse(error) => write!(f, "auth parse error: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPermissionWarning {
    pub message: String,
}

impl AuthPermissionWarning {
    #[cfg(not(unix))]
    fn unsupported_platform() -> Self {
        Self {
            message: "could not apply restrictive permissions to auth storage on this platform"
                .to_string(),
        }
    }

    #[cfg(unix)]
    fn chmod_failed(error: &io::Error) -> Self {
        Self {
            message: format!("could not apply restrictive permissions to auth storage: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthWriteOutcome {
    pub path: PathBuf,
    pub permission_warning: Option<AuthPermissionWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSource {
    Environment,
    StoredAuth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOpenAiAuth {
    pub api_key: OpenAiApiKey,
    pub source: AuthSource,
}

#[derive(Debug)]
pub enum AuthResolutionError {
    Missing,
    Read(AuthError),
}

impl fmt::Display for AuthResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthResolutionError::Missing => write!(f, "{MISSING_OPENAI_AUTH_MESSAGE}"),
            AuthResolutionError::Read(error) => write!(f, "failed to read OpenAI auth: {error}"),
        }
    }
}

pub fn auth_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".bo").join("auth.json")
}

pub fn read_auth(path: &Path) -> Result<AuthFile, AuthError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(AuthError::NotFound),
        Err(error) => return Err(AuthError::Io(error)),
    };
    serde_json::from_str(&contents).map_err(AuthError::Parse)
}

pub fn write_openai_auth(
    path: &Path,
    api_key: OpenAiApiKey,
) -> Result<AuthWriteOutcome, AuthError> {
    let mut auth = match read_auth(path) {
        Ok(auth) => auth,
        Err(AuthError::NotFound) => AuthFile::default(),
        Err(error) => return Err(error),
    };

    auth.providers.openai = Some(OpenAiAuth {
        api_key: Some(api_key),
    });

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AuthError::Io)?;
    }

    let json = serde_json::to_string_pretty(&auth).map_err(AuthError::Parse)?;
    std::fs::write(path, json).map_err(AuthError::Io)?;

    let permission_warning = apply_restrictive_permissions(path);

    Ok(AuthWriteOutcome {
        path: path.to_path_buf(),
        permission_warning,
    })
}

pub fn resolve_openai_api_key(path: &Path) -> Result<ResolvedOpenAiAuth, AuthResolutionError> {
    if let Ok(raw) = std::env::var("OPENAI_API_KEY") {
        if let Ok(api_key) = OpenAiApiKey::new(raw) {
            return Ok(ResolvedOpenAiAuth {
                api_key,
                source: AuthSource::Environment,
            });
        }
    }

    let auth = match read_auth(path) {
        Ok(auth) => auth,
        Err(AuthError::NotFound) => return Err(AuthResolutionError::Missing),
        Err(error) => return Err(AuthResolutionError::Read(error)),
    };

    match auth.providers.openai.and_then(|openai| openai.api_key) {
        Some(api_key) => Ok(ResolvedOpenAiAuth {
            api_key,
            source: AuthSource::StoredAuth,
        }),
        None => Err(AuthResolutionError::Missing),
    }
}

#[cfg(unix)]
fn apply_restrictive_permissions(path: &Path) -> Option<AuthPermissionWarning> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, permissions)
        .err()
        .map(|error| AuthPermissionWarning::chmod_failed(&error))
}

#[cfg(not(unix))]
fn apply_restrictive_permissions(_path: &Path) -> Option<AuthPermissionWarning> {
    Some(AuthPermissionWarning::unsupported_platform())
}

#[cfg(test)]
#[path = "../tests/engine_auth_tests.rs"]
mod tests;
