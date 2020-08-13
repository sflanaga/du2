#![allow(dead_code)]
#![allow(unused_imports)]

use std::cmp::max;
use std::collections::{BinaryHeap, BTreeMap};
use std::fs::{FileType, Metadata, symlink_metadata};
#[cfg(target_family = "unix")]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
#[cfg(target_family = "windows")]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::thread::spawn;
use std::time::{Duration, Instant};
use std::time::SystemTime;

use cpu_time::ProcessTime;
use structopt::StructOpt;
#[cfg(target_family = "unix")]
use users::{get_current_uid, get_user_by_uid};

use anyhow::{anyhow, Context, Result};
use lazy_static::lazy_static;
use util::greek;
use worker_queue::*;

use crate::tstatus::{ThreadStatus, ThreadTracker, spawn_death_timeout_thread};
use crate::util::multi_extension;
use crate::cli::{APP,EXE};

mod tstatus;
mod worker_queue;
mod util;
mod cli;


fn main() {
    if let Err(err) = parls() {
        eprintln!("ERROR in main: {}", &err);
        std::process::exit(11);
    }
}


//noinspection ALL
fn read_dir_thread(queue: &mut WorkerQueue<Option<PathBuf>>, out_q: &mut WorkerQueue<Option<Vec<(PathBuf, Metadata)>>>, t_status: &mut ThreadStatus) {
    // get back to work slave loop....
    let t_cpu_time = cpu_time::ThreadTime::now();
    loop {
        match _read_dir_worker(queue, out_q, t_status) {
            Err(e) => {
                // filthy filthy error catch
                eprintln!("{}: major error: {}  cause: {}", *EXE, e, e.root_cause());
            }
            Ok(()) => break,
        }
    }

    if APP.write_thread_cpu_time {
        eprintln!("read dir thread cpu time: {:.3}", t_cpu_time.elapsed().as_secs_f64());
    }
}

//noinspection ALL
fn _read_dir_worker(queue: &mut WorkerQueue<Option<PathBuf>>, out_q: &mut WorkerQueue<Option<Vec<(PathBuf, Metadata)>>>, t_status: &mut ThreadStatus) -> Result<()> {
    let mut pops_done = 0;
    t_status.register("started");
    loop {
        if APP.update_status {
            t_status.set_state("pop blocked");
        }
        match queue.pop() {
            None => break,
            Some(p) => { //println!("path: {}", p.to_str().unwrap()),
                pops_done += 1;
                if APP.verbose > 1 {
                    if p.to_str().is_none() { break; } else { eprintln!("{}: listing for {}", *EXE, p.to_str().unwrap()); }
                }
                let mut other_dirs = vec![];
                let mut metalist = vec![];
                if APP.update_status {
                    t_status.set_state(&format!("at {} pops; reading dir: {}", pops_done, p.display()));
                }
                let dir_itr = match std::fs::read_dir(&p) {
                    Err(e) => {
                        eprintln!("{}: stat of dir: '{}', error: {}", *EXE, p.display(), e);
                        continue;
                    }
                    Ok(i) => i,
                };
                for entry in dir_itr {
                    let entry = entry?;
                    let path = entry.path();
                    let md = match symlink_metadata(&entry.path()) {
                        Err(e) => {
                            eprintln!("{}: stat of file for symlink: '{}', error: {}", *EXE, p.display(), e);
                            continue;
                        }
                        Ok(md) => md,
                    };
                    if APP.verbose > 3 {
                        eprintln!("{}: raw meta: {:#?}", *EXE, &md);
                    }


                    let file_type: FileType = md.file_type();
                    if !file_type.is_symlink() {
                        if file_type.is_file() {
                            //
                            // re filters
                            //
                            if let Some(re) = &APP.re {
                                let s = path.to_str().unwrap();
                                if !re.is_match(path.to_str().unwrap()) {
                                    if APP.verbose > 1 {
                                        eprintln!("{}: filtered file not matching re: {}", *EXE, s);
                                    }
                                    continue;
                                }
                                if APP.verbose > 1 {
                                    eprintln!("{}: NOT filtered file DOES match re: {}", *EXE, s);
                                }
                            }
                            if let Some(re) = &APP.exclude_re {
                                let s = path.to_str().unwrap();
                                if re.is_match(path.to_str().unwrap()) {
                                    if APP.verbose > 1 {
                                        eprintln!("{}: filtered path matching exclude_re: {}", *EXE, s);
                                    }
                                    continue;
                                }
                                if APP.verbose > 1 {
                                    eprintln!("{}: NOT filtered file DOES NOT match re: {}", *EXE, s);
                                }
                            }
                            //
                            // age filters
                            //
                            let f_age = md.modified()?;
                            if APP.file_newer_than.map_or(true, |x| x < f_age) && APP.file_older_than.map_or(true, |x| x > f_age) {
                                metalist.push((path.clone(), md.clone()));
                                //write_meta(&path, &md);
                            }
                        } else if file_type.is_dir() {
                            metalist.push((path.clone(), md));
                            other_dirs.push(path);
                        }
                    } else {
                        if APP.verbose > 0 { eprintln!("{}: skipping sym link: {}", *EXE, path.to_string_lossy()); }
                    }
                }

                if APP.update_status {
                    t_status.set_state(&format!("push meta, at {} pops", pops_done));
                }
                out_q.push(Some(metalist))?;

                if APP.update_status {
                    t_status.set_state(&format!("pushing {} dirs and at {} pops", other_dirs.len(), pops_done));
                }
                for d in other_dirs {
                    queue.push(Some(d))?;
                }
            }
        }
    }
    if APP.update_status {
        t_status.set_state("exit");
    }
    Ok(())
}

