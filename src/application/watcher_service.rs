use crate::adapters::fs::inotify::{FsChangeEvent, LinuxInotifyWatcher};
use crate::application::sync_service::{rewrite_compat_from_db, SyncService};
use crate::config::AppConfig;
use crate::error::Result;
use crate::ports::repository::IndexRepository;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(350);
const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);

pub struct WatcherService<'a> {
    cfg: &'a AppConfig,
    repo: &'a dyn IndexRepository,
    sync_service: &'a SyncService<'a>,
    watcher: &'a mut LinuxInotifyWatcher,
    spawn_images: bool,
}

impl<'a> WatcherService<'a> {
    pub fn new(
        cfg: &'a AppConfig,
        repo: &'a dyn IndexRepository,
        sync_service: &'a SyncService<'a>,
        watcher: &'a mut LinuxInotifyWatcher,
        spawn_images: bool,
    ) -> Self {
        Self {
            cfg,
            repo,
            sync_service,
            watcher,
            spawn_images,
        }
    }

    pub fn serve_forever(&mut self, running: Arc<AtomicBool>) -> Result<()> {
        // Initial startup sync ensures DB & inode_map match disk.
        self.sync_service.sync(self.spawn_images)?;
        let mut last_reconcile_at = Instant::now();

        let mut pending_renames: Vec<(PathBuf, PathBuf, bool)> = Vec::new();
        let mut pending_deletes: HashSet<(PathBuf, bool)> = HashSet::new();
        let mut has_dirty = false;
        let mut last_event_at: Option<Instant> = None;

        while running.load(Ordering::SeqCst) {
            let events = self.watcher.poll_events(150)?;

            // inotify is a fast path, not a source of truth. Reconcile periodically to
            // recover from queue overflow, missed events, unavailable roots at startup,
            // and directories created while the daemon was not watching them.
            if last_reconcile_at.elapsed() >= RECONCILE_INTERVAL {
                self.watcher.reconcile_watches();
                self.sync_service.sync(self.spawn_images)?;
                last_reconcile_at = Instant::now();
            }
            if !events.is_empty() {
                last_event_at = Some(Instant::now());
                for ev in events {
                    match ev {
                        FsChangeEvent::Renamed {
                            old_path,
                            new_path,
                            is_dir,
                        } => {
                            pending_renames.push((old_path, new_path, is_dir));
                        }
                        FsChangeEvent::Deleted { path, is_dir } => {
                            pending_deletes.insert((path, is_dir));
                        }
                        FsChangeEvent::Modified | FsChangeEvent::Overflow => {
                            has_dirty = true;
                        }
                    }
                }
            }

            let ready = last_event_at
                .map(|t| t.elapsed() >= DEBOUNCE_WINDOW)
                .unwrap_or(false)
                && !self.watcher.has_pending_cookies();

            if ready && (!pending_renames.is_empty() || !pending_deletes.is_empty() || has_dirty) {
                let mut mutated = false;

                for (target, is_dir) in pending_deletes.drain() {
                    let p_str = target.to_string_lossy();
                    if self.repo.delete_path(&p_str, is_dir)? > 0 {
                        mutated = true;
                    }
                }

                for (old_p, new_p, is_dir) in pending_renames.drain(..) {
                    let old_s = old_p.to_string_lossy();
                    let new_s = new_p.to_string_lossy();
                    if self.repo.rename_path(&old_s, &new_s, is_dir)? > 0 {
                        mutated = true;
                    }
                }

                if has_dirty || mutated {
                    // `sync` updates filename TF-IDF weights for renamed files, indexes created/modified
                    // files, rebuilds sparse IDF/postings if needed, and updates `file-index.txt` + `index.rev`.
                    self.sync_service.sync(self.spawn_images)?;
                } else {
                    rewrite_compat_from_db(&self.cfg.cache_txt, &self.cfg.rev_file, self.repo)?;
                }

                has_dirty = false;
                last_event_at = None;
            }
        }

        Ok(())
    }
}
