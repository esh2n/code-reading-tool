//! Where an endpoint's API key comes from: nowhere, an environment
//! variable, or a command the user names (a password manager, the OS
//! keychain, anything that prints the key). The key never appears in
//! configuration, and no particular tool is assumed.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crt_app::ExplainError;

use crate::config::EndpointConfig;

/// How long a key command may take. Long enough for a person to answer a
/// password manager's prompt; a short limit makes slow unlocks fail.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// One endpoint's key, resolved at most once per process.
pub(crate) enum KeySource {
    None,
    Env(String),
    /// Run on the first request that needs it. The outcome, success or
    /// failure, is kept: a failing command is not run again on every read,
    /// which would raise a prompt each time.
    Command {
        argv: Vec<String>,
        resolved: Mutex<Option<Result<String, String>>>,
    },
}

impl KeySource {
    pub(crate) fn for_endpoint(e: &EndpointConfig) -> Result<Self, ExplainError> {
        match (&e.api_key_env, &e.api_key_command) {
            (Some(_), Some(_)) => Err(ExplainError::Config(format!(
                "{}: set either api_key_env or api_key_command, not both",
                e.base_url
            ))),
            (Some(var), None) => Ok(Self::Env(var.clone())),
            (None, Some(argv)) if argv.first().is_none_or(|p| p.trim().is_empty()) => {
                Err(ExplainError::Config(format!(
                    "{}: api_key_command must name a program, e.g. [\"program\", \"arg\"]",
                    e.base_url
                )))
            }
            (None, Some(argv)) => Ok(Self::Command {
                argv: argv.clone(),
                resolved: Mutex::new(None),
            }),
            (None, None) => Ok(Self::None),
        }
    }

    /// The key, `None` for keyless endpoints, or why it is unavailable.
    /// The reason never contains anything the command printed.
    pub(crate) fn key(&self) -> Result<Option<String>, String> {
        match self {
            Self::None => Ok(None),
            Self::Env(var) => match std::env::var(var) {
                Ok(key) if !key.is_empty() => Ok(Some(key)),
                _ => Err(format!(
                    "environment variable {var} (api_key_env) is not set"
                )),
            },
            Self::Command { argv, resolved } => {
                // Held while the command runs, so concurrent reads wait for
                // one prompt instead of raising several.
                let mut slot = resolved
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let outcome = slot.get_or_insert_with(|| {
                    // A key this process already got from the same command
                    // is reused: editing an unrelated setting rebuilds the
                    // sources and must not raise the prompt again.
                    if let Some(key) = lock(known_keys()).get(argv) {
                        return Ok(key.clone());
                    }
                    let got = run(argv, COMMAND_TIMEOUT);
                    if let Ok(key) = &got {
                        lock(known_keys()).insert(argv.clone(), key.clone());
                    }
                    got
                });
                outcome.clone().map(Some)
            }
        }
    }
}

/// Keys got from commands during this process, by command. Only successes:
/// a failure is retried once the configuration changes, so fixing the
/// cause (unlocking a vault) and saving the file is enough.
fn known_keys() -> &'static Mutex<HashMap<Vec<String>, String>> {
    static KEYS: OnceLock<Mutex<HashMap<Vec<String>, String>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The most a key command may print. A key is one line; more is a mistake.
const MAX_OUTPUT: u64 = 64 * 1024;

/// Runs `argv` and takes the first line of its standard output as the key.
pub(crate) fn run(argv: &[String], timeout: Duration) -> Result<String, String> {
    let program = &argv[0];
    let what = format!("api_key_command ({program})");
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Not shown anywhere: a credential tool's messages may include
        // secret material.
        .stderr(Stdio::null());
    // Its own process group, so a timeout stops whatever it started too
    // (a shell's children keep the output pipe open otherwise).
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("{what} could not be started: {e}"))?;
    let stdout = child.stdout.take().ok_or(format!("{what}: no output"))?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut out = String::new();
        let read = stdout
            .take(MAX_OUTPUT)
            .read_to_string(&mut out)
            .map(|_| out);
        let _ = tx.send(read);
    });
    let deadline = Instant::now() + timeout;
    let timed_out = |child: &mut Child| {
        stop(child);
        format!("{what} did not finish within {}s", timeout.as_secs())
    };
    let Ok(output) = rx.recv_timeout(timeout) else {
        return Err(timed_out(&mut child));
    };
    // The output may close before the command ends; the same deadline
    // covers the rest.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => return Err(timed_out(&mut child)),
            Err(e) => return Err(format!("{what} could not be waited on: {e}")),
        }
    };
    if !status.success() {
        return Err(format!("{what} failed ({status})"));
    }
    let output = output.map_err(|_| format!("{what} printed something that is not text"))?;
    match output.lines().next().map(str::trim) {
        Some(key) if !key.is_empty() => Ok(key.to_string()),
        _ => Err(format!("{what} printed nothing")),
    }
}

/// Stops the command and everything in its process group, and reaps it.
fn stop(child: &mut Child) {
    #[cfg(unix)]
    {
        // No safe signal API in std; the `kill` utility reaches the group.
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), script.into()]
    }

    #[test]
    fn the_first_line_is_the_key() {
        assert_eq!(
            run(&sh("printf 'k3y\\nmore'"), COMMAND_TIMEOUT).unwrap(),
            "k3y"
        );
    }

    #[test]
    fn failures_never_echo_what_the_command_printed() {
        let err = run(&sh("echo SECRET; echo SECRET >&2; exit 3"), COMMAND_TIMEOUT).unwrap_err();
        assert!(err.contains("failed"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
        assert!(
            run(&sh("true"), COMMAND_TIMEOUT)
                .unwrap_err()
                .contains("nothing")
        );
        assert!(
            run(&["/nonexistent/crt-key".into()], COMMAND_TIMEOUT)
                .unwrap_err()
                .contains("could not be started")
        );
    }

    #[test]
    fn a_command_that_hangs_is_stopped() {
        let err = run(&sh("sleep 30"), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
    }

    #[test]
    fn a_timeout_stops_what_the_command_started_too() {
        let started = Instant::now();
        // The shell's child keeps the output open; it must be stopped too.
        let err = run(&sh("sleep 30 & sleep 30"), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // Output closed early, process still running: the deadline holds.
        let err = run(&sh("exec >&-; sleep 30"), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
    }

    #[test]
    fn a_key_is_reused_by_a_new_source_for_the_same_command() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("count");
        let script = format!("echo x >> '{}'; printf reused", count.display());
        for _ in 0..2 {
            let source = KeySource::Command {
                argv: sh(&script),
                resolved: Mutex::new(None),
            };
            assert_eq!(source.key().unwrap().as_deref(), Some("reused"));
        }
        let runs = std::fs::read_to_string(&count).unwrap().lines().count();
        assert_eq!(runs, 1);
    }

    #[test]
    fn a_command_runs_once_per_process_even_when_it_fails() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("count");
        let script = format!("echo x >> '{}'; exit 1", count.display());
        let source = KeySource::Command {
            argv: sh(&script),
            resolved: Mutex::new(None),
        };
        assert!(source.key().is_err());
        assert!(source.key().is_err());
        let runs = std::fs::read_to_string(&count).unwrap().lines().count();
        assert_eq!(runs, 1);
    }
}
