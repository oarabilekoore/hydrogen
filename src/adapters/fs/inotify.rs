use crate::adapters::fs::scanner::should_skip_name;
use crate::error::{HydrogenError, Result};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

const WATCH_MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_DONT_FOLLOW
    | libc::IN_ONLYDIR;

const COOKIE_TTL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub enum FsChangeEvent {
    Renamed {
        old_path: PathBuf,
        new_path: PathBuf,
        is_dir: bool,
    },
    Deleted {
        path: PathBuf,
        is_dir: bool,
    },
    Modified,
    Overflow,
}

pub struct LinuxInotifyWatcher {
    fd: i32,
    roots: Vec<PathBuf>,
    wd_to_path: HashMap<i32, PathBuf>,
    path_to_wd: HashMap<PathBuf, i32>,
    pending_cookies: HashMap<u32, (Instant, PathBuf, bool)>,
}

impl LinuxInotifyWatcher {
    pub fn new(roots: Vec<PathBuf>) -> Result<Self> {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(HydrogenError::Io(std::io::Error::last_os_error()));
        }

        let mut watcher = Self {
            fd,
            roots: roots.clone(),
            wd_to_path: HashMap::new(),
            path_to_wd: HashMap::new(),
            pending_cookies: HashMap::new(),
        };

