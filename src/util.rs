pub fn greekshort_i(v: u64) -> String {
    const GREEK_SUFFIXES: &'static [&'static str] = &["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];

    let mut number = v;
    let mut multi = 0;

    while number >= 1000 && multi < GREEK_SUFFIXES.len()-1 {
        multi += 1;
        number /= 1024u64;
    }

    let mut s = format!("{}", number);
    s.truncate(4);
    if s.ends_with(".") {
        s.pop();
    }
    if s.len() < 4 { s.push(' ' ); }

    return format!("{}{}", s, GREEK_SUFFIXES[multi]);
}

pub fn greek(v: f64) -> String {
    const GREEK_SUFFIXES: &'static [&'static str] = &["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];

    let mut number = v;
    let mut multi = 0;

    while number >= 1000.0 && multi < GREEK_SUFFIXES.len()-1 {
        multi += 1;
        number /= 1024.0;
    }

    let mut s = format!("{}", number);
    s.truncate(4);
    if s.ends_with(".") {
        s.pop();
    }
    if s.len() < 4 { s.push(' ' ); }

    return format!("{}{}", s, GREEK_SUFFIXES[multi]);
}

