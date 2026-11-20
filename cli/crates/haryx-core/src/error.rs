//! Error kinds of haryx, mirroring `claude_swap/exceptions.py`.
//!
//! One enum instead of a class tree: callers match on the kind where upstream catches a subclass.
//! `CredentialRead`/`CredentialWrite` are both "credential errors" (`is_credential`), and
//! `ClaudeCodeLockTimeout` is a lock error (`is_lock`).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Generic failure (upstream `ClaudeSwitchError`).
    #[error("{0}")]
    General(String),
    #[error("{0}")]
    Credential(String),
    #[error("{0}")]
    CredentialRead(String),
    #[error("{0}")]
    CredentialWrite(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Switch(String),
    #[error("{0}")]
    Session(String),
    #[error("{0}")]
    Lock(String),
    /// Timed out on one of Claude Code's own advisory locks. Nothing was mutated; safe to retry.
    #[error("{0}")]
    ClaudeCodeLockTimeout(String),
    #[error("{0}")]
    AccountNotFound(String),
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Transfer(String),
    #[error("{0}")]
    Migration(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn is_credential(&self) -> bool {
        matches!(
            self,
            Error::Credential(_) | Error::CredentialRead(_) | Error::CredentialWrite(_)
        )
    }

    pub fn is_lock(&self) -> bool {
        matches!(self, Error::Lock(_) | Error::ClaudeCodeLockTimeout(_))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
