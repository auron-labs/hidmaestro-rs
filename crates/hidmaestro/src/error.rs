use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("bridge executable not found: {0}")]
    BridgeNotFound(String),

    #[error("failed to spawn bridge process `{program}`: {source}")]
    Spawn { program: String, source: io::Error },

    #[error("invalid HIDMaestro pipe name `{name}`: {reason}")]
    InvalidPipeName { name: String, reason: &'static str },

    #[error("named-pipe bridge mode is only supported on Windows; remove HIDMAESTRO_PIPE_NAME or use stdio bridge spawning")]
    PipeUnsupported,

    #[error("failed to connect to HIDMaestro named pipe `{name}`: {source}")]
    PipeConnect { name: String, source: io::Error },

    #[error("bridge I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("bridge returned malformed JSON: {0}")]
    Protocol(#[from] serde_json::Error),

    #[error("bridge error: {0}")]
    Remote(String),

    #[error("bridge exited unexpectedly{0}")]
    BridgeExited(String),

    #[error("unknown button name: {0}")]
    UnknownButton(String),

    #[error("unknown axis name: {0}")]
    UnknownAxis(String),

    #[error("unknown hat direction: {0}")]
    UnknownHat(String),

    #[error("no such controller: {0}")]
    NoSuchController(String),

    #[error("timeout waiting for bridge response")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, Error>;
