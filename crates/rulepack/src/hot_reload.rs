// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-141 hot-reloadable rule packs.
//!
//! [`HotReloadRegistry`] wraps a [`Registry`] behind an
//! [`ArcSwap<Registry>`]: readers pull a [`Snapshot`] (a cheap
//! `Arc<Registry>` clone) and hold it for the lifetime of their
//! lookup. Reloading replaces the inner pointer atomically; in-flight
//! readers continue to use the prior snapshot. No `RwLock`, no reader
//! starvation, no transmission interruption.
//!
//! [`HotReloadRegistry::spawn_watcher`] starts a background thread
//! that listens for filesystem events under the rule-pack directory
//! and calls [`HotReloadRegistry::reload_now`] on every change. The
//! watcher uses the cross-platform `notify` crate (inotify on Linux,
//! `FSEvents` on macOS, `ReadDirectoryChangesW` on Windows, polling
//! fallback). Tests pin behavior with `reload_now` directly to keep
//! them deterministic without depending on OS event timing.
//!
//! Failure isolation: a reload that produces an invalid manifest set
//! (bad signature, malformed JSON, anything that
//! [`Registry::insert`] rejects) leaves the previous snapshot in
//! place and surfaces a [`HotReloadError`]. Callers in long-running
//! services should log the error and continue serving from the
//! known-good snapshot.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{event::EventKind, Event, RecursiveMode, Watcher};
use thiserror::Error;

use crate::{Manifest, Registry, RulepackError};

/// Cheap read-only snapshot of the current rule-pack registry.
///
/// Holding a `Snapshot` keeps the snapshot's [`Registry`] alive for
/// the duration of the borrow but does not block reloads.
pub type Snapshot = Arc<Registry>;

/// Errors raised by [`HotReloadRegistry`].
#[derive(Debug, Error)]
pub enum HotReloadError {
    /// The rule-pack directory could not be enumerated.
    #[error("failed to read rule-pack directory {path}: {detail}")]
    ReadDir {
        /// Path that was attempted.
        path: PathBuf,
        /// Operator-readable I/O reason.
        detail: String,
    },
    /// A specific manifest file failed to load or verify.
    #[error("rule-pack manifest {path} rejected: {source}")]
    Manifest {
        /// Path to the offending JSON file.
        path: PathBuf,
        /// Underlying [`RulepackError`] from [`Manifest::from_json`].
        #[source]
        source: Box<RulepackError>,
    },
    /// The notify watcher failed to start.
    #[error("failed to start filesystem watcher on {path}: {detail}")]
    Watcher {
        /// Path that was being watched.
        path: PathBuf,
        /// Operator-readable reason.
        detail: String,
    },
}

/// Atomically-swappable, file-backed rule-pack registry.
///
/// Snapshots are cheap (`Arc` clone); reloads are
/// non-blocking-for-readers (`ArcSwap::store`); a malformed reload
/// leaves the previous snapshot in place.
pub struct HotReloadRegistry {
    inner: ArcSwap<Registry>,
    dir: PathBuf,
}

impl HotReloadRegistry {
    /// Load every `*.json` file under `dir` into a fresh registry.
    ///
    /// # Errors
    ///
    /// Returns [`HotReloadError::ReadDir`] when the directory can't be
    /// enumerated and [`HotReloadError::Manifest`] when any single
    /// file is malformed or fails signature verification.
    pub fn load_from_dir(dir: impl Into<PathBuf>) -> Result<Arc<Self>, HotReloadError> {
        let dir = dir.into();
        let registry = read_registry(&dir)?;
        Ok(Arc::new(Self {
            inner: ArcSwap::from(Arc::new(registry)),
            dir,
        }))
    }

