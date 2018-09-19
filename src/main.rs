#![allow(unused_imports)]
#![allow(unused_variables)]

extern crate users;
extern crate console;
extern crate separator;
extern crate chrono;

use chrono::offset::Local;
use chrono::DateTime;

use std::fs;
use std::env::args;
use std::process;
use std::io::prelude::*;
use std::path::Path;
use std::path::PathBuf;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BinaryHeap;
use std::os::linux::fs::MetadataExt;
use users::{get_user_by_uid, get_current_uid};
use std::time::Instant;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize};
use std::thread;
use console::Term;

use std::time::{Duration, SystemTime};

type GenError = Box<std::error::Error>;
type GenResult<T> = Result<T, GenError>;
use std::fmt;

use separator::Separatable;

mod util;
use util::{greek};

static mut COUNT_STATS: usize = 0;
static mut TICK_GO: usize = 1;

#[derive(Eq, Debug)]
struct TrackedPath {
    size: u64,
    path: PathBuf
}

#[derive(Debug)]
struct TimeSpec {
    newer_than_check: bool,
    older_than_check: bool,
    older_than: SystemTime,
    newer_than: SystemTime,
}

impl Ord for TrackedPath {
    fn cmp(&self, other: &TrackedPath) -> Ordering {
        self.size.cmp(&other.size).reverse()
    }
}