// TODO: add username and or id
// get windows user id / name?  how?  up to snuff here with unix

#[cfg(target_family = "unix")]
fn write_meta(path: &PathBuf, meta: &Metadata) -> Result<()> {
    let file_type = match meta.file_type() {
        x if x.is_file() => 'f',
        x if x.is_dir() => 'd',
        x if x.is_symlink() => 's',
        _ => 'N',
    };
    match get_user_by_uid(meta.uid()) {
        None => {
            println!("{}{}{}{}{}{}{:o}{}{}{}{}", file_type, APP.delimiter, path.to_string_lossy(),
                     APP.delimiter, meta.size(), APP.delimiter, meta.permissions().mode(), APP.delimiter,
                     meta.uid(), APP.delimiter, meta.modified()?.duration_since(SystemTime::UNIX_EPOCH)?.as_secs());
        }
        Some(user) => {
            println!("{}{}{}{}{}{}{:o}{}{}{}{}", file_type, APP.delimiter, path.to_string_lossy(),
                     APP.delimiter, meta.size(), APP.delimiter, meta.permissions().mode(), APP.delimiter,
                     user.name().to_string_lossy(), APP.delimiter, meta.modified()?.duration_since(SystemTime::UNIX_EPOCH)?.as_secs());
        }
    };
    Ok(())
}

#[cfg(target_family = "unix")]
fn write_meta_header() {
    println!("{}{}{}{}{}{}{}{}{}{}{}", "type", APP.delimiter, "path",
             APP.delimiter, "size", APP.delimiter, "permissions", APP.delimiter,
             "user", APP.delimiter,
             "epoch_last_modification");
}

#[cfg(target_family = "windows")]
fn write_meta_header() {
    println!("{}{}{}{}{}{}{}{}{}", "type", APP.delimiter, "path",
             APP.delimiter, "size", APP.delimiter, "readonly", APP.delimiter,
             "epoch_last_modification");
}
#[cfg(target_family = "windows")]
fn write_meta(path: &PathBuf, meta: &Metadata) -> Result<()> {
    let file_type = match meta.file_type() {
        x if x.is_file() => 'f',
        x if x.is_dir() => 'd',
        x if x.is_symlink() => 's',
        _ => 'N',
    };
    println!("{}{}{}{}{}{}{}{}{}", file_type, APP.delimiter, path.display(),
             APP.delimiter, meta.len(), APP.delimiter, meta.permissions().readonly(), APP.delimiter,
             meta.modified()?.duration_since(SystemTime::UNIX_EPOCH)?.as_secs());
    Ok(())
}

#[derive(Eq, Debug)]
struct TrackedPath {
    size: u64,
    path: PathBuf,
}

