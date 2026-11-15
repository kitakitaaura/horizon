//! A check of the patched std in a real browser: `smokeTest()` from the page's console or
//! `web/smoke.html`.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};
use wasm_bindgen::prelude::*;

/// Runs each check, or only those whose name contains `only`, and reports one line per check:
/// `ok` or what went wrong.
#[wasm_bindgen(js_name = smokeTest)]
pub fn smoke_test(only: Option<String>) -> String {
    let mut out = String::new();
    let mut check = |name: &str, f: &dyn Fn() -> Result<String, String>| {
        if only.as_deref().is_some_and(|only| !name.contains(only)) {
            return;
        }
        let started = Instant::now();
        let result = f();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let _ = match result {
            Ok(detail) => writeln!(out, "ok   {name} ({ms:.1} ms) {detail}"),
            Err(e) => writeln!(out, "FAIL {name} ({ms:.1} ms) {e}"),
        };
    };

    check("clock", &|| {
        let a = Instant::now();
        let wall = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        let b = Instant::now();
        if b < a {
            return Err("went back".into());
        }
        if wall.as_secs() < 1_700_000_000 {
            return Err(format!("wall clock {wall:?}"));
        }
        Ok(format!("unix {}", wall.as_secs()))
    });

    check("parallelism", &|| {
        std::thread::available_parallelism()
            .map(|n| n.to_string())
            .map_err(|e| e.to_string())
    });

    check("spawn+join", &|| {
        let handles: Vec<_> = (0..8u64)
            .map(|i| {
                std::thread::Builder::new()
                    .name(format!("smoke {i}"))
                    .spawn(move || {
                        let name = std::thread::current().name().map(str::to_owned);
                        (i * i, name)
                    })
            })
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let mut sum = 0;
        for (i, h) in handles.into_iter().enumerate() {
            let (v, name) = h.join().map_err(|_| "thread panicked")?;
            if name.as_deref() != Some(format!("smoke {i}").as_str()) {
                return Err(format!("thread {i} named {name:?}"));
            }
            sum += v;
        }
        if sum != 140 {
            return Err(format!("sum {sum}"));
        }
        Ok("8 threads".into())
    });

    check("mutex contention", &|| {
        let counter = Arc::new(Mutex::new(0u64));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let counter = counter.clone();
                std::thread::spawn(move || {
                    for _ in 0..2_000 {
                        *counter.lock().unwrap() += 1;
                    }
                })
            })
            .collect();
        // The main thread contends too: it spins rather than blocks.
        for _ in 0..2_000 {
            *counter.lock().unwrap() += 1;
        }
        for h in handles {
            h.join().map_err(|_| "panicked")?;
        }
        let n = *counter.lock().unwrap();
        if n != 10_000 {
            Err(format!("{n}"))
        } else {
            Ok("10000".into())
        }
    });

    check("channel+condvar", &|| {
        let (tx, rx) = mpsc::channel();
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let pair2 = pair.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            tx.send(42u32).unwrap();
            let (lock, cv) = &*pair2;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        });
        let v = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        let (lock, cv) = &*pair;
        let guard = cv
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |ready| {
                !*ready
            })
            .map_err(|_| "poisoned")?;
        if !*guard.0 {
            return Err("condvar timed out".into());
        }
        drop(guard);
        t.join().map_err(|_| "panicked")?;
        Ok(format!("got {v}"))
    });

    check("nested spawn", &|| {
        let total = Arc::new(AtomicU64::new(0));
        let t2 = total.clone();
        std::thread::spawn(move || {
            let inner: Vec<_> = (0..4)
                .map(|_| {
                    let t = t2.clone();
                    std::thread::spawn(move || t.fetch_add(1, Ordering::Relaxed))
                })
                .collect();
            for h in inner {
                h.join().unwrap();
            }
        })
        .join()
        .map_err(|_| "panicked")?;
        Ok(format!("{}", total.load(Ordering::Relaxed)))
    });

    check("burst from the main thread", &|| {
        // More threads than the pool keeps ready, joined before the main thread yields: the
        // spawner worker has to start the rest while this thread spins.
        let handles: Vec<_> = (0..96u64).map(|i| std::thread::spawn(move || i)).collect();
        let sum: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        if sum == 96 * 95 / 2 { Ok("96 threads".into()) } else { Err(format!("sum {sum}")) }
    });

    check("thread-locals fresh per thread", &|| {
        thread_local!(static N: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
        let firsts: Vec<u32> = (0..6)
            .map(|_| {
                std::thread::spawn(|| {
                    N.with(|n| {
                        let v = n.get();
                        n.set(v + 1);
                        v
                    })
                })
            })
            .map(|h| h.join().unwrap())
            .collect();
        if firsts.iter().all(|&v| v == 0) {
            Ok("".into())
        } else {
            Err(format!("{firsts:?}"))
        }
    });

    check("fs", &|| {
        let e = |e: std::io::Error| e.to_string();
        std::fs::create_dir_all("/smoke/a/b").map_err(e)?;
        std::fs::write("/smoke/a/b/hello.txt", b"hello").map_err(e)?;
        std::env::set_current_dir("/smoke/a").map_err(e)?;
        let text = std::fs::read_to_string("b/hello.txt").map_err(e)?;
        std::env::set_current_dir("/").map_err(e)?;
        if text != "hello" {
            return Err(text);
        }
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open("/smoke/a/b/hello.txt")
                .map_err(e)?;
            f.write_all(b" world").map_err(e)?;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open("/smoke/a/b/hello.txt")
                .map_err(e)?;
            f.seek(SeekFrom::Start(0)).map_err(e)?;
            f.write_all(b"J").map_err(e)?;
        }
        let text = std::fs::read_to_string("/smoke/a/b/hello.txt").map_err(e)?;
        if text != "Jello world" {
            return Err(text);
        }
        std::fs::rename("/smoke/a/b/hello.txt", "/smoke/hi.txt").map_err(e)?;
        let names: Vec<_> = std::fs::read_dir("/smoke")
            .map_err(e)?
            .map(|d| d.unwrap().file_name().into_string().unwrap())
            .collect();
        if names != ["a", "hi.txt"] {
            return Err(format!("{names:?}"));
        }
        // Another thread sees the same files.
        let seen = std::thread::spawn(|| std::fs::read("/smoke/hi.txt").map(|b| b.len()))
            .join()
            .map_err(|_| "panicked")?
            .map_err(e)?;
        std::fs::remove_dir_all("/smoke").map_err(e)?;
        if std::path::Path::new("/smoke").exists() {
            return Err("still there".into());
        }
        Ok(format!("{seen} bytes from a worker"))
    });

    check("env", &|| {
        // SAFETY: the patched std keeps variables in a locked map.
        unsafe { std::env::set_var("HORIZON_SMOKE", "1") };
        let seen = std::thread::spawn(|| std::env::var("HORIZON_SMOKE"))
            .join()
            .map_err(|_| "panicked")?;
        if seen.as_deref() == Ok("1") {
            Ok("".into())
        } else {
            Err(format!("{seen:?}"))
        }
    });

    check("sleep on main thread", &|| {
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(15));
        let ms = started.elapsed().as_millis();
        if ms >= 15 {
            Ok(format!("{ms} ms"))
        } else {
            Err(format!("{ms} ms"))
        }
    });

    out
}
