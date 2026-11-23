//! Java in the browser, where there are no processes: a JVM is Ristretto's wasm32-wasip1 build in
//! a worker of its own (web/jvm/worker.js), which the page's JVM spawner (web/jvm/spawner.js)
//! starts when this module asks through [`MAILBOX`].
//!
//! The thread that started a JVM and its worker share a [`Channel`] in this module's shared
//! memory. Everything the JVM does outside its own memory comes here as a request: each file
//! operation, its output, its input, its exit. This thread answers from the game's filesystem
//! while it waits for the JVM, so the JVM sees the same files as everything else, with no copies
//! either way: its `/` is this filesystem; `/jdk` is the Java image the page provides.
//!
//! A request: the worker writes `OP`, its arguments and `data`, then sets `STATE` to `REQUEST` and
//! waits; this thread answers in the same words and `data` and sets `STATE` to `REPLY`; the worker
//! reads the answer and sets `STATE` back to `IDLE`. Paths in `data` are relative to `/`, after a
//! little-endian u32 length; what a write carries follows them.

use std::cell::UnsafeCell;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::SeqCst};
use std::time::{Duration, Instant};

/// The most `data` one request or answer carries; the worker splits larger reads and writes.
const CAPACITY: usize = 1 << 20;

/// How long the page's JVM spawner has to take a new JVM from [`MAILBOX`]. It takes one at once
/// when it runs; when it doesn't (its runtime's .wasm failed to load), nothing else would ever
/// answer, and the step would wait out its whole timeout.
const PICKUP_TIMEOUT: Duration = Duration::from_secs(60);

// A channel's words.
const STATE: usize = 0;
const OP: usize = 1;
/// A request's first argument; an answer's errno (0: success) or count.
const A: usize = 2;
const B: usize = 3;
/// Bytes in `data`.
const LEN: usize = 4;
const OFFSET_LO: usize = 5;
const OFFSET_HI: usize = 6;
/// `KILL` (set here: stop the JVM) and `GONE` (set by the spawner: its worker stopped).
const FLAGS: usize = 7;
/// The start request's length at the start of `data` (JSON: `{"args": [...], "env": {...}}`).
const SPEC: usize = 8;

/// Set by the worker once it has read an answer.
#[allow(dead_code)]
const IDLE: i32 = 0;
const REQUEST: i32 = 1;
const REPLY: i32 = 2;

const KILL: i32 = 1;
const GONE: i32 = 2;

// Requests (web/jvm/rpc.js sends the same numbers).
const STAT: i32 = 1;
const LIST: i32 = 2;
const READ: i32 = 3;
const WRITE: i32 = 4;
const TRUNCATE: i32 = 5;
const CREATE: i32 = 6;
const MKDIR: i32 = 7;
const UNLINK: i32 = 8;
const RMDIR: i32 = 9;
const RENAME: i32 = 10;
const STDOUT: i32 = 11;
const STDERR: i32 = 12;
const STDIN: i32 = 13;
const EXIT: i32 = 14;

// WASI's error numbers, which the worker returns to the JVM as they are.
const EEXIST: i32 = 20;
const EINVAL: i32 = 28;
const EIO: i32 = 29;
const EISDIR: i32 = 31;
const ENOENT: i32 = 44;
const ENOTDIR: i32 = 54;
const ENOTEMPTY: i32 = 55;

#[repr(C)]
struct Channel {
    words: [AtomicI32; 16],
    data: UnsafeCell<[u8; CAPACITY]>,
}

// SAFETY: `data` is written by one side at a time, as `STATE` hands it over.
unsafe impl Sync for Channel {}

impl Channel {
    fn new() -> Box<Self> {
        let layout = std::alloc::Layout::new::<Self>();
        // SAFETY: zeroed atomics and bytes are valid; the box frees it with the same layout.
        unsafe {
            let raw = std::alloc::alloc_zeroed(layout).cast::<Self>();
            if raw.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            Box::from_raw(raw)
        }
    }