    /// Cheap, lock-free read of the current registry.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.inner.load_full()
    }

    /// Reload from disk and atomically swap if the new set verifies.
    /// Leaves the previous snapshot in place on failure.
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::load_from_dir`].
    pub fn reload_now(&self) -> Result<(), HotReloadError> {
        let next = read_registry(&self.dir)?;
        self.inner.store(Arc::new(next));
        Ok(())
    }

    /// The directory this registry watches.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Number of currently-loaded manifests (cheap; goes through the snapshot).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.load().manifests_len()
    }

    /// True when no manifests are currently loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.load().manifests_is_empty()
    }

    /// Start a background thread that watches `self.dir` for filesystem
    /// changes and calls [`Self::reload_now`] on every relevant event.
    ///
    /// Returns a [`WatchHandle`]; dropping the handle stops the
    /// watcher thread cleanly. The watcher coalesces a burst of
    /// events into one reload via a short debounce window.
    ///
    /// # Errors
    ///
    /// Returns [`HotReloadError::Watcher`] when the OS watcher cannot
    /// be initialized.
    pub fn spawn_watcher(self: Arc<Self>) -> Result<WatchHandle, HotReloadError> {
        self.spawn_watcher_with_debounce(Duration::from_millis(50))
    }

    /// Like [`Self::spawn_watcher`] but lets tests pin the debounce
    /// window so they aren't racing the OS event clock.
    ///
    /// # Errors
    ///
    /// Returns [`HotReloadError::Watcher`] when the OS watcher cannot
    /// be initialized.
    pub fn spawn_watcher_with_debounce(
        self: Arc<Self>,
        debounce: Duration,
    ) -> Result<WatchHandle, HotReloadError> {
        let (event_tx, event_rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = notify::recommended_watcher(move |res| {
            // Channel close means the WatchHandle was dropped; nothing to do.
            let _ = event_tx.send(res);
        })
        .map_err(|e| HotReloadError::Watcher {
            path: self.dir.clone(),
            detail: e.to_string(),
        })?;
        watcher
            .watch(&self.dir, RecursiveMode::NonRecursive)
            .map_err(|e| HotReloadError::Watcher {
                path: self.dir.clone(),
                detail: e.to_string(),
            })?;

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let registry = Arc::clone(&self);
        let join = std::thread::Builder::new()
            .name("rulepack-hot-reload".into())
            .spawn(move || {
                // Keep `watcher` alive for the whole thread lifetime; the
                // notify backend stops emitting events as soon as it drops.
                let _keep_alive = watcher;
                let mut pending = false;
                loop {
                    let wait = if pending {
                        debounce
                    } else {
                        Duration::from_secs(30)
                    };
                    match event_rx.recv_timeout(wait) {
                        Ok(Ok(event)) if is_relevant(&event) => {
                            pending = true;
                        }
                        Ok(_) => { /* error or irrelevant event; ignore */ }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if pending {
                                pending = false;
                                // Reload errors are logged-by-caller; the
                                // watcher thread keeps running.
                                let _ = registry.reload_now();
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                }
            })
            .map_err(|e| HotReloadError::Watcher {
                path: self.dir.clone(),
                detail: e.to_string(),
            })?;
        Ok(WatchHandle {
            join: Some(join),
            stop_tx,
        })
    }
}

impl std::fmt::Debug for HotReloadRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `inner` is an ArcSwap<Registry> with no useful Debug impl
        // and would dump every loaded manifest verbatim; for an
        // operational log line the directory + manifest count is the
        // useful summary.
        f.debug_struct("HotReloadRegistry")
            .field("dir", &self.dir)
            .field("loaded", &self.len())
            .finish_non_exhaustive()
    }
}

/// Handle to a running hot-reload watcher thread. Dropping the
/// handle signals the thread to stop after its next event.
pub struct WatchHandle {
    join: Option<JoinHandle<()>>,
    stop_tx: mpsc::Sender<()>,
}

impl WatchHandle {
    /// Stop the watcher thread and wait for it to exit.
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(j) = self.join.take() {
            // Best-effort: don't wedge a dropping caller if the thread
            // is stuck on an OS event. Two seconds is well above the
            // debounce.
            let (done_tx, done_rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = j.join();
                let _ = done_tx.send(());
            });
            let _ = done_rx.recv_timeout(Duration::from_secs(2));
        }
    }
}

fn read_registry(dir: &Path) -> Result<Registry, HotReloadError> {
    let entries = fs::read_dir(dir).map_err(|e| HotReloadError::ReadDir {
        path: dir.to_owned(),
        detail: e.to_string(),
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| HotReloadError::ReadDir {
            path: dir.to_owned(),
            detail: e.to_string(),
        })?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
    // Sort so the registry order is deterministic regardless of FS order.
    files.sort();
    let mut registry = Registry::default();
    for path in files {
        let raw = fs::read_to_string(&path).map_err(|e| HotReloadError::ReadDir {
            path: dir.to_owned(),
            detail: e.to_string(),
        })?;
        let manifest = Manifest::from_json(&raw).map_err(|e| HotReloadError::Manifest {
            path: path.clone(),
            source: Box::new(e),
        })?;
        registry
            .insert(manifest)
            .map_err(|e| HotReloadError::Manifest {
                path,
                source: Box::new(e),
            })?;
    }
    Ok(registry)
}

fn is_relevant(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
    ) && event.paths.iter().any(|p| {
        p.extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    })
}

// Lightweight visibility shims for snapshot inspection. Re-exported
// from the crate root via the `pub use` line in lib.rs.
impl Registry {
    /// Number of currently-loaded manifests (used by [`HotReloadRegistry`]).
    #[must_use]
    pub fn manifests_len(&self) -> usize {
        self.len()
    }

    /// Whether the registry currently holds zero manifests.
    #[must_use]
    pub fn manifests_is_empty(&self) -> bool {
        self.is_empty()
    }

    /// Iterate over rule-pack identifiers in insertion order.
    pub fn rulepack_ids(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|m| m.rulepack_id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tempfile::TempDir;

    const SEED_EN16931: &str = include_str!("../data/en16931-cen-2024.json");
    const SEED_PEPPOL: &str = include_str!("../data/peppol-bis-3-openpeppol-2024.json");
    const SEED_XRECHNUNG: &str = include_str!("../data/xrechnung-kosit-2024.json");

    fn write_pack(dir: &Path, name: &str, raw: &str) {
        fs::write(dir.join(name), raw).expect("write rule pack");
    }

    #[test]
    fn load_from_dir_loads_every_manifest() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "en16931.json", SEED_EN16931);
        write_pack(tmp.path(), "peppol.json", SEED_PEPPOL);
        write_pack(tmp.path(), "xrechnung.json", SEED_XRECHNUNG);
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        let snap = registry.snapshot();
        let count = snap.rulepack_ids().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn load_from_dir_ignores_non_json() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "en16931.json", SEED_EN16931);
        fs::write(tmp.path().join("README.md"), "ignore me").unwrap();
        fs::write(tmp.path().join("notes.txt"), "ignore me too").unwrap();
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn load_from_dir_rejects_bad_signature() {
        let tmp = TempDir::new().unwrap();
        // Tamper with the EN 16931 manifest's body so the BLAKE3
        // signature stops matching.
        let mut tampered: serde_json::Value = serde_json::from_str(SEED_EN16931).unwrap();
        tampered["body"]["rules"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id":"BR-FAKE","text":"injected"}));
        let raw = serde_json::to_string(&tampered).unwrap();
        write_pack(tmp.path(), "bad.json", &raw);
        let err = HotReloadRegistry::load_from_dir(tmp.path()).unwrap_err();
        assert!(matches!(err, HotReloadError::Manifest { .. }));
    }

    #[test]
    fn snapshot_is_immutable_across_reload() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "a.json", SEED_EN16931);
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        let before = registry.snapshot();
        let before_len = before.manifests_len();
        write_pack(tmp.path(), "b.json", SEED_PEPPOL);
        registry.reload_now().unwrap();
        let after = registry.snapshot();
        assert_eq!(before.manifests_len(), before_len, "old snapshot unchanged");
        assert_eq!(after.manifests_len(), before_len + 1, "new snapshot grew");
    }

    #[test]
    fn reload_now_picks_up_added_and_removed_packs() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "a.json", SEED_EN16931);
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        assert_eq!(registry.len(), 1);

        write_pack(tmp.path(), "b.json", SEED_PEPPOL);
        registry.reload_now().unwrap();
        assert_eq!(registry.len(), 2);

        fs::remove_file(tmp.path().join("a.json")).unwrap();
        registry.reload_now().unwrap();
        assert_eq!(registry.len(), 1);
        let snap = registry.snapshot();
        let first_id = snap.rulepack_ids().next().unwrap().to_owned();
        assert!(
            first_id.contains("peppol"),
            "remaining pack should be peppol; got {first_id:?}"
        );
    }

    #[test]
    fn failed_reload_leaves_prior_snapshot_intact() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "good.json", SEED_EN16931);
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        let before_ids: Vec<String> = registry
            .snapshot()
            .rulepack_ids()
            .map(str::to_owned)
            .collect();
        // Add a malformed pack; reload should fail without overwriting.
        write_pack(tmp.path(), "bad.json", "{ this is not json");
        let err = registry.reload_now().unwrap_err();
        assert!(matches!(err, HotReloadError::Manifest { .. }));
        let after_ids: Vec<String> = registry
            .snapshot()
            .rulepack_ids()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            before_ids, after_ids,
            "failed reload must not change snapshot"
        );
    }

    #[test]
    fn concurrent_readers_do_not_block_reload() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "a.json", SEED_EN16931);
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut readers = Vec::new();
        for _ in 0..4 {
            let r = Arc::clone(&registry);
            let s = Arc::clone(&stop);
            readers.push(std::thread::spawn(move || {
                while !s.load(std::sync::atomic::Ordering::Relaxed) {
                    let snap = r.snapshot();
                    // Touch the data so the optimizer doesn't elide the load.
                    let _ids: Vec<&str> = snap.rulepack_ids().collect();
                }
            }));
        }
        // Now perform a bunch of reloads while readers spin. If any
        // reader blocked a reload, this test would deadlock; the test
        // harness's per-test timeout would catch it.
        for cycle in 0..50 {
            let extra = tmp.path().join(format!("extra-{cycle}.json"));
            fs::write(&extra, SEED_PEPPOL).unwrap();
            registry.reload_now().unwrap();
            fs::remove_file(&extra).unwrap();
            registry.reload_now().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for r in readers {
            r.join().unwrap();
        }
    }

    #[test]
    fn watcher_thread_picks_up_filesystem_changes() {
        let tmp = TempDir::new().unwrap();
        write_pack(tmp.path(), "a.json", SEED_EN16931);
        let registry = HotReloadRegistry::load_from_dir(tmp.path()).unwrap();
        let handle = Arc::clone(&registry)
            .spawn_watcher_with_debounce(Duration::from_millis(20))
            .unwrap();

        // Add a second pack; the watcher should pick it up within a
        // reasonable window (debounce + OS event lag).
        write_pack(tmp.path(), "b.json", SEED_PEPPOL);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if registry.len() == 2 {
                break;
            }
            if Instant::now() > deadline {
                handle.stop();
                panic!("watcher did not pick up new pack within deadline");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        handle.stop();
    }
}
