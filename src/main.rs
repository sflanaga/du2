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
use util::{greek,dur_from_str};

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

#[allow(dead_code)]
fn track_top_n(map: &mut BTreeMap<u64, PathBuf>, path: &Path, size: u64, limit: usize) -> bool {
    if size == 0 {
        return false
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
    false
}

fn track_top_n2(heap: &mut BinaryHeap<TrackedPath>, p: &Path, s: u64, limit: usize) -> bool {
    if s == 0 {
        return false
    }

    if limit > 0 {
        if heap.len() < limit {
            heap.push(TrackedPath{size: s, path: p.to_path_buf()});
            return true
        } else if heap.peek().expect("cannot peek when the size is greater than 0!?").size < s {
            heap.pop();
            heap.push(TrackedPath{size: s, path: p.to_path_buf()});
            return true;
        }
    }
    false
}

#[derive(Debug)]
struct WalkSetting {
    verbose: bool,
    no_user: bool,
    limit: usize,
    age: TimeSpec,
    user_map: BTreeMap<u32, u64>,
    top_dir: BinaryHeap<TrackedPath>,
    top_cnt_dir: BinaryHeap<TrackedPath>,
    top_cnt_file: BinaryHeap<TrackedPath>,
    top_cnt_overall: BinaryHeap<TrackedPath>,
    top_dir_overall: BinaryHeap<TrackedPath>,
    top_files: BinaryHeap<TrackedPath>
}    


fn walk_dir(ws: &mut WalkSetting, dir: &Path, depth: u32) -> GenResult<(u64,u64)> {
    let mut paths = vec![];
    {       
        // collect the entries up front to close the read dir right away 
        let itr = fs::read_dir(dir);
        match itr {
            Ok(itr) => {
                paths = itr.collect();  
            },
            Err(e) =>
                if ws.verbose { eprintln!("Cannot read dir: {}, error: {} so skipping ", &dir.to_str().unwrap(), &e) 
            },
        }
    }
    let mut this_tot = 0;
    let mut this_cnt = 0;

    let mut local_tot = 0u64;
    let mut local_cnt_file = 0u64;
    let mut local_cnt_dir = 0u64;
    for e in paths {
        let e = e?;
        let meta = e.metadata()?;
        let p = e.path();
        if meta.is_file() {
            unsafe { COUNT_STATS +=1; }
            let f_age = meta.modified().unwrap();
            if (!ws.age.newer_than_check || ws.age.newer_than < f_age ) &&
               (!ws.age.older_than_check || ws.age.older_than > f_age) {
                let s = meta.len();
                this_tot += s;
                local_tot += s;
                if !ws.no_user {
                    let uid = meta.st_uid();
                    *ws.user_map.entry(uid).or_insert(0) += s;
                }
                local_cnt_file += 1;
                this_cnt +=1;

                track_top_n2(&mut ws.top_files, &p, s, ws.limit); 
                // println!("{}", p.to_str().unwrap());
            }
        } else if meta.is_dir() {
            local_cnt_dir += 1;
            unsafe { COUNT_STATS +=1; }
            match walk_dir(ws, &p, depth+1) {
                Ok( (that_tot, that_cnt) ) => { this_tot += that_tot; this_cnt += that_cnt; },
                Err(e) => if ws.verbose { eprint!("error trying walk {}, error = {} but continuing",p.to_string_lossy(), e) },
            };
        }
    }
    track_top_n2(&mut ws.top_dir, &dir, local_tot, ws.limit); // track single immediate space
    track_top_n2(&mut ws.top_cnt_dir, &dir, local_cnt_dir, ws.limit); // track dir with most # of dir right under it
    track_top_n2(&mut ws.top_cnt_file, &dir, local_cnt_file, ws.limit); // track dir with most # of file right under it
    track_top_n2(&mut ws.top_cnt_overall, &dir, this_cnt, ws.limit); // track overall count
    track_top_n2(&mut ws.top_dir_overall, &dir, this_tot, ws.limit); // track overall size
    Ok( (this_tot, this_cnt) )
}

fn run() -> GenResult<()> {

    let argv : Vec<String> = args().skip(1).map( |x| x).collect();
    //if argv.len() == 1 { help(); }

    let filelist = &mut vec![];

    let mut ws = WalkSetting {
        verbose: false,
        no_user: false,
        limit: 25,
        age: TimeSpec {
            newer_than_check: false,
            older_than_check: false,
            newer_than: SystemTime::now(),
            older_than: SystemTime::now(),
        },
        user_map: BTreeMap::new(),
        top_dir: BinaryHeap::new(),
        top_cnt_dir: BinaryHeap::new(),
        top_cnt_file: BinaryHeap::new(),
        top_cnt_overall: BinaryHeap::new(),
        top_dir_overall: BinaryHeap::new(),
        top_files: BinaryHeap::new(),
    };

    let mut ticker_time = 200u64;
    let age = SystemTime::now();
    let mut i = 0;
    while i < argv.len() {
        match &argv[i][..] {
            "--help" | "-h" => { // field list processing
                help();
            },
            "-n" => { 
                i += 1;
                ws.limit = argv[i].parse::<usize>().unwrap();
            },
            "-i" => { 
                i += 1;
                ticker_time = argv[i].parse::<u64>().unwrap();
            },
            "--file-newer-than" => { 
                i += 1;
                let age_i = dur_from_str(argv[i].as_str());
                ws.age.newer_than_check = true;
                ws.age.newer_than -= age_i;
                let datetime: DateTime<Local> = ws.age.newer_than.into();
                println!("consider files newer than: {}", datetime.format("%Y-%m-%d %T"));
            },
            "--file-older-than" => { 
                i += 1;
                let age_i = dur_from_str(argv[i].as_str());
                ws.age.older_than_check = true;
                ws.age.older_than -= age_i;
                let datetime: DateTime<Local> = ws.age.older_than.into();
                println!("consider files older than: {}", datetime.format("%Y-%m-%d %T"));
             },
            "-v" => { 
                ws.verbose = true;
            },
            "--no-user" => { 
                ws.no_user = true;
            },
            x => {
                if ws.verbose { println!("adding filename {} to scan", x); }
                filelist.push(x);
            }
        }
        i += 1;
    }
    if filelist.is_empty() {
        println!("Using ./ as top directory");
        filelist.push("./");
    }

    let start_f = Instant::now();

    let mut total = 0u64;
    let mut count = 0u64;

    for path in filelist {
        let path = Path::new(& path);
        if path.exists() {
            if path.metadata().unwrap().is_dir() {
                println!("scanning \"{}\"", path.to_string_lossy());

                let thread = thread::spawn(move || {
                    if !console::user_attended() {return; }
                    let term = Term::stdout();
                    let mut last_cnt = 0f64;
                    let mut ticks = 1f64;
                    loop {
                        unsafe {
                            let mult = 1000.0 / ticker_time as f64;
                            let delta = (COUNT_STATS as f64 - last_cnt)*mult;
                            let allrate = (COUNT_STATS as f64)*mult/ticks;
                            let now_cnt = COUNT_STATS as f64;
                            print!("nodes: {} spot rate/s: {} all rate/s: {}", now_cnt.separated_string(), delta.separated_string(), allrate.separated_string());
                            std::io::stdout().flush().unwrap();
                            last_cnt = COUNT_STATS as f64;
                            thread::sleep(Duration::from_millis(ticker_time));
                            term.clear_line().unwrap();
                            if TICK_GO == 0 {
                                break;
                            }
                            ticks += 1.0;
                         }
                    }
                });

                match walk_dir(&mut ws,  &path, 0) {
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
    let sec = (elapsed.as_secs() as f64)+((elapsed.subsec_nanos() as f64)/1_000_000_000.0);

    #[derive(Debug)]
    struct U2u {
        size: u64,
        uid: u32
    };
    let mut user_vec: Vec<U2u> = ws.user_map.iter().map( |(&x,&y)| U2u {size: y, uid:x } ).collect();
    user_vec.sort_by( |b,a| a.size.cmp(&b.size).then(b.uid.cmp(&b.uid)) );
    if total>0 {
        println!("File space scanned: {} and {} files in {} seconds", greek(total as f64), count, sec);
        if !user_vec.is_empty() { 
            println!("\nSpace used per user");
            for ue in &user_vec {
                match get_user_by_uid(ue.uid) {
                    None => println!("uid{:7} {} ", ue.uid, greek(ue.size as f64)),
                    Some(user) => println!("{:10} {} ", user.name(), greek(ue.size as f64)),
                }

            }
        }
    } else {
        eprintln!("nothing scanned");
        return Ok( () );
    }
    
    println!("\nTop dir with space usage directly inside them:");
    for v in ws.top_dir.into_sorted_vec() {
        println!("{:10} {}", greek(v.size as f64),v.path.to_string_lossy());
    }

    println!("\nTop dir size recursive:");
    for v in ws.top_dir_overall.into_sorted_vec() {
        println!("{:10} {}", greek(v.size as f64),v.path.to_string_lossy());
    }

    println!("\nTop count of files  recursive:");
    for v in ws.top_cnt_overall.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

     println!("\nTop counts of files in a single directory:");
    for v in ws.top_cnt_file.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

    println!("\nTop counts of directories in a single directory:");
    for v in ws.top_cnt_dir.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

    println!("\nLargest file(s):");
    for v in ws.top_files.into_sorted_vec() {
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


pub fn version() -> &'static str {
    concat!(env!("CARGO_PKG_VERSION"), include_str!(concat!(env!("OUT_DIR"), "/commit-info.txt")))
}



fn help() {
eprintln!("\ndu2 [options] dir1 .. dirN
csv [options] <reads from stdin>
    -h|--help  this help
    --no-user  cuts off user id level collection - experimental for speed?
    --file-newer-than <time ago spec>
    --file-older-than <time ago spec>
    note: time ago spec example: 1y22d4m means 1 year 22 days and 4 minutes ago
    -a [2h33m32s] count only files older than X time ago 
    -n  how many top X to track for reporting
    -i  ticker interval is ms - default is 200ms
    -v  verbose mode - mainly print directories it does not have permission to scan");
eprintln!("version: {}\n", version());
process::exit(1);
}



