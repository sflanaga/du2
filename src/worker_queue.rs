#![allow(dead_code)]
#![allow(unused_imports)]

use std::collections::LinkedList;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};

struct InnerQ<T> {
    queue: LinkedList<T>,
    curr_poppers: usize,
    curr_pushers: usize,
    max_waiters: usize,
    dead: usize,
    max_q_len_reached: usize,
    limit: usize,
}

#[derive(Clone)]
pub struct WorkerQueue<T> {
    tqueue: Arc<Mutex<InnerQ<T>>>,
    cond_more: Arc<Condvar>,
    cond_hasroom: Arc<Condvar>,
    looks_done: Arc<Condvar>,
}

#[derive(Debug)]
pub struct QueueStats {
    pub curr_poppers: usize,
    pub curr_pushers: usize,
    pub curr_q_len: usize,
}
//
// unbounded queue (aka limit of 0) is known to work well - much simpler
//
impl<T> WorkerQueue<T> {
    pub fn new(max_waits: usize, limit_: usize) -> WorkerQueue<T> {
        WorkerQueue {
            tqueue: Arc::new(Mutex::new(
                InnerQ {
                    queue: LinkedList::new(),
                    curr_poppers: 0,
                    curr_pushers: 0,
                    dead: 0,
                    max_waiters: max_waits,
                    max_q_len_reached: 0,
                    limit: limit_,
                })),
            cond_more: Arc::new(Condvar::new()),
            cond_hasroom: Arc::new(Condvar::new()),
            looks_done: Arc::new(Condvar::new()),
        }
    }
    pub fn push(&mut self, item: T) -> Result<()> {
        let mut lck_q = self.tqueue.lock().unwrap();
        lck_q.curr_pushers += 1;
        if lck_q.queue.len() > lck_q.max_q_len_reached {
            lck_q.max_q_len_reached = lck_q.queue.len();
        }
        if lck_q.limit == 0 {
            lck_q.queue.push_front(item);
        } else {
            if lck_q.limit <= lck_q.queue.len() && lck_q.curr_pushers >= lck_q.max_waiters {
                lck_q.dead += 1;
                self.looks_done.notify_all();
                Err(anyhow!("Queue overflow reached - cannot push another, len at {} and this is the {}(th) pusher",
                lck_q.queue.len(), lck_q.curr_pushers))?;
            }
            while lck_q.limit > 0 && lck_q.queue.len() >= lck_q.limit {
                lck_q = self.cond_hasroom.wait(lck_q).unwrap();
            }
            lck_q.queue.push_front(item);
        }
        lck_q.curr_pushers -= 1;
        self.cond_more.notify_one();
        Ok(())
    }
    pub fn pop(&mut self) -> T {
        let mut lck_q = self.tqueue.lock().unwrap();
        lck_q.curr_poppers += 1;
        while lck_q.queue.len() < 1 {
            if lck_q.curr_poppers == lck_q.max_waiters {
                self.looks_done.notify_one();
            }
            lck_q = self.cond_more.wait(lck_q).unwrap();
        }
        let res = lck_q.queue.pop_back().unwrap();
        self.cond_hasroom.notify_one();
        lck_q.curr_poppers -= 1;
        res
    }
    pub fn waiters(&self) -> usize {
        let lck_q = self.tqueue.lock().unwrap();
        lck_q.curr_poppers
    }

    pub fn wait_for_finish_timeout(&self, dur: Duration) -> Result<i64> {
        let ret = {
            let mut lck_q = self.tqueue.lock().unwrap();
            // sanity check because we have more new work than the queue can hold
            while !(lck_q.queue.len() <= 0 && lck_q.curr_poppers == lck_q.max_waiters) {
                let x = self.looks_done.wait_timeout(lck_q, dur).unwrap();
                lck_q = x.0;
                if x.1.timed_out() {
                    return Ok(-1);
                }
                if lck_q.limit != 0 && lck_q.curr_pushers >= lck_q.max_waiters && lck_q.queue.len() >= (lck_q.limit) {
                    Err(anyhow!("Queue looks stuck at limit {} and pushers {}", &lck_q.queue.len(), &lck_q.curr_pushers))?;
                }
                if lck_q.dead > 0 {
                    Err(anyhow!("Thread death detected - likely due to overflow, #dead: {}", &lck_q.dead))?;
                }
            }
            lck_q.curr_poppers as i64
        };

        Ok(ret)
    }

    pub fn wait_for_finish(&self) -> Result<usize> {
        let mut lck_q = self.tqueue.lock().unwrap();
        // sanity check because we have more new work than the queue can hold
        if lck_q.limit > 0 && lck_q.curr_pushers >= lck_q.max_waiters && lck_q.queue.len() >= lck_q.limit {
            Err(anyhow!("Queue looks stuck at limit {} and waiters {}", &lck_q.queue.len(), &lck_q.curr_poppers))?;
        }
        while !(lck_q.queue.len() <= 0 && lck_q.curr_poppers >= lck_q.max_waiters) {
            lck_q = self.looks_done.wait(lck_q).unwrap();
        }
        Ok(lck_q.curr_poppers)
    }

    pub fn notify_all(&self) {
        self.cond_hasroom.notify_all();
        self.cond_more.notify_all();
    }

    pub fn status(&self) {
        let lck_q = self.tqueue.lock().unwrap();
        eprintln!("q len: {}  threads:  {}  dead:  {}  poppers: {}  pushers: {}",
                  lck_q.queue.len(), lck_q.max_waiters, lck_q.dead,
                  lck_q.curr_poppers, lck_q.curr_pushers);
    }
    pub fn print_max_queue(&self) {
        let lck_q = self.tqueue.lock().unwrap();
        eprintln!("max q reached: {}", lck_q.max_q_len_reached);
    }

    pub fn get_stats(&self) -> QueueStats {
        let lck_q = self.tqueue.lock().unwrap();
        QueueStats {
            curr_pushers: lck_q.curr_pushers,
            curr_poppers: lck_q.curr_poppers,
            curr_q_len: lck_q.queue.len(),
        }
    }
}
