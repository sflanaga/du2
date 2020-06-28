#![allow(dead_code)]

use std::borrow::Cow;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

// Cow here let's us not allocate in the common case
pub fn multi_extension(p: & Path) -> Option<Cow<str>> {
    if let Some(filename) = p.to_str() {
        if filename.len() > 0 {
            let mut last_i = filename.len() - 1;
            for x in filename.chars().rev().zip((0..filename.len()).rev()) {
                // println!("i: {} {}  lasti: {}", x.0, x.1, last_i);

                if (last_i - x.1) > 4 {
                    break;
                } else if x.0 == '.' {
                    last_i = x.1;
                }
            }

            if last_i != filename.len() - 1 {
                return Some(Cow::Borrowed(&&filename[last_i..]));
            } else {
                return None;
            }
        }
    }
    None
}

fn mem_metric<'a>(v: usize) -> (f64, &'a str) {
    const METRIC: [&str; 8] = ["B ", "KB", "MB", "GB", "TB", "PB", "EB", "ZB"];

    let mut size = 1usize << 10;
    for e in &METRIC {
        if v < size {
            return ((v as f64 / (size >> 10) as f64) as f64, e);
        }
        size <<= 10;
    }
    (v as f64, "")
}

/// keep only a few significant digits of a simple float value
fn sig_dig(v: f64, digits: usize) -> String {
    let x = format!("{}", v);
    let mut d = String::new();
    let mut count = 0;
    let mut found_pt = false;
    for c in x.chars() {
        if c != '.' {
            count += 1;
        } else {
            if count >= digits {
                break;
            }
            found_pt = true;
        }

        d.push(c);

        if count >= digits && found_pt {
            break;
        }
    }
    d
}

pub fn mem_metric_digit(v: usize, sig: usize) -> String {
    if v == 0 || v > std::usize::MAX / 2 {
        return format!("{:>width$}", "unknown", width = sig + 3);
    }
    let vt = mem_metric(v);
    format!("{:>width$} {}", sig_dig(vt.0, sig), vt.1, width = sig + 1, )
}

const GREEK_SUFFIXES: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];

pub fn greek(v: f64) -> String {
    let mut number = v;
    let mut multi = 0;

    while number >= 1000.0 && multi < GREEK_SUFFIXES.len() - 1 {
        multi += 1;
        number /= 1024.0;
    }

    let mut s = format!("{}", number);
    s.truncate(4);
    if s.ends_with('.') {
        s.pop();
    }
    if s.len() < 4 { s.push(' '); }

    return format!("{:<5}{}", s, GREEK_SUFFIXES[multi]);
}

#[cfg(target_os = "windows")]
pub fn gettid() -> usize {
    unsafe { winapi::um::processthreadsapi::GetCurrentThreadId() as usize }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn gettid() -> usize {
    unsafe { libc::syscall(libc::SYS_gettid) as usize }
}
