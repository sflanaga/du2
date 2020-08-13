use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use anyhow::{Context, anyhow, Result};

#[cfg(target_os = "windows")]
pub fn gettid() -> usize {
    unsafe { winapi::um::processthreadsapi::GetCurrentThreadId() as usize }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn gettid() -> usize {
    unsafe { libc::syscall(libc::SYS_gettid) as usize }
}

struct ThreadStatusInner {
    name: String,
    state: String,
    tid: usize,
}

#[derive(Clone)]
pub struct ThreadStatus {
    status: Arc<Mutex<ThreadStatusInner>>,
}

impl ThreadStatus {
    fn new(state: &str, name: &str) -> ThreadStatus {
        ThreadStatus {
            status: Arc::new(Mutex::new(
                ThreadStatusInner {
                    name: name.to_string(),
                    state: state.into(),
                    tid: 0,
                }))
        }
    }
    pub fn set_state(&mut self, s: &str) {
        self.status.lock().unwrap().state = s.into();
    }
    pub fn register(&mut self, state: &str) {
        let mut l = self.status.lock().unwrap();
        l.state = state.into();
        l.tid = gettid();
    }
}

pub struct ThreadTracker {
    list: Vec<ThreadStatus>,
}

impl ThreadTracker {
    pub fn new() -> ThreadTracker {
        ThreadTracker {
            list: Vec::new(),
        }
    }
    pub fn setup_thread(&mut self, name: &str, state: &str) -> ThreadStatus {
        let ts = ThreadStatus::new(state, name);
        let cl = ts.clone();
        self.list.push(ts);
        cl
    }
    pub fn eprint_status(&self) {
        for ts in self.list.iter().enumerate() {
            let g_ts = ts.1.status.lock().unwrap();
            eprintln!("index: {:2} {:<10} tid: {:6}  status: \"{}\"", ts.0, &g_ts.name, &g_ts.tid, &g_ts.state);
        }
    }

    // once you start monitor you can no longer add/change it
    pub fn monitor(&self, interval_ms: u64) {
        loop {
            self.eprint_status();
            thread::sleep(Duration::from_millis(interval_ms));
        }
    }

    pub fn monitor_on_enter(&self) {
        loop {
            let mut buff = String::new();
            if std::io::stdin().read_line(&mut buff).is_err() {
                eprintln!("Unable to read line for user thread status cue");
                return;
            };
            self.eprint_status();
        }
    }
}

pub fn spawn_death_timeout_thread(die_dur: Duration, tt: &mut ThreadTracker) {
    let mut die_status = tt.setup_thread("die-in", "starting...");
    thread::spawn(move || {
        die_status.register("started");
        eprintln!("die thread on - die in {:.2} secs", die_dur.as_secs_f64());
        die_status.set_state(&format!("waiting to die after {:.2} secs", die_dur.as_secs_f64()));
        thread::sleep(die_dur);
        println!("*** SELF TERMINATION TIMEOUT ***");
        std::process::exit(1);
    });
}