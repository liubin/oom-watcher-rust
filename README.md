# OOM-watcher-rust

A Rust based OOM watcher PoC

Rust PoC of watching OOM events from cgroup.

## How to

### Setup cgroup

Create a new cgroup named `testoom` and set memory to `102400` bytes, this value will lead a simple `date` command to be killed by OOM killer.

```
mkdir -p /sys/fs/cgroup/memory/testoom

echo 102400 > /sys/fs/cgroup/memory/testoom/memory.limit_in_bytes
echo $$ > /sys/fs/cgroup/memory/testoom/tasks
```

### Run oom-watcher-rust

Download source code and build.

```
git clone https://github.com/liubin/oom-watcher-rust.git
cd oom-watcher-rust
cargo build
```

Start `oom-watcher-rust`.

```
target/debug/oom-watcher-rust /sys/fs/cgroup/memory/testoom
```

### Trigger an OOM event

Run `date` will lead it to be killed.

```
date
Killed
```