        for root in &roots {
            watcher.add_watch_tree(root);
        }
        Ok(watcher)
    }

    pub fn has_pending_cookies(&self) -> bool {
        !self.pending_cookies.is_empty()
    }

    fn is_under_ignored(&self, path: &Path) -> bool {
        for root in &self.roots {
            if let Ok(rel) = path.strip_prefix(root) {
                return rel.components().any(|c| {
                    c.as_os_str()
                        .to_str()
                        .map(should_skip_name)
                        .unwrap_or(true)
                });
            }
        }
        true
    }

    fn add_watch_dir(&mut self, dir: &Path) {
        if self.path_to_wd.contains_key(dir) || self.is_under_ignored(dir) {
            return;
        }
        let Ok(c_path) = CString::new(dir.as_os_str().as_bytes()) else {
            return;
        };
        let wd = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), WATCH_MASK) };
        if wd >= 0 {
            self.wd_to_path.insert(wd, dir.to_path_buf());
            self.path_to_wd.insert(dir.to_path_buf(), wd);
        }
    }

    fn add_watch_tree(&mut self, root: &Path) {
        if !root.is_dir() || self.is_under_ignored(root) {
            return;
        }
        let iter = WalkDir::new(root)
            .max_depth(6)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                e.file_type().is_dir()
                    && (e.depth() == 0
                        || e.file_name()
                            .to_str()
                            .map(|n| !should_skip_name(n))
                            .unwrap_or(false))
            });

        for entry in iter.flatten() {
            self.add_watch_dir(entry.path());
        }
    }

    fn remove_watch_tree(&mut self, prefix: &Path) {
        let to_drop: Vec<(PathBuf, i32)> = self
            .path_to_wd
            .iter()
            .filter(|(p, _)| *p == prefix || p.starts_with(prefix))
            .map(|(p, &wd)| (p.clone(), wd))
            .collect();

        for (p, wd) in to_drop {
            self.path_to_wd.remove(&p);
            self.wd_to_path.remove(&wd);
            unsafe {
                libc::inotify_rm_watch(self.fd, wd);
            }
        }
    }

    fn rekey_watch_tree(&mut self, old_dir: &Path, new_dir: &Path) {
        let updates: Vec<(PathBuf, i32)> = self
            .path_to_wd
            .iter()
            .filter(|(p, _)| *p == old_dir || p.starts_with(old_dir))
            .map(|(p, &wd)| (p.clone(), wd))
            .collect();

        for (p, wd) in updates {
            if let Ok(rel) = p.strip_prefix(old_dir) {
                let updated = if rel.as_os_str().is_empty() {
                    new_dir.to_path_buf()
                } else {
                    new_dir.join(rel)
                };
                self.path_to_wd.remove(&p);
                self.path_to_wd.insert(updated.clone(), wd);
                self.wd_to_path.insert(wd, updated);
            }
        }
    }

    pub fn poll_events(&mut self, timeout_ms: i32) -> Result<Vec<FsChangeEvent>> {
        let mut pfd = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let poll_rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if poll_rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(HydrogenError::Io(err));
            }
        }

        let mut out = Vec::new();
        let mut buf = [0u8; 65536];

        if poll_rc > 0 && (pfd.revents & libc::POLLIN) != 0 {
            loop {
                let n = unsafe {
                    libc::read(
                        self.fd,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if n <= 0 {
                    break;
                }
                let bytes = &buf[..n as usize];
                let mut offset = 0usize;

                while offset + 16 <= bytes.len() {
                    let wd = i32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
                    let mask = u32::from_ne_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
                    let cookie =
                        u32::from_ne_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
                    let name_len =
                        u32::from_ne_bytes(bytes[offset + 12..offset + 16].try_into().unwrap())
                            as usize;
                    offset += 16;

                    let name_end = (offset + name_len).min(bytes.len());
                    let name_slice = &bytes[offset..name_end];
                    offset = name_end;

                    if (mask & libc::IN_Q_OVERFLOW) != 0 {
                        out.push(FsChangeEvent::Overflow);
                        continue;
                    }

                    if (mask & libc::IN_IGNORED) != 0 {
                        if let Some(old_p) = self.wd_to_path.remove(&wd) {
                            self.path_to_wd.remove(&old_p);
                        }
                        continue;
                    }

                    let Some(base) = self.wd_to_path.get(&wd).cloned() else {
                        continue;
                    };

                    let name = if name_slice.is_empty() || name_slice[0] == 0 {
                        String::new()
                    } else {
                        CStr::from_bytes_until_nul(name_slice)
                            .map(|c| c.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    };

                    let is_dir = (mask & libc::IN_ISDIR) != 0;
                    if !name.is_empty() && should_skip_name(&name) && (mask & libc::IN_MOVED_TO) == 0
                    {
                        continue;
                    }

                    let full_path = if name.is_empty() {
                        base
                    } else {
                        base.join(&name)
                    };

                    if (mask & libc::IN_MOVED_FROM) != 0 {
                        self.pending_cookies
                            .insert(cookie, (Instant::now(), full_path, is_dir));
                    } else if (mask & libc::IN_MOVED_TO) != 0 {
                        if let Some((_, old_path, was_dir)) = self.pending_cookies.remove(&cookie) {
                            if self.is_under_ignored(&full_path) {
                                out.push(FsChangeEvent::Deleted {
                                    path: old_path.clone(),
                                    is_dir: was_dir,
                                });
                                if was_dir {
                                    self.remove_watch_tree(&old_path);
                                }
                            } else {
                                if was_dir {
                                    self.rekey_watch_tree(&old_path, &full_path);
                                }
                                out.push(FsChangeEvent::Renamed {
                                    old_path,
                                    new_path: full_path,
                                    is_dir: was_dir,
                                });
                            }
                        } else if !self.is_under_ignored(&full_path) {
                            if is_dir {
                                self.add_watch_tree(&full_path);
                            }
                            out.push(FsChangeEvent::Modified);
                        }
                    } else if (mask & (libc::IN_DELETE | libc::IN_DELETE_SELF)) != 0 {
                        if is_dir {
                            self.remove_watch_tree(&full_path);
                        }
                        out.push(FsChangeEvent::Deleted {
                            path: full_path,
                            is_dir,
                        });
                    } else if (mask & libc::IN_CREATE) != 0 {
                        if is_dir && !self.is_under_ignored(&full_path) {
                            self.add_watch_tree(&full_path);
                            out.push(FsChangeEvent::Modified);
                        }
                    } else if (mask & libc::IN_CLOSE_WRITE) != 0
                        && !is_dir
                        && !self.is_under_ignored(&full_path)
                    {
                        out.push(FsChangeEvent::Modified);
                    }
                }
            }
        }

        // Expire unpaired IN_MOVED_FROM cookies (e.g., file moved outside watched roots)
        let expired: Vec<u32> = self
            .pending_cookies
            .iter()
            .filter(|(_, (ts, _, _))| ts.elapsed() >= COOKIE_TTL)
            .map(|(&c, _)| c)
            .collect();

        for c in expired {
            if let Some((_, old_path, is_dir)) = self.pending_cookies.remove(&c) {
                if is_dir {
                    self.remove_watch_tree(&old_path);
                }
                out.push(FsChangeEvent::Deleted {
                    path: old_path,
                    is_dir,
                });
            }
        }

        Ok(out)
    }
}

impl Drop for LinuxInotifyWatcher {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}
