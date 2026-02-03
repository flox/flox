# Plan: Replace PID Polling with waitpid_any Crate

## Setup: Create Git Worktree

```bash
cd /flox/me
git worktree add ../waitpid-any-refactor origin/main
cd ../waitpid-any-refactor
git cherry-pick <devShell-fix-commit>  # Get last commit from devShell-fix branch
```

Then run `nix develop` in the worktree before making changes.

**First step**: Commit this plan file to the worktree for reference during implementation.

## Summary

Replace the 100ms polling loop in the executive with event-driven process monitoring using the `waitpid_any` crate. Each monitored PID gets a dedicated thread that:
- **With expiration**: Sleeps until expiration, then waits for process exit
- **Without expiration**: Immediately waits for process exit

## Files to Modify

| File | Changes |
|------|---------|
| `cli/flox-activations/Cargo.toml` | Add `waitpid-any = "0.3.0"` and `notify = "6"` dependencies |
| `cli/flox-activations/src/cli/executive/mod.rs` | Update `run_monitoring_loop()` to use channel-based events |
| `cli/flox-activations/src/cli/executive/pid_monitor.rs` | **NEW** - PID monitoring coordinator and thread spawning |
| `cli/flox-activations/src/cli/executive/watcher.rs` | Simplify - remove polling, keep state file update logic |
| `cli/flox-core/src/activations.rs` | Add `all_attached_pids_with_expiration()` helper method |

## Architecture

```
Main Thread (Coordinator)
    |
    +---> blocking recv() on event channel
    |
    +---> PidEvent::ProcessExited { pid }      --> update state.json, check if all done
    +---> PidEvent::TerminationSignal          --> bail and exit
    +---> PidEvent::SigChld                    --> reap orphaned children
    +---> PidEvent::StartServices              --> start process-compose (SIGUSR1)

Event Sources:
- PID watcher threads (one per monitored PID) using waitpid_any
- File watcher using notify crate (watches state.json, spawns new PID watchers directly)
- Signal handler thread (SIGINT/SIGTERM/SIGQUIT/SIGCHLD/SIGUSR1)
```

## Implementation Steps

### 1. Add dependencies to `cli/flox-activations/Cargo.toml`
```toml
waitpid-any = "0.3.0"
notify = "6"
```

### 2. Add helper method to `cli/flox-core/src/activations.rs`

```rust
/// Returns all attached PIDs with their expirations, flattened from all start IDs
pub fn all_attached_pids_with_expiration(&self) -> Vec<(i32, Option<OffsetDateTime>)> {
    self.attached_pids
        .iter()
        .map(|(pid, attachment)| (pid.0, attachment.expiration))
        .collect()
}
```

### 3. Create `cli/flox-activations/src/cli/executive/pid_monitor.rs`

**PidEvent enum:**
```rust
pub enum PidEvent {
    ProcessExited { pid: i32 },
    TerminationSignal,  // SIGINT/SIGTERM/SIGQUIT
    SigChld,            // Child process needs reaping
    StartServices,      // SIGUSR1 - start process-compose
}
```

**PidMonitorCoordinator struct:**
```rust
pub struct PidMonitorCoordinator {
    sender: Sender<PidEvent>,
    pub receiver: Receiver<PidEvent>,
    known_pids: Arc<Mutex<HashSet<i32>>>,
}
```

**PID watcher thread logic:**
1. If expiration exists and not yet passed: `thread::sleep()` until expiration
2. Open `WaitHandle::open(pid)` - if fails (ESRCH, process already dead), send `ProcessExited`
3. Call `wait()` blocking until process exits, retry on EINTR
4. Send `ProcessExited` event

### 4. State file watcher using `notify` crate

Use filesystem watcher (inotify on Linux, kqueue on macOS) to detect state.json changes.

**Note**: Watch state.json directly. Although atomic writes use temp file + rename, the only time state.json is renamed/removed is when the executive exits anyway, so we don't need to worry about losing the inode.

```rust
// Watch state.json directly
watcher.watch(&state_json_path, RecursiveMode::NonRecursive)?;

// In the callback
if event.kind.is_modify() {
    // Re-read state.json and spawn watchers for new PIDs
}
```

- On modify events, re-read state.json
- Spawn watchers for any new PIDs not in `known_pids`
- Track `known_pids` in `Arc<Mutex<HashSet<i32>>>`

### 5. Signal handler thread

Spawn a thread using `signal_hook::iterator::Signals`:
- SIGINT/SIGTERM/SIGQUIT → send `PidEvent::TerminationSignal`
- SIGCHLD → send `PidEvent::SigChld`
- SIGUSR1 → send `PidEvent::StartServices`

### 6. Update `run_monitoring_loop()` in `mod.rs`

Replace the polling loop with blocking `recv()`:

```rust
fn run_monitoring_loop(...) -> Result<()> {
    let mut coordinator = PidMonitorCoordinator::new()?;

    // Start monitoring existing PIDs from state.json
    for (pid, expiration) in activations.all_attached_pids_with_expiration() {
        coordinator.start_monitoring(pid, expiration)?;
    }

    // Start file watcher for state.json changes
    let _watcher = coordinator.start_state_watcher(state_json_path);

    // Start signal handler thread
    let _signal_handler = coordinator.start_signal_handler();

    loop {
        match coordinator.receiver.recv() {
            Ok(PidEvent::ProcessExited { pid }) => {
                // Update state.json (detach pid), check if all PIDs gone
                if all_terminated { return cleanup_all(...); }
            }
            Ok(PidEvent::TerminationSignal) => {
                bail!("received stop signal, exiting without cleanup");
            }
            Ok(PidEvent::SigChld) => {
                reap_orphaned_children();
            }
            Ok(PidEvent::StartServices) => {
                handle_start_services_signal(...)?;
            }
            Err(_) => bail!("event channel disconnected"),
        }
    }
}
```

### 7. Simplify `watcher.rs`

- Remove polling logic from `cleanup_pids()`
- Keep methods for updating state.json (detach single PID, check if empty)
- The `PidWatcher` struct may be simplified or merged into coordinator

## Error Handling

| Scenario | Handling |
|----------|----------|
| Process exits before `WaitHandle::open()` | Returns ESRCH → treat as exited |
| EINTR during wait | Retry in a loop |
| PID reuse race | `waitpid_any` locks onto process entity, not just PID number |
| PID removed from state while watcher running | `ProcessExited` event tries to detach; if already gone, no-op |
| File watcher misses event | Acceptable - eventual consistency via ProcessExited events |

## Testing

**Existing tests should continue to work:**
- `monitoring_loop_removes_state_on_cleanup`
- `monitoring_loop_bails_on_termination_signal`

**New tests to add:**
- PID watcher thread with quick-exiting process
- PID watcher with expiration (sleep then wait)
- Handle open failure for non-existent PID
- New PID added to state.json triggers watcher spawn

## Migration Notes

- The subreaper functionality (Linux) remains unchanged
- Background log GC threads remain unchanged
- The `handle_start_services_signal()` function is used as-is