    fn word(&self, index: usize) -> i32 {
        self.words[index].load(SeqCst)
    }

    fn set(&self, index: usize, value: i32) {
        self.words[index].store(value, SeqCst);
    }

    /// Waits while word `index` holds `expected`, at most `timeout`.
    fn wait(&self, index: usize, expected: i32, timeout: Duration) {
        let nanos = i64::try_from(timeout.as_nanos()).unwrap_or(i64::MAX);
        // SAFETY: the word lives as long as the channel; a pool thread may block.
        unsafe {
            core::arch::wasm32::memory_atomic_wait32(self.words[index].as_ptr(), expected, nanos);
        }
    }

    fn notify(&self, index: usize) {
        // SAFETY: as in `wait`.
        unsafe {
            core::arch::wasm32::memory_atomic_notify(self.words[index].as_ptr(), u32::MAX);
        }
    }

    /// The first `len` bytes of `data`.
    ///
    /// # Safety
    /// Only while this side holds `data` (between `REQUEST` and `REPLY`, or before the start).
    unsafe fn data(&self, len: usize) -> &[u8] {
        unsafe { &(&*self.data.get())[..len.min(CAPACITY)] }
    }

    /// # Safety
    /// As [`Self::data`].
    #[allow(clippy::mut_from_ref)]
    unsafe fn data_mut(&self) -> &mut [u8; CAPACITY] {
        unsafe { &mut *self.data.get() }
    }
}

/// Start requests for the page's JVM spawner: a channel's address in a free slot, then `count`
/// goes up, which the spawner waits on. A raised `KILL` flag also counts up, so the spawner looks.
#[repr(C)]
pub struct Mailbox {
    count: AtomicI32,
    slots: [AtomicU32; 32],
}

pub static MAILBOX: Mailbox = Mailbox {
    count: AtomicI32::new(0),
    slots: [const { AtomicU32::new(0) }; 32],
};

/// Where [`MAILBOX`] is in memory, for the page (`horizon.jvmMailboxAddr()`).
pub fn mailbox_addr() -> usize {
    std::ptr::from_ref(&MAILBOX) as usize
}

fn ring() {
    MAILBOX.count.fetch_add(1, SeqCst);
    // SAFETY: a static's address.
    unsafe {
        core::arch::wasm32::memory_atomic_notify(MAILBOX.count.as_ptr(), u32::MAX);
    }
}

/// `java <args>` for Ristretto, from the options prepare gives HotSpot: its tuning options have no
/// meaning there, and its classpath option is `--cp`.
pub fn java_args(args: &[String]) -> Vec<String> {
    let mut out = vec!["java".to_string()];
    for arg in args {
        if crate::java::BATCH_OPTIONS.contains(&arg.as_str())
            || ["-Xmx", "-Xms", "-Xss", "-Xverify", "-XX:"]
                .iter()
                .any(|p| arg.starts_with(p))
        {
            continue;
        }
        out.push(if arg == "-cp" || arg == "-classpath" {
            "--cp".into()
        } else {
            arg.clone()
        });
    }
    out
}

/// `java <args>` (HotSpot's options, as prepare gives them) to its end, standard output and error
/// into `log`.
pub fn run(args: &[String], log: &fs::File, timeout: Duration) -> Result<(), String> {
    let log = log.try_clone().map_err(|e| e.to_string())?;
    let mut jvm = Jvm::spawn(&java_args(args), log, false)?;
    let code = jvm.wait(timeout)?;
    // Its standard output belongs in the log too, as with HotSpot.
    let _ = jvm.log.write_all(&jvm.stdout);
    match code {
        0 => Ok(()),
        code => Err(format!("java failed (exit {code})")),
    }
}

/// What [`Jvm::serve`] saw.
enum Event {
    Served,
    /// The JVM asked for input and there is none yet: its request waits.
    NeedsInput,
    Exited,
}

