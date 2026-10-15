//! Child processes for apps: a launch description that is checked before anything runs, and the
//! termination status and captured output of a child that has finished. The program is named by an absolute path and
//! its arguments and environment are checked for bytes that cannot cross the process boundary. Sandboxing and cgroup
//! placement belong to the shell, which launches apps; this module runs only what the app asks for.
//!
//! The child inherits the app's environment, and [`ProcessSpec::env`] adds or replaces one variable.

use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

/// Why a launch was refused or could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    /// The program path is empty or relative.
    NotAbsolute,
    /// An argument, an environment name or value, or the program path holds a NUL byte.
    NulByte,
    /// An environment variable name is empty or holds `=`.
    InvalidEnvName,
    /// The program could not be started. The kind is the operating system's.
    Launch(std::io::ErrorKind),
    /// The child finished without a status the system reports.
    NoStatus,
}

/// How a finished child ended: by an exit code, or by a signal that stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Termination {
    Exited(i32),
    Signaled(i32),
}

impl Termination {
    /// The termination of a finished child. None when the status carries neither a code nor a signal.
    pub fn from_status(status: ExitStatus) -> Option<Self> {
        match (status.code(), status.signal()) {
            (Some(code), _) => Some(Termination::Exited(code)),
            (None, Some(signal)) => Some(Termination::Signaled(signal)),
            (None, None) => None,
        }
    }

    /// True only for an exit with code zero.
    pub fn succeeded(self) -> bool {
        self == Termination::Exited(0)
    }
}

/// A child that has finished: its termination and the bytes it wrote to each output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub termination: Termination,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A checked launch description: an absolute program, its arguments and the variables it adds to the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSpec {
    program: PathBuf,
    arguments: Vec<String>,
    environment: Vec<(String, String)>,
}

fn has_nul(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

impl ProcessSpec {
    /// A launch of `program`, which must be an absolute path without NUL bytes.
    pub fn new(program: impl Into<PathBuf>) -> Result<Self, SpawnError> {
        let program = program.into();
        if !program.is_absolute() {
            return Err(SpawnError::NotAbsolute);
        }
        if has_nul(program.as_os_str().as_encoded_bytes()) {
            return Err(SpawnError::NulByte);
        }
        Ok(ProcessSpec { program, arguments: Vec::new(), environment: Vec::new() })
    }

    /// Appends one argument. An argument with a NUL byte is refused.
    pub fn argument(mut self, argument: &str) -> Result<Self, SpawnError> {
        if has_nul(argument.as_bytes()) {
            return Err(SpawnError::NulByte);
        }
        self.arguments.push(argument.to_string());
        Ok(self)
    }

    /// Sets one environment variable for the child, replacing an inherited one with the same name. The name must not be
    /// empty and must not hold `=`; neither the name nor the value may hold a NUL byte.
    pub fn env(mut self, name: &str, value: &str) -> Result<Self, SpawnError> {
        if name.is_empty() || name.contains('=') {
            return Err(SpawnError::InvalidEnvName);
        }
        if has_nul(name.as_bytes()) || has_nul(value.as_bytes()) {
            return Err(SpawnError::NulByte);
        }
        self.environment.retain(|(key, _)| key != name);
        self.environment.push((name.to_string(), value.to_string()));
        Ok(self)
    }

    pub fn program(&self) -> &PathBuf {
        &self.program
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// Starts the child with no stdin, waits for it to finish and returns its termination and both outputs.
    pub fn run(&self) -> Result<Finished, SpawnError> {
        let output = Command::new(&self.program)
            .args(&self.arguments)
            .envs(self.environment.iter().map(|(name, value)| (name.as_str(), value.as_str())))
            .output()
            .map_err(|error| SpawnError::Launch(error.kind()))?;
        let termination = Termination::from_status(output.status).ok_or(SpawnError::NoStatus)?;
        Ok(Finished { termination, stdout: output.stdout, stderr: output.stderr })
    }
}