#[derive(Eq, Debug)]
struct TrackedExtension {
    size: u64,
    extension: String,
}

impl Ord for TrackedPath {
    fn cmp(&self, other: &TrackedPath) -> std::cmp::Ordering {
        self.size.cmp(&other.size).reverse()
    }
}

impl PartialOrd for TrackedPath {
    fn partial_cmp(&self, other: &TrackedPath) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TrackedPath {
    fn eq(&self, other: &TrackedPath) -> bool {
        self.size == other.size
    }
}

impl Ord for TrackedExtension {
    fn cmp(&self, other: &TrackedExtension) -> std::cmp::Ordering {
        self.size.cmp(&other.size).reverse()
    }
}

impl PartialOrd for TrackedExtension {
    fn partial_cmp(&self, other: &TrackedExtension) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TrackedExtension {
    fn eq(&self, other: &TrackedExtension) -> bool {
        self.size == other.size
    }
}

#[derive(Debug, Clone)]
pub struct AgeRange {
    oldest_file_direct: Option<Duration>,
    oldest_file_recursive: Option<Duration>,
    newest_file_direct: Option<Duration>,
    newest_file_recursive: Option<Duration>,
}

impl AgeRange {
    pub fn new() -> Self {
        AgeRange{
            oldest_file_direct: None,
            oldest_file_recursive: None,
            newest_file_direct: None,
            newest_file_recursive: None,
        }
    }
    pub fn update_direct(&mut self, new: &Duration) {
        Self::max_age(&mut self.oldest_file_direct, &new);
        Self::min_age(&mut self.newest_file_direct, &new);
    }
    pub fn update_recursive(&mut self, new: &Duration) {
        Self::max_age(&mut self.oldest_file_recursive, &new);
        Self::min_age(&mut self.newest_file_recursive, &new);
    }
    fn min_age(store: &mut Option<Duration>, new: &Duration) {
        match store {
            None => *store = Some(*new),
            Some(s) => if new < s {
                *store = Some(*new);
            },
        }
    }