/// A running JVM.
pub struct Jvm {
    channel: Option<Box<Channel>>,
    /// The [`MAILBOX`] slot holding the channel and when it was put there, until the spawner
    /// takes it.
    waiting: Option<(usize, Instant)>,
    /// Why it never started ([`Self::check_taken`]), for every later wait on it: the exporter
    /// JVMs serve several steps, and each should say why.
    never_started: Option<String>,
    /// The step it served last ([`Step`]): a request for another means it started before.
    step: Option<String>,
    stdin: Vec<u8>,
    /// No more input: a read gets the end of input.
    stdin_closed: bool,
    stdout: Vec<u8>,
    /// Where the JVM's standard error goes.
    log: fs::File,
    exit: Option<i32>,
    /// The file the last read or write used, kept open for the next, and whether it can write.
    open: Option<(PathBuf, fs::File, bool)>,
}

impl Jvm {
    /// Starts `java` with `args` (Ristretto's, see [`java_args`]). With `interactive`, its input
    /// is what [`Jvm::send_line`] sends; otherwise it has none.
    pub fn spawn(args: &[String], log: fs::File, interactive: bool) -> Result<Self, String> {
        let channel = Channel::new();
        let quote = |s: &str| {
            crate::pyjson::dumps(
                &crate::pyjson::Value::Str(s.into()),
                &crate::pyjson::COMPACT,
            )
        };
        let spec = format!(
            "{{\"args\":[{}],\"env\":{{\"JAVA_HOME\":\"/jdk\"}}}}",
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(",")
        );
        if spec.len() > CAPACITY {
            return Err("The Java command line is too long".into());
        }
        // SAFETY: nobody else knows the channel yet.
        unsafe { channel.data_mut()[..spec.len()].copy_from_slice(spec.as_bytes()) };
        channel.set(SPEC, spec.len() as i32);
        let address = std::ptr::from_ref::<Channel>(&channel) as u32;
        let slot = MAILBOX
            .slots
            .iter()
            .position(|slot| slot.compare_exchange(0, address, SeqCst, SeqCst).is_ok());
        let Some(slot) = slot else {
            return Err("Too many Java runtimes at once".into());
        };
        ring();
        Ok(Self {
            channel: Some(channel),
            waiting: Some((slot, Instant::now())),
            never_started: None,
            step: None,
            stdin: Vec::new(),
            stdin_closed: !interactive,
            stdout: Vec::new(),
            log,
            exit: None,
            open: None,
        })
    }

    fn channel(&self) -> &Channel {
        self.channel.as_deref().unwrap()
    }

    /// Queues `line` for the JVM's standard input.
    pub fn send_line(&mut self, line: &str) {
        self.stdin.extend_from_slice(line.as_bytes());
        self.stdin.push(b'\n');
    }

