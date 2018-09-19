use std::time::Duration;

const GREEK_SUFFIXES: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];

#[allow(dead_code)]
pub fn greekshort_i(v: u64) -> String {

    let mut number = v;
    let mut multi = 0;

    while number >= 1000 && multi < GREEK_SUFFIXES.len()-1 {
        multi += 1;
        number /= 1024u64;
    }

    let mut s = format!("{}", number);
    s.truncate(4);
    if s.ends_with('.') {
        s.pop();
    }
    if s.len() < 4 { s.push(' ' ); }

    return format!("{}{}", s, GREEK_SUFFIXES[multi]);
}

pub fn greek(v: f64) -> String {

    let mut number = v;
    let mut multi = 0;

    while number >= 1000.0 && multi < GREEK_SUFFIXES.len()-1 {
        multi += 1;
        number /= 1024.0;
    }

    let mut s = format!("{}", number);
    s.truncate(4);
    if s.ends_with('.') {
        s.pop();
    }
    if s.len() < 4 { s.push(' ' ); }

    return format!("{}{}", s, GREEK_SUFFIXES[multi]);
}

pub fn dur_from_str(s: &str) -> Duration {
    let mut _tmp = String::new();
    let mut tot_secs = 0u64;
    for c in s.chars() {
        if c >= '0' && c <= '9' { _tmp.push(c); }
        else {
            tot_secs += match c {
                's' => _tmp.parse::<u64>().unwrap(),
                'm' => _tmp.parse::<u64>().unwrap() * 60,
                'h' => _tmp.parse::<u64>().unwrap() * 3600,
                'd' => _tmp.parse::<u64>().unwrap() * 24 * 3600,
                'w' => _tmp.parse::<u64>().unwrap() * 24 * 3600 * 7,
                'y' => _tmp.parse::<u64>().unwrap() * 24 * 3600 * 365,
                _ => panic!("char {} not understood", c),
            };
            _tmp.clear();
        }
    }
    Duration::from_secs(tot_secs)
}



