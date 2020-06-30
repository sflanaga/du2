
du2 0.2.1
Fast parallel file system lister / usage statistics summary

Latency vs throughput: The theory here is that parallel listing overcomes latency issues on remote files systems by
having multiple requests in play at once.  Usually remote file systems capable of good throughput will have higher
latency than local file systems largely because the OS owns a faster and exclusive cache to local file system metadata.

Each "opendir" is finished to completion so that the directory to minimize open time, but this costs more memory than
straight recursion. This might also contribute to better performance as it may reduce contention on that remote file
system versus holding the opendir open as you recurse a directory's children.  Sub directories found are queued for
other threads to query to completion, and therefore because the number of directories may be large the queue grows
unbounded.  The queue must be unbounded or a deadlock can occur as the worker is also a master (creator of new work).

Because in this application directories are evaluated in no particular order, it is necessary to aggregate lower
directories up the tree containing ALL directories for usage summaries. This tree is the bulk of the memory used and is
proportional to the tree directory count.

Symbolic links are not followed
```
USAGE:
    du2.exe [OPTIONS] <DIRECTORY>

OPTIONS:
    -d, --delimiter <delimiter>                Disk usage mode - do not write the files found [default: |]
        --die-in <die-in>                      write cpu time consumed by each thread
        --exclude-re <exclude-re>              Exclude FILEs that match this RE
        --file-newer-than <file-newer-than>    Only count/sum entries newer than this age
        --file-older-than <file-older-than>    Only count/sum entries older than this age
    -h, --help                                 Prints help information
    -l                                         Write file list
        --extra                                
    -t, --worker-threads <no-threads>          Number worker threads [default: 0]
        --progress                             Writes progress stats on every ticker interval
        --re <re>                              Keep only FILEs that match this RE
        --write_thread_status                  Writes thread status every ticker interval - used to debug things
        --t-status-on-key                      Writes thread status when stdin sees a line entered by user
    -i, --ticker-interval <ticker-interval>    Interval at which stats are written - 0 means no ticker is run [default:
                                               200]
    -n <top-n-limit>                           Report top usage limit [default: 10]
    -u, --usage-trees                          Write disk usage summary
    -V, --version                              Prints version information
    -v                                         Verbosity - use more than one v for greater detail
        --write-thread-cpu-time                write cpu time consumed by each thread

ARGS:
    <DIRECTORY>    Directory to search

```