    /// The JVM's next line of output, serving its requests until it comes.
    pub fn read_line(&mut self, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(end) = self.stdout.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.stdout.drain(..=end).collect();
                return Ok(String::from_utf8_lossy(&line[..end])
                    .trim_end_matches('\r')
                    .into());
            }
            if let Some(why) = &self.never_started {
                return Err(why.clone());
            }
            if let Some(code) = self.exit {
                return Err(format!("Java stopped (exit {code})"));
            }
            if let Event::NeedsInput = self.serve(deadline)? {
                return Err("Java waits for input instead of answering".into());
            }
        }
    }

    /// Serves the JVM until it exits; its exit status.
    pub fn wait(&mut self, timeout: Duration) -> Result<i32, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(why) = &self.never_started {
                return Err(why.clone());
            }
            if let Some(code) = self.exit {
                return Ok(code);
            }
            if let Event::NeedsInput = self.serve(deadline)? {
                return Err("Java waits for input that never comes".into());
            }
        }
    }

    /// Answers the JVM's next request.
    fn serve(&mut self, deadline: Instant) -> Result<Event, String> {
        loop {
            self.check_taken()?;
            let channel = self.channel();
            if channel.word(FLAGS) & GONE != 0 && channel.word(STATE) != REQUEST {
                self.exit.get_or_insert(-1);
                return Ok(Event::Exited);
            }
            let state = channel.word(STATE);
            if state == REQUEST {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("Java took over an hour and was stopped".into());
            }
            channel.wait(
                STATE,
                state,
                (deadline - now).min(Duration::from_millis(250)),
            );
        }
        let channel = self.channel.take().unwrap();
        let event = self.answer(&channel);
        self.channel = Some(channel);
        Ok(event)
    }

    /// An error once the spawner has left this JVM in [`MAILBOX`] for [`PICKUP_TIMEOUT`], taken
    /// back out so a spawner that starts later doesn't run it.
    fn check_taken(&mut self) -> Result<(), String> {
        let Some((slot, since)) = self.waiting else {
            return Ok(());
        };
        let address = std::ptr::from_ref::<Channel>(self.channel()) as u32;
        let slot = &MAILBOX.slots[slot];
        if slot.load(SeqCst) != address {
            self.waiting = None;
            return Ok(());
        }
        if since.elapsed() < PICKUP_TIMEOUT {
            return Ok(());
        }
        if slot.compare_exchange(address, 0, SeqCst, SeqCst).is_err() {
            // Taken just now.
            self.waiting = None;
            return Ok(());
        }
        self.channel().words[FLAGS].fetch_or(GONE, SeqCst);
        self.exit = Some(-1);
        let why = format!(
            "No Java runtime started in {} s: the page's Java spawner isn't running (did its \
             runtime's .wasm load? The page's ?java= names it; the console says why)",
            PICKUP_TIMEOUT.as_secs()
        );
        self.never_started = Some(why.clone());
        Err(why)
    }

    fn answer(&mut self, channel: &Channel) -> Event {
        let op = channel.word(OP);
        let len = channel.word(LEN) as usize;
        let offset = (channel.word(OFFSET_LO) as u32 as u64)
            | ((channel.word(OFFSET_HI) as u32 as u64) << 32);
        // SAFETY: the worker handed `data` over with `REQUEST`.
        let data = unsafe { channel.data(len) }.to_vec();
        let (path, payload) = split_path(&data);
        let step = STEP.with(|s| s.borrow().clone());
        let warm = self.step.is_some() && self.step != step;
        if step.is_some() {
            self.step = step;
        }
        note(
            op,
            &path,
            if op == READ {
                channel.word(B) as usize
            } else {
                payload.len()
            },
            warm,
        );
        let reply = |errno: i32, count: i32, bytes: &[u8]| {
            // SAFETY: this side holds `data` until `REPLY`.
            unsafe { channel.data_mut()[..bytes.len()].copy_from_slice(bytes) };
            channel.set(A, errno);
            channel.set(B, count);
            channel.set(LEN, bytes.len() as i32);
            channel.set(STATE, REPLY);
            channel.notify(STATE);
        };
        let done = |result: std::io::Result<()>| match result {
            Ok(()) => reply(0, 0, &[]),
            Err(e) => reply(errno(&e), 0, &[]),
        };
        match op {
            STAT => match fs::metadata(&path) {
                Ok(meta) => {
                    let mut out = [0u8; 24];
                    out[0] = if meta.is_dir() { 1 } else { 2 };
                    out[8..16].copy_from_slice(&(meta.len() as f64).to_le_bytes());
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
                    out[16..24].copy_from_slice(&mtime.to_le_bytes());
                    reply(0, 0, &out);
                }
                Err(e) => reply(errno(&e), 0, &[]),
            },
            LIST => match fs::read_dir(&path) {
                Ok(entries) => {
                    let names: Vec<String> = entries
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect();
                    let text = names.join("\n");
                    if text.len() > CAPACITY {
                        reply(EIO, 0, &[]);
                    } else {
                        reply(0, names.len() as i32, text.as_bytes());
                    }
                }
                Err(e) => reply(errno(&e), 0, &[]),
            },
            READ => {
                let want = (channel.word(B) as usize).min(CAPACITY);
                let mut buffer = vec![0u8; want];
                match self.file(&path, false).and_then(|f| {
                    f.seek(SeekFrom::Start(offset))?;
                    let mut n = 0;
                    while n < want {
                        let got = f.read(&mut buffer[n..])?;
                        if got == 0 {
                            break;
                        }
                        n += got;
                    }
                    Ok(n)
                }) {
                    Ok(n) => reply(0, n as i32, &buffer[..n]),
                    Err(e) => reply(errno(&e), 0, &[]),
                }
            }
            WRITE => match self.file(&path, true).and_then(|f| {
                f.seek(SeekFrom::Start(offset))?;
                f.write_all(payload)
            }) {
                Ok(()) => reply(0, payload.len() as i32, &[]),
                Err(e) => reply(errno(&e), 0, &[]),
            },
            TRUNCATE => done(self.file(&path, true).and_then(|f| f.set_len(offset))),
            CREATE => done(
                fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&path)
                    .map(drop),
            ),
            MKDIR => done(fs::create_dir(&path)),
            UNLINK => {
                self.open = None;
                done(fs::remove_file(&path))
            }
            RMDIR => done(fs::remove_dir(&path)),
            RENAME => {
                self.open = None;
                let (to, _) = split_path(payload);
                done(fs::rename(&path, to))
            }
            STDOUT => {
                self.stdout.extend_from_slice(payload);
                reply(0, payload.len() as i32, &[]);
            }
            STDERR => {
                let _ = self.log.write_all(payload);
                reply(0, payload.len() as i32, &[]);
            }
            STDIN => {
                if self.stdin.is_empty() && !self.stdin_closed {
                    // Answered by the next `serve` after `send_line`.
                    return Event::NeedsInput;
                }
                let n = (channel.word(B) as usize)
                    .min(self.stdin.len())
                    .min(CAPACITY);
                let bytes: Vec<u8> = self.stdin.drain(..n).collect();
                reply(0, n as i32, &bytes);
            }
            EXIT => {
                self.exit = Some(channel.word(A));
                reply(0, 0, &[]);
                return Event::Exited;
            }
            _ => reply(EINVAL, 0, &[]),
        }
        Event::Served
    }

    /// The open file at `path`: the last request's, unless it was another file or opened for
    /// reading only and this one writes.
    fn file(&mut self, path: &Path, write: bool) -> std::io::Result<&mut fs::File> {
        let reuse = matches!(&self.open, Some((open, _, writable)) if open == path && (*writable || !write));
        if !reuse {
            let file = fs::OpenOptions::new().read(true).write(write).open(path)?;
            self.open = Some((path.to_path_buf(), file, write));
        }
        Ok(&mut self.open.as_mut().unwrap().1)
    }
}

