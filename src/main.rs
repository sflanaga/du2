#![allow(unused_imports)]
#![allow(unused_variables)]

extern crate users;

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

type GenError = Box<std::error::Error>;
type GenResult<T> = Result<T, GenError>;
use std::fmt;


mod util;
use util::{greek};

#[derive(Eq, Debug)]
struct TrackedPath {
    size: u64,
    path: PathBuf
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

fn walk_dir(verbose: bool, limit: usize, dir: &Path, depth: u32,
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
            for e in itr {
                let e = e?;
                let meta = e.metadata()?;
                let p = e.path();
                if meta.is_file() {
                    let s = meta.len();
                    this_tot += s;
                    local_tot += s;
                    let uid = meta.st_uid();
                    *user_map.entry(uid).or_insert(0) += s;
                    local_cnt_file += 1;
                    this_cnt +=1;
                    track_top_n2(&mut top_files, &p, s, limit); // track single immediate space
                    // println!("{}", p.to_str().unwrap());
                } else if meta.is_dir() {
                    local_cnt_dir += 1;
                    //let (that_tot, that_cnt) = walk_dir(limit, &p, depth+1, user_map, top_dir, top_cnt_dir, top_cnt_file, top_dir_overall, top_files)?;
                    match walk_dir(verbose, limit, &p, depth+1, user_map, top_dir, top_cnt_dir, top_cnt_file, top_dir_overall, top_files) {
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
                println!(r#"scanning \"{}\""#, path.to_string_lossy());
                match walk_dir(verbose, limit, &path, 0, &mut user_map, &mut top_dir,  &mut top_cnt_dir,  &mut top_cnt_file,  &mut top_dir_overall, &mut top_files) {
                    Ok( (that_tot, that_cnt) ) => { total += that_tot; count += that_cnt; },
                    Err(e) =>
                        eprint!("error trying walk top dir {}, error = {} but continuing",path.to_string_lossy(), e),
                    }
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


    println!("\nTop dir classical:");
    for v in top_dir_overall.into_sorted_vec() {
        println!("{:10} {}", greek(v.size as f64),v.path.to_string_lossy());
    }


    println!("\nTop file count under single directory:");
    for v in top_cnt_file.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

    println!("\nTop directory under single directory:");
    for v in top_cnt_dir.into_sorted_vec() {
        println!("{:10} {}", v.size,v.path.to_string_lossy());
    }

    println!("\nTop sized file(s):");
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
eprintln!(r###"\ndu2 [options] dir1 .. dirN
csv [options] <reads from stdin>
    -h|--help  this help
    -n  how many top X to track for reporting
    -v  verbose mode - mainly print directories it does not have permission to scan"###);
eprintln!("version: {}\n", version());
process::exit(1);
}