    fn max_age(store: &mut Option<Duration>, new: &Duration) {
        match store {
            None => *store = Some(*new),
            Some(s) => if new > s {
                *store = Some(*new);
            },
        }
    }
}


#[derive(Debug, Clone)]
struct DirStats {
    size_directly: u64,
    size_recursively: u64,
    file_count_directly: u64,
    file_count_recursively: u64,
    dir_count_directly: u64,
    dir_count_recursively: u64,
    age_range: AgeRange,
}


impl DirStats {
    pub fn new() -> Self {
        DirStats { size_recursively: 0, size_directly: 0, file_count_recursively: 0, file_count_directly: 0, dir_count_directly: 0, dir_count_recursively: 0 , age_range: AgeRange::new(), }
    }
}

#[derive(Debug)]
struct UserUsage {
    size: u64,
    uid: u32,
}

struct AllStats {
    dtree: BTreeMap<PathBuf, DirStats>,
    extensions: BTreeMap<String, u64>,
    user_map: BTreeMap<u32, (u64, u64)>,
    top_dir: BinaryHeap<TrackedPath>,
    top_cnt_dir: BinaryHeap<TrackedPath>,
    top_cnt_file: BinaryHeap<TrackedPath>,
    top_cnt_overall: BinaryHeap<TrackedPath>,
    top_dir_overall: BinaryHeap<TrackedPath>,
    top_files: BinaryHeap<TrackedPath>,
    top_ext: BinaryHeap<TrackedExtension>,
    total_usage: u64,
}

//noinspection ALL
fn track_top_n_ext(heap: &mut BinaryHeap<TrackedExtension>, ext: &String, s: u64, limit: usize) {
    if limit > 0 {
        if heap.len() < limit {
            heap.push(TrackedExtension { size: s, extension: ext.clone() });
            return;
        } else if heap.peek().expect("internal error: cannot peek when the size is greater than 0!?").size < s {
            heap.pop();
            heap.push(TrackedExtension { size: s, extension: ext.clone() });
            return;
        }
    }
}

//noinspection ALL
fn track_top_n(heap: &mut BinaryHeap<TrackedPath>, p: &PathBuf, s: u64, limit: usize) {

    if limit > 0 {
        if heap.len() < limit {
            heap.push(TrackedPath { size: s, path: p.clone() });
            return;
        } else if heap.peek().expect("internal error: cannot peek when the size is greater than 0!?").size < s {
            heap.pop();
            heap.push(TrackedPath { size: s, path: p.clone() });
            return;
        }
    }
}

fn child_is_higher_than_base_dir(base: &Path, child: &Path) -> Result<bool> {
    let base_s = base.to_string_lossy();
    let child_s = child.to_string_lossy();

    // we are trying to avoid canonicalize() as it appears to access the file system
    // further trying to do this on OsStr seems pointless - no iterator or access
    // many others access the bytes of OsStr and compare things.... why is it not std?
    if child_s.starts_with(&base_s[..]) {
        Ok(false)
    } else {
        Ok(true)
    }
}

//noinspection ALL
fn perk_up_disk_usage(top: &mut AllStats, list: &Vec<(PathBuf, Metadata)>) -> Result<()> {
    if list.len() > 0 {
        if let Some(mut parent) = list[0].0.ancestors().skip(1).next() {
            if child_is_higher_than_base_dir(&APP.dir, &parent)? {
                return Ok(())
            }

            let dstats = {
                let mut dstats: &mut DirStats = if top.dtree.contains_key(parent) {
                    top.dtree.get_mut(parent).unwrap()
                } else {
                    let dstats = DirStats::new();
                    top.dtree.insert(parent.to_path_buf(), dstats);
                    top.dtree.get_mut(parent).unwrap()
                };
                for afile in list {
                    let filetype = afile.1.file_type();
                    let f_age = afile.1.modified()?;

                    track_top_n(&mut top.top_files, &afile.0.to_path_buf(), afile.1.len(), APP.limit);

                    #[cfg(target_family = "windows")]
                        let uid = 0;
                    #[cfg(target_family = "unix")]
                        let uid = afile.1.uid();
                    let ref mut tt = *top.user_map.entry(uid).or_insert((0, 0));
                    tt.0 += 1;
                    tt.1 += afile.1.len();
                    top.total_usage += afile.1.len();

                    if filetype.is_file() {
                        if APP.file_newer_than.map_or(true, |x| x < f_age) && APP.file_older_than.map_or(true, |x| x > f_age) {
                            if let Some(ext) = multi_extension(&afile.0) {
                                match top.extensions.get_mut(ext.as_ref()) {
                                    Some(ext_sz) => *ext_sz += afile.1.len(),
                                    None => { top.extensions.insert(ext.to_string(), afile.1.len()); }
                                }
                            };

                            dstats.file_count_directly += 1;
                            dstats.file_count_recursively += 1;
                            dstats.size_directly += afile.1.len();
                            dstats.size_recursively += afile.1.len();
                        }
                    } else if filetype.is_dir() {
                        if APP.file_newer_than.map_or(true, |x| x < f_age) && APP.file_older_than.map_or(true, |x| x > f_age) {
                            dstats.dir_count_directly += 1;
                            dstats.dir_count_recursively += 1;
                            // eprintln!("dir size {} :: {}", afile.0.display(), afile.1.len());
                            dstats.size_directly += afile.1.len();
                            dstats.size_recursively += afile.1.len();
                        }
                    }
                }
                dstats.clone()
            };
            // go up tree and add stuff
            loop {
                if let Some(nextpar) = parent.ancestors().skip(1).next() {
                    if parent == APP.dir { break; }

                    if nextpar == parent {
                        break;
                    }
                    let mut upstats = if top.dtree.contains_key(nextpar) {
                        top.dtree.get_mut(nextpar).unwrap()
                    } else {
                        let dstats = DirStats::new();
                        top.dtree.insert(nextpar.to_path_buf(), dstats);
                        top.dtree.get_mut(nextpar).unwrap()
                    };
                    upstats.size_recursively += dstats.size_recursively;
                    upstats.file_count_recursively += dstats.file_count_recursively;
                    upstats.dir_count_recursively += dstats.dir_count_recursively;

                    //eprintln!("up: {} from {}", nextpar.display(), parent.display());
                    parent = nextpar;
                } else {
                    break;
                }
            }
        }
    }
    Ok(())
}

//noinspection ALL
fn file_track(startout: Instant,
              cputime: ProcessTime,
              stats: &mut AllStats,
              out_q: &mut WorkerQueue<Option<Vec<(PathBuf, Metadata)>>>,
              work_q: &mut WorkerQueue<Option<PathBuf>>,
              t_status: &mut ThreadStatus,
) -> Result<()> {
    t_status.register("started");
    let t_cpu_thread_time = cpu_time::ThreadTime::now();
    let count = Arc::new(AtomicUsize::new(0));
    let sub_count = count.clone();
    let sub_out_q = out_q.clone();
    let sub_work_q = work_q.clone();
    if APP.progress {
        thread::spawn(move || {
            let mut last = 0;
            let start_f = Instant::now();

            loop {
                thread::sleep(Duration::from_millis(APP.ticker_interval));
                let thiscount = sub_count.load(Ordering::Relaxed);

                let elapsed = start_f.elapsed();
                let sec: f64 = (elapsed.as_secs() as f64) + (elapsed.subsec_nanos() as f64 / 1_000_000_000.0);
                let rate = (thiscount as f64 / sec) as usize;

                let stats_workers: QueueStats = sub_work_q.get_stats();
                let stats_io: QueueStats = sub_out_q.get_stats();
                eprint!("\rfiles: {}  rate: {}  blocked: {}  directory q len: {}  io q len: {}                 ",
                        thiscount, rate, stats_workers.curr_poppers, stats_workers.curr_q_len, stats_io.curr_q_len);
                if thiscount < last {
                    break;
                }
                last = thiscount;
            }
        });
    }

    let mut pop_count = 0;
    if APP.list_files {
        write_meta_header();
    }

    loop {
        //let mut c_t_status = t_status;
        if APP.update_status {
            t_status.set_state(&format!("wait at pop: nodes: {}", pop_count));
        }
        match out_q.pop() {
            Some(list) => {
                pop_count += list.len();
                if APP.update_status {
                    t_status.set_state(&format!("perking pop: {}", pop_count));
                }

                count.fetch_add(list.len(), Ordering::Relaxed);
                if APP.usage_mode {
                    if APP.update_status {
                        t_status.set_state("recording stats");
                    }
                    perk_up_disk_usage(stats, &list)?;
                }
                if APP.list_files {
                    for (path, md) in list {
                        let f_age = md.modified()?;
                        if APP.file_newer_than.map_or(true, |x| x < f_age) && APP.file_older_than.map_or(true, |x| x > f_age) {
                            if APP.t_status_interval {
                                t_status.set_state("writing meta data");
                            }
                            write_meta(&path, &md)?
                        }
                    }
                }
            }
            None => break,
        }
    }
    if APP.update_status {
        t_status.set_state(&format!("perking {} entries", stats.dtree.len()));
    }
    let last_count = count.load(Ordering::Relaxed);
    count.store(0, Ordering::Relaxed);

    if APP.usage_mode {
        let track_cpu_time = cpu_time::ThreadTime::now();
        use num_format::{Locale, ToFormattedString};
        println!("Scanned {} files / {} usage in [{:.3} / {:.3}] (real / cpu) seconds",
                 last_count.to_formatted_string(&Locale::en),
                 greek(stats.total_usage as f64),
                 (Instant::now() - startout).as_secs_f64(), cputime.elapsed().as_secs_f64());
        for x in stats.dtree.iter() {
            track_top_n(&mut stats.top_dir, &x.0, x.1.size_directly, APP.limit); // track single immediate space
            track_top_n(&mut stats.top_cnt_dir, &x.0, x.1.dir_count_directly, APP.limit); // track dir with most # of dir right under it
            track_top_n(&mut stats.top_cnt_file, &x.0, x.1.file_count_directly, APP.limit); // track dir with most # of file right under it
            track_top_n(&mut stats.top_cnt_overall, &x.0, x.1.file_count_recursively, APP.limit); // track overall count
            track_top_n(&mut stats.top_dir_overall, &x.0, x.1.size_recursively, APP.limit); // track overall size
        }

        for x in stats.extensions.iter() {
            track_top_n_ext(&mut stats.top_ext, &x.0, *x.1, APP.limit);
        }
        if APP.update_status {
            t_status.set_state("print");
        }
        print_disk_report(&stats);
        eprintln!("perk cpu time: {}", track_cpu_time.elapsed().as_secs_f32());
    }
    if APP.update_status {
        t_status.set_state("exit");
    }
    if APP.write_thread_cpu_time {
        eprintln!("file track thread cpu time: {:.3}", t_cpu_thread_time.elapsed().as_secs_f64());
    }
    Ok(())
}

//noinspection ALL
fn to_sort_vec(heap: &BinaryHeap<TrackedPath>) -> Vec<TrackedPath> {
    let mut v = Vec::with_capacity(heap.len());
    for i in heap {
        v.push(TrackedPath {
            path: i.path.clone(),
            size: i.size,
        });
    }
    v.sort();
    v
}

//noinspection ALL
fn to_sort_vec_file_ext(heap: &BinaryHeap<TrackedExtension>) -> Vec<TrackedExtension> {
    let mut v = Vec::with_capacity(heap.len());
    for i in heap {
        v.push(TrackedExtension {
            extension: i.extension.clone(),
            size: i.size,
        });
    }
    v.sort();
    v
}

//noinspection ALL
fn print_disk_report(stats: &AllStats) {
    #[derive(Debug)]
    struct U2u {
        count: u64,
        size: u64,
        uid: u32,
    }

    let mut user_vec: Vec<U2u> = stats.user_map.iter().map(|(&x, &y)| U2u { count: y.0, size: y.1, uid: x }).collect();
    user_vec.sort_by(|b, a| a.size.cmp(&b.size).then(b.uid.cmp(&b.uid)));
    //println!("File space scanned: {} and {} files in {} seconds", greek(total as f64), count, sec);
    if !user_vec.is_empty() {
        println!("\nSpace/file-count per user");
        for ue in &user_vec {
            #[cfg(target_family = "unix")]
            match get_user_by_uid(ue.uid) {
                None => println!("uid{:7} {} / {}", ue.uid, greek(ue.size as f64), ue.count),
                Some(user) => println!("{:10} {} / {}", user.name().to_string_lossy(), greek(ue.size as f64), ue.count),
            }
            #[cfg(target_family = "windows")]
            println!("uid{:>7} {} / {}", ue.uid, greek(ue.size as f64), ue.count);
        }
    }
    if !stats.top_dir.is_empty() {
        println!("\nTop dir with space usage directly inside them: {}", stats.top_dir.len());
        for v in to_sort_vec(&stats.top_dir) {
            println!("{:>14} {}", greek(v.size as f64), &v.path.display());
        }
    }

    if !stats.top_dir_overall.is_empty() {
        println!("\nTop dir size recursive: {}", stats.top_dir_overall.len());
        for v in to_sort_vec(&stats.top_dir_overall) {
            //let rel = v.path.as_path().strip_prefix(CLI.dir.as_path()).unwrap();
            println!("{:>14} {}", greek(v.size as f64), &v.path.display());
        }
    }
    use num_format::{Locale, ToFormattedString};

    if !stats.top_cnt_overall.is_empty() {
        println!("\nTop count of files recursive: {}", stats.top_cnt_overall.len());
        for v in to_sort_vec(&stats.top_cnt_overall) {
            println!("{:>14} {}", v.size.to_formatted_string(&Locale::en), &v.path.display());
        }
    }

    if !stats.top_cnt_file.is_empty() {
        println!("\nTop counts of files in a single directory: {}", stats.top_cnt_file.len());
        for v in to_sort_vec(&stats.top_cnt_file) {
            println!("{:>14} {}", v.size.to_formatted_string(&Locale::en), &v.path.display());
        }
    }

    if !stats.top_cnt_dir.is_empty() {
        println!("\nTop counts of directories in a single directory: {}", stats.top_cnt_dir.len());
        for v in to_sort_vec(&stats.top_cnt_dir) {
            println!("{:>14} {}", v.size.to_formatted_string(&Locale::en), &v.path.display());
        }
    }
    if !stats.top_files.is_empty() {
        println!("\nTop largest file(s): {}", stats.top_files.len());
        for v in to_sort_vec(&stats.top_files) {
            println!("{:>14} {}", greek(v.size as f64), &v.path.display());
        }
    }
    if !stats.top_ext.is_empty() {
        println!("\nTop usage by file extension: {}", stats.top_ext.len());
        for v in to_sort_vec_file_ext(&stats.top_ext) {
            println!("{:>14} {}", greek(v.size as f64), &v.extension);
        }
    }
}


//noinspection ALL
fn parls() -> Result<()> {
    if APP.verbose > 0 { eprintln!("CLI: {:#?}", *APP); }
    let mut q: WorkerQueue<Option<PathBuf>> = WorkerQueue::new(APP.no_threads, 0);
    let mut oq: WorkerQueue<Option<Vec<(PathBuf, Metadata)>>> = WorkerQueue::new(1, 0);

    let mut allstats = AllStats {
        dtree: BTreeMap::new(),
        extensions: BTreeMap::new(),
        top_files: BinaryHeap::new(),
        top_dir: BinaryHeap::new(),
        top_cnt_dir: BinaryHeap::new(),
        top_cnt_file: BinaryHeap::new(),
        top_cnt_overall: BinaryHeap::new(),
        top_dir_overall: BinaryHeap::new(),
        top_ext: BinaryHeap::new(),
        user_map: BTreeMap::new(),
        total_usage: 0u64,
    };


    let mut tt = ThreadTracker::new();
    let mut main_status = tt.setup_thread("main", "setup");
    main_status.register("registered");

    if let Some(die_dur) = APP.die_in {
        spawn_death_timeout_thread(die_dur, &mut tt);
    };


    q.push(Some(APP.dir.to_path_buf())).with_context(|| format!("Cannot push top path: {}", APP.dir.display()))?;
    let startout = Instant::now();
    let startcpu = ProcessTime::now();

    let mut handles = vec![];
    for _i in 0..APP.no_threads {
        let mut q = q.clone();
        let mut oq = oq.clone();
        let mut t_status = tt.setup_thread("read_dir", "starting...");
        //let mut c_t_status = t_status.clone();
        let h = spawn(move || read_dir_thread(&mut q, &mut oq, &mut t_status));
        handles.push(h);
    }

    let w_h = {
        let mut c_oq = oq.clone();
        let mut c_q = q.clone();
        let mut ft_status = tt.setup_thread("file_trk", "starting...");
        //let mut c_ft_status = ft_status.clone();
        spawn(move || file_track(startout, startcpu, &mut allstats, &mut c_oq, &mut c_q, &mut ft_status))
    };

    main_status.set_state("monitor started");
    if APP.t_status_on_key || APP.t_status_interval {
        thread::spawn(move || {
            if APP.t_status_on_key {
                tt.monitor_on_enter();
            } else if APP.t_status_interval {
                tt.monitor(APP.ticker_interval);
            }
        });
    }


    match (APP.list_files, APP.usage_mode) {
        (true, true) => println!("List file stats and disk usage summary for: {}", APP.dir.display()),
        (false, true) => println!("Scanning disk usage summary for: {}", APP.dir.display()),
        (true, false) => println!("List file stats under: {}", APP.dir.display()),
        _ => Err(anyhow!("Error - neither usage or list mode specified"))?,
    }
    main_status.set_state("wait on queue finish");

    loop {
        let x = q.wait_for_finish_timeout(Duration::from_millis(250))?;
        if x != -1 { break; }
        if APP.verbose > 0 { q.status() };
    }
    if APP.verbose > 0 { q.print_max_queue(); }
    if APP.verbose > 0 { eprintln!("finished so sends the Nones and join"); }
    main_status.set_state("joining");
    for _ in 0..APP.no_threads { q.push(None)?; }
    for h in handles {
        h.join().expect("Cannot join a readdir thread");
    }
    main_status.set_state("output queue wait");
    if APP.verbose > 0 { eprintln!("waiting on out finish"); }
    oq.wait_for_finish()?;
    if APP.verbose > 0 { eprintln!("push none of out queue"); }
    oq.push(None)?;
    if APP.verbose > 0 { eprintln!("joining out thread"); }
    w_h.join().expect("cannot join a output thread")?;


    println!("last cpu time: {}", startcpu.elapsed().as_secs_f32());
    Ok(())
}