impl Drop for Jvm {
    fn drop(&mut self) {
        let Some(channel) = self.channel.take() else {
            return;
        };
        if channel.word(FLAGS) & GONE == 0 {
            channel.words[FLAGS].fetch_or(KILL, SeqCst);
            ring();
        }
        // The channel is never freed (1 MiB per JVM, a dozen per preparation). `GONE` only means
        // the spawner asked for the worker to stop (Worker.terminate), not that it has: a worker
        // waiting for input that wakes after the memory went to a new JVM's channel would take
        // that JVM's answers as its own.
        Box::leak(channel);
    }
}

/// What a step's JVM did so far, from the requests it made: something to show the player while
/// a step runs for minutes, as the exporters print almost nothing.
#[derive(Default)]
struct Activity {
    requests: u64,
    /// Bytes read from jars: Minecraft's code and data, as the JVM loads it; and when last.
    jar_bytes: u64,
    jar_read: Option<Instant>,
    /// Its JVM had served an earlier step: started, with Minecraft loaded.
    warm: bool,
    /// Files the step created, and the last one's name.
    written: u64,
    last_written: String,
}

impl Activity {
    fn describe(&self) -> String {
        let mib = self.jar_bytes as f64 / f64::from(1 << 20);
        let loading = self
            .jar_read
            .is_some_and(|at| at.elapsed() < Duration::from_secs(3));
        if self.written > 0 {
            format!(
                "writing {}, {} file{} so far",
                self.last_written,
                self.written,
                if self.written == 1 { "" } else { "s" }
            )
        } else if loading {
            format!("loading Minecraft, {mib:.0} MB")
        } else if mib >= 1.0 {
            format!("working, {mib:.0} MB of Minecraft loaded")
        } else if self.warm || self.jar_bytes > 0 {
            "working".into()
        } else if self.requests > 0 {
            "starting Java".into()
        } else {
            // Its exporter JVM is busy with another step, or not started yet.
            "waiting for a free Java runtime".into()
        }
    }
}

