//! A bounded request/reply channel to one initialized exporter JVM: a `java` process, or in the
//! browser the page's JVM (browser_jvm.rs), with the same task lines on its input.
use crate::*;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, ChildStdin, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver};

const TASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

/// A task's line: `[log, main, [args...]]`.
fn task(main: &str, args: &[String], log: &Path) -> String {
    let task = Value::Array(vec![
        Value::Str(arg(log)),
        Value::Str(main.into()),
        Value::Array(args.iter().cloned().map(Value::Str).collect()),
    ]);
    pyjson::dumps(&task, &pyjson::COMPACT)
}

/// The worker's answer to a task: an empty string, or what failed.
fn outcome(main: &str, reply: &str) -> Result<(), String> {
    match pyjson::loads(reply)?.as_str() {
        Some("") => Ok(()),
        _ => Err(format!("Exporter {main} failed: {reply}")),
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct Worker {
    jvm: crate::browser_jvm::Jvm,
}

#[cfg(target_arch = "wasm32")]
impl Worker {
    pub(crate) fn new(run: &Run, name: &str, classes: &Path, cp: &str) -> Result<Self, String> {
        let log = run.log_file(&run.local.join(format!("{name}-worker.log")))?;
        let args = crate::browser_jvm::java_args(&[
            "-cp".into(),
            format!("{}:{cp}", classes.display()),
            "dev.horizonlab.parity.PrepareWorker".into(),
        ]);
        Ok(Self {
            jvm: crate::browser_jvm::Jvm::spawn(&args, log, true)?,
        })
    }

    pub(crate) fn run(&mut self, main: &str, args: &[String], log: &Path) -> Result<(), String> {
        self.jvm.send_line(&task(main, args, log));
        let reply = self
            .jvm
            .read_line(TASK_TIMEOUT)
            .map_err(|e| format!("Exporter JVM stopped or timed out: {e}"))?;
        outcome(main, &reply)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Worker {
    child: Child,
    input: ChildStdin,
    replies: Receiver<std::io::Result<String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    pub(crate) fn new(run: &Run, name: &str, classes: &Path, cp: &str) -> Result<Self, String> {
        let log = run.log_file(&run.local.join(format!("{name}-worker.log")))?;
        let mut child = java::command(&run.jdk.java)
            .args(java::BATCH_OPTIONS)
            .args([
                "-Xmx2G",
                "-cp",
                &format!(
                    "{}{}{}",
                    classes.display(),
                    if cfg!(windows) { ";" } else { ":" },
                    cp
                ),
                "dev.horizonlab.parity.PrepareWorker",
            ])
            // Vanilla's code logs relative to it (logs/): in the scratch, as `Run::run`.
            .current_dir(&run.local)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
        })
    }

    pub(crate) fn run(&mut self, main: &str, args: &[String], log: &Path) -> Result<(), String> {
        writeln!(self.input, "{}", task(main, args, log)).map_err(|e| e.to_string())?;
        self.input.flush().map_err(|e| e.to_string())?;
        let reply = self
            .replies
            .recv_timeout(TASK_TIMEOUT)
            .map_err(|e| format!("Exporter JVM stopped or timed out: {e}"))?
            .map_err(|e| e.to_string())?;
        outcome(main, &reply)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