impl PartialOrd for TrackedPath {
    fn partial_cmp(&self, other: &TrackedPath) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TrackedPath {
    fn eq(&self, other: &TrackedPath) -> bool {
        self.size == other.size
    }
}

fn durFromStr(s: &str) -> Duration {
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


fn track_top_n(map: &mut BTreeMap<u64, PathBuf>, path: &Path, size: u64, limit: usize) -> bool {
    if size <= 0 {
        return false;
    }

    if limit > 0 {
        if map.len() < limit {
            let spath = path.to_path_buf();
            map.insert(size, spath);
            return true
        } else {
            let lowest = match map.iter().next() {
                Some( (l,p) ) => *l,
                None => 0u64
            };
            if lowest < size {
                map.remove(&lowest);
                let spath = path.to_path_buf();
                map.insert(size, spath);
            }
        }
    }
    return false;
}

fn track_top_n2(heap: &mut BinaryHeap<TrackedPath>, p: &Path, s: u64, limit: usize) -> bool {
    if s <= 0 {
        return false;
    }

    if limit > 0 {
        if heap.len() < limit {
            heap.push(TrackedPath{size: s, path: p.to_path_buf()});
            return true
        } else {
            if heap.peek().expect("cannot peek when the size is greater than 0!?").size < s {
                heap.pop();
                heap.push(TrackedPath{size: s, path: p.to_path_buf()});
                return true;
            }
        }
    }
    return false;
}

fn walk_dir(verbose: bool, limit: usize, age: &TimeSpec, dir: &Path, depth: u32,
    user_map: &mut BTreeMap<u32, u64>,
    mut top_dir: &mut BinaryHeap<TrackedPath>,
    mut top_cnt_dir: &mut BinaryHeap<TrackedPath>,
    mut top_cnt_file: &mut BinaryHeap<TrackedPath>,
    mut top_dir_overall: &mut BinaryHeap<TrackedPath>,
    mut top_files: &mut BinaryHeap<TrackedPath>) -> GenResult<(u64,u64)> {
    let itr = fs::read_dir(dir);
    let mut this_tot = 0;
    let mut this_cnt = 0;
    match itr {
        Ok(itr) => {
            let mut local_tot = 0u64;
            let mut local_cnt_file = 0u64;
            let mut local_cnt_dir = 0u64;
            let paths : Vec<_> = itr.collect();
            for e in paths {
                let e = e?;
                let meta = e.metadata()?;
                let p = e.path();
                if meta.is_file() {
                    let f_age = meta.modified().unwrap();
                    if (!age.newer_than_check || age.newer_than < f_age ) &&
                       (!age.older_than_check || age.older_than > f_age) {
                        let s = meta.len();
                        this_tot += s;
                        local_tot += s;
                        let uid = meta.st_uid();
                        *user_map.entry(uid).or_insert(0) += s;
                        local_cnt_file += 1;
                        this_cnt +=1;

                        unsafe { COUNT_STATS +=1; }
                        track_top_n2(&mut top_files, &p, s, limit); 
                        // println!("{}", p.to_str().unwrap());
                    }
                } else if meta.is_dir() {
                    local_cnt_dir += 1;
                    unsafe { COUNT_STATS +=1; }
                    match walk_dir(verbose, limit, &age, &p, depth+1, user_map, top_dir, top_cnt_dir, top_cnt_file, top_dir_overall, top_files) {
                        Ok( (that_tot, that_cnt) ) => { this_tot += that_tot; this_cnt += that_cnt; },
                        Err(e) => if verbose { eprint!("error trying walk {}, error = {} but continuing",p.to_string_lossy(), e) },
                    };
                }
            }
            track_top_n2(&mut top_dir, &dir, local_tot, limit); // track single immediate space
            track_top_n2(&mut top_cnt_dir, &dir, local_cnt_dir, limit); // track dir with most # of dir right under it
            track_top_n2(&mut top_cnt_file, &dir, local_cnt_file, limit); // track dir with most # of file right under it
            track_top_n2(&mut top_dir_overall, &dir, this_tot, limit); // track top dirs overall - main will be largest
        },
        Err(e) =>
            if verbose { eprintln!("Cannot read dir: {}, error: {} so skipping ", &dir.to_str().unwrap(), &e) },
    }
    return Ok( (this_tot, this_cnt) );
}

fn run() -> GenResult<()> {

    let argv : Vec<String> = args().skip(1).map( |x| x).collect();
    //if argv.len() == 1 { help(); }

    let filelist = &mut vec![];
    let mut verbose = false;
    let mut limit = 25;
    let mut time_spec = TimeSpec {
        newer_than_check: false,
        older_than_check: false,
        newer_than: SystemTime::now(),
        older_than: SystemTime::now(),
    };

    let mut age = SystemTime::now();
    let mut i = 0;
    while i < argv.len() {
        match &argv[i][..] {
            "--help" | "-h" => { // field list processing
                help();
            },
            "-n" => { 
                i += 1;
                limit = argv[i].parse::<usize>().unwrap();
            },
            "--file-newer-than" => { 
                i += 1;
                let age_i = durFromStr(argv[i].as_str());
                time_spec.newer_than_check = true;
                time_spec.newer_than = time_spec.newer_than - age_i;
                let datetime: DateTime<Local> = time_spec.newer_than.into();
                println!("consider files newer than: {}", datetime.format("%Y-%m-%d %T"));
            },
            "--file-older-than" => { 
                i += 1;
                let age_i = durFromStr(argv[i].as_str());
                time_spec.older_than_check = true;
                time_spec.older_than = time_spec.older_than - age_i;
                let datetime: DateTime<Local> = time_spec.older_than.into();
                println!("consider files older than: {}", datetime.format("%Y-%m-%d %T"));
             },
            "-v" => { 
                verbose = true;
            },
            x => {
                if verbose { println!("adding filename {} to scan", x); }
                filelist.push(x);
            }
        }
        i += 1;
    }
    if filelist.len() <= 0 {
        println!("Using ./ as top directory");
        filelist.push("./");
    }

    let start_f = Instant::now();

    let mut user_map: BTreeMap<u32, u64> = BTreeMap::new();

    let mut top_dir: BinaryHeap<TrackedPath> = BinaryHeap::new();
    let mut top_cnt_dir: BinaryHeap<TrackedPath> = BinaryHeap::new();
    let mut top_cnt_file: BinaryHeap<TrackedPath> = BinaryHeap::new();
    let mut top_dir_overall: BinaryHeap<TrackedPath> = BinaryHeap::new();
    let mut top_files: BinaryHeap<TrackedPath> = BinaryHeap::new();
    let mut total = 0u64;
    let mut count = 0u64;


    for path in filelist {
        let path = Path::new(& path);
        if path.exists() {
            if path.metadata().unwrap().is_dir() {
                println!("scanning \"{}\"", path.to_string_lossy());

                let thread = thread::spawn(|| {
                    if !console::user_attended() {return; }
                    let term = Term::stdout();
                    let mut last_cnt = 0usize;
                    let mut ticks = 1usize;
                    loop {
                        unsafe {
                            let delta = (COUNT_STATS - last_cnt)*5;
                            let allrate = COUNT_STATS*5/ticks;
                            let now_cnt = COUNT_STATS;
                            print!("nodes: {} spot rate/s: {} all rate/s: {}", now_cnt.separated_string(), delta.separated_string(), allrate.separated_string());
                            std::io::stdout().flush().unwrap();
                            last_cnt = COUNT_STATS;
                            thread::sleep(Duration::from_millis(200));
                            term.clear_line();
                            if TICK_GO == 0 {
                                break;
                            }
                            ticks += 1;
                         }
                    }
                });

                match walk_dir(verbose, limit, &time_spec, &path, 0, &mut user_map, &mut top_dir,  &mut top_cnt_dir,  &mut top_cnt_file,  &mut top_dir_overall, &mut top_files) {
                    Ok( (that_tot, that_cnt) ) => { total += that_tot; count += that_cnt; },
                    Err(e) =>
                        eprint!("error trying walk top dir {}, error = {} but continuing",path.to_string_lossy(), e),
                    }
                unsafe { TICK_GO = 0; }
                thread.join().unwrap();

            } else { // not a dir
                eprintln!("path \"{}\" is a file and not a directory!", path.to_string_lossy());
            }
        } else { // does not exist
            eprintln!("path \"{}\" does not exist!", path.to_string_lossy());
        }
    }
    //let (total,count) = walk_dir(limit, &path, 0, &mut user_map, &mut top_dir,  &mut top_cnt_dir,  &mut top_cnt_file,  &mut top_dir_overall, &mut top_files)?;

    let elapsed = start_f.elapsed();
    let sec = (elapsed.as_secs() as f64) + (elapsed.subsec_nanos() as f64 / 1000_000_000.0);


    #[derive(Debug)]
    struct u2u {
        size: u64,
        uid: u32
    };
    let mut user_vec: Vec<u2u> = user_map.iter().map( |(&x,&y)| u2u {size: y, uid:x } ).collect();
    user_vec.sort_by( |b,a| a.size.cmp(&b.size).then(b.uid.cmp(&b.uid)) );
    if user_vec.len() > 0 {
        println!("File space scanned: {} and {} files in {} seconds", greek(total as f64), count, sec);

        println!("\nSpace used per user");
        for ue in &user_vec {
            match get_user_by_uid(ue.uid) {
                None => println!("uid{:7} {} ", ue.uid, greek(ue.size as f64)),
                Some(user) => println!("{:10} {} ", user.name(), greek(ue.size as f64)),
            }

        }
    } else {
        eprintln!("nothing scanned");
        return Ok( () );
    }
    
    println!("\nTop dir with space usage directly inside them:");
    for v in top_dir.into_sorted_vec() {
        println!("{:10} {}", greek(v.size as f64),v.path.to_string_lossy());
    }


    println!("\nTop dir size recursive:");
    for v in top_dir_overall.into_sorted_vec() {
        println!("{:10} {}", greek(v.size as f64),v.path.to_string_lossy());
    }


    println!("\nTop counts of files in a single directory:");
    for v in top_cnt_file.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

    println!("\nTop counts of directories in a single directory:");
    for v in top_cnt_dir.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

    println!("\nLargest file(s):");
    for v in top_files.into_sorted_vec() {
        println!("{:10} {}", greek(v.size as f64),v.path.to_string_lossy());
    }

    Ok( () )
}

fn main() {
    if let Err(err) = run() {
        println!("uncontrolled error: {}", &err);
        std::process::exit(1);
    }
}


const VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    concat!(env!("CARGO_PKG_VERSION"), include_str!(concat!(env!("OUT_DIR"), "/commit-info.txt")))
}



fn help() {
eprintln!("\ndu2 [options] dir1 .. dirN
csv [options] <reads from stdin>
    -h|--help  this help
    --file-newer-than <time ago spec>
    --file-older-than <time ago spec>
    note: time ago spec example: 1y22d4m means 1 year 22 days and 4 minutes ago
    -a [2h33m32s] count only files older than X time ago 
    -n  how many top X to track for reporting
    -v  verbose mode - mainly print directories it does not have permission to scan");
eprintln!("version: {}\n", version());
process::exit(1);
}