/// The running steps' [`Activity`], by step.
static ACTIVITY: std::sync::Mutex<Vec<(String, Activity)>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    /// The step this thread runs (its JVM's requests are served on it): see [`Step`].
    static STEP: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn activity<T>(f: impl FnOnce(&mut Vec<(String, Activity)>) -> T) -> T {
    f(&mut ACTIVITY.lock().unwrap_or_else(|e| e.into_inner()))
}

/// While it lives, the JVM requests this thread serves count for step `what`'s [`activities`].
pub struct Step(String);

impl Step {
    pub fn enter(what: &str) -> Self {
        STEP.with(|s| *s.borrow_mut() = Some(what.to_owned()));
        activity(|a| a.push((what.to_owned(), Activity::default())));
        Self(what.to_owned())
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        STEP.with(|s| *s.borrow_mut() = None);
        activity(|a| a.retain(|(what, _)| *what != self.0));
    }
}

/// What each running step's JVM is doing, in words: `(step, what it does)`.
pub fn activities() -> Vec<(String, String)> {
    activity(|a| {
        a.iter()
            .map(|(what, act)| (what.clone(), act.describe()))
            .collect()
    })
}

/// Counts request `op` on `path` (`count` bytes) for this thread's step; `warm`: the JVM making
/// it had served a step before.
fn note(op: i32, path: &Path, count: usize, warm: bool) {
    let Some(step) = STEP.with(|s| s.borrow().clone()) else {
        return;
    };
    activity(|a| {
        let Some((_, act)) = a.iter_mut().find(|(what, _)| *what == step) else {
            return;
        };
        act.requests += 1;
        act.warm |= warm;
        match op {
            READ if path.extension().is_some_and(|e| e == "jar") => {
                act.jar_bytes += count as u64;
                act.jar_read = Some(Instant::now());
            }
            // Vanilla's own log files aren't the step's output.
            CREATE
                if !path
                    .extension()
                    .is_some_and(|e| e == "log" || e == "lock" || e == "gz") =>
            {
                act.written += 1;
                act.last_written = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            _ => {}
        }
    });
}

/// A request's path (absolute, in this filesystem) and what follows it.
fn split_path(data: &[u8]) -> (PathBuf, &[u8]) {
    if data.len() < 4 {
        return (PathBuf::from("/"), &[]);
    }
    let len = (u32::from_le_bytes(data[..4].try_into().unwrap()) as usize).min(data.len() - 4);
    let path = String::from_utf8_lossy(&data[4..4 + len]);
    (Path::new("/").join(path.as_ref()), &data[4 + len..])
}

fn errno(error: &std::io::Error) -> i32 {
    match error.kind() {
        ErrorKind::NotFound => ENOENT,
        ErrorKind::AlreadyExists => EEXIST,
        ErrorKind::NotADirectory => ENOTDIR,
        ErrorKind::IsADirectory => EISDIR,
        ErrorKind::DirectoryNotEmpty => ENOTEMPTY,
        ErrorKind::InvalidInput => EINVAL,
        _ => EIO,
    }
}
