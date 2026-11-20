//! Incremental background library scan.
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use anyhow::Result;

use super::database::Database;
use super::{is_media_path, metadata};
use crate::event::{AppEvent, ScanEvent};

const BATCH: usize = 200;
const PROGRESS_EVERY: usize = 100;

#[derive(Debug, Default, PartialEq)]
pub struct ScanStats {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Human-readable problems (missing dirs, unreadable dirs); the scan carries on.
    pub errors: Vec<String>,
}

/// Scan `dirs` on a background thread with its own DB connection.
pub fn spawn_scan(db_path: PathBuf, dirs: Vec<PathBuf>, tx: Sender<AppEvent>) -> JoinHandle<()> {
    thread::spawn(move || {
        let send = |e| {
            let _ = tx.send(AppEvent::Scan(e));
        };
        let db = match Database::open(&db_path) {
            Ok(db) => db,
            Err(e) => return send(ScanEvent::Error(format!("library database: {e:#}"))),
        };
        let result = scan(&db, &dirs, |scanned| send(ScanEvent::Progress { scanned }));
        match result {
            Ok(stats) => {
                for e in stats.errors {
                    send(ScanEvent::Error(e));
                }
                send(ScanEvent::Finished {
                    added: stats.added,
                    updated: stats.updated,
                    removed: stats.removed,
                });
            }
            Err(e) => send(ScanEvent::Error(format!("scan failed: {e:#}"))),
        }
    })
}

/// Index new/changed media under `dirs` and drop rows for files that vanished.
/// `progress` receives the running count of media files seen.
pub fn scan(db: &Database, dirs: &[PathBuf], mut progress: impl FnMut(usize)) -> Result<ScanStats> {
    let mut stats = ScanStats::default();
    let known = db.known_files()?;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut roots = Vec::new();
    let mut unreadable: Vec<PathBuf> = Vec::new();
    let mut pending = 0;

    db.begin()?;
    let walked = (|| -> Result<()> {
        for dir in dirs {
            if !dir.is_dir() {
                stats
                    .errors
                    .push(format!("directory not found: {}", dir.display()));
                continue;
            }
            roots.push(dir.clone());
            walk(dir, &mut unreadable, &mut |path, mtime, size| {
                if !seen.insert(path.to_path_buf()) {
                    return Ok(()); // reached twice via overlapping dirs
                }
                if seen.len().is_multiple_of(PROGRESS_EVERY) {
                    progress(seen.len());
                }
                let prev = known.get(path);
                if prev == Some(&(mtime, size)) {
                    return Ok(());
                }
                db.upsert_track(&metadata::read_track(path), mtime)?;
                if prev.is_some() {
                    stats.updated += 1;
                } else {
                    stats.added += 1;
                }
                pending += 1;
                if pending >= BATCH {
                    db.commit()?;
                    db.begin()?;
                    pending = 0;
                }
                Ok(())
            })?;
        }
        Ok(())
    })();
    db.commit()?;
    walked?;

    for d in &unreadable {
        stats
            .errors
            .push(format!("cannot read directory: {}", d.display()));
    }
    // Files under unreadable dirs were not seen, but that does not mean they are gone.
    let gone: Vec<PathBuf> = known
        .keys()
        .filter(|p| roots.iter().any(|r| p.starts_with(r)))
        .filter(|p| !unreadable.iter().any(|u| p.starts_with(u)))
        .filter(|p| !seen.contains(*p))
        .cloned()
        .collect();
    db.remove_paths(&gone)?;
    stats.removed = gone.len();
    Ok(stats)
}

/// Depth-first walk calling `visit(path, mtime, size)` for each media file.
/// Symlinked directories are skipped, which rules out loops.
fn walk(
    root: &Path,
    unreadable: &mut Vec<PathBuf>,
    visit: &mut impl FnMut(&Path, i64, u64) -> Result<()>,
) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                unreadable.push(dir);
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if is_media_path(&path).is_some() {
                // fs::metadata follows file symlinks; broken links are skipped.
                let Ok(meta) = fs::metadata(&path) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                visit(&path, mtime, meta.len())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn write(dir: &Path, rel: &str, body: &[u8]) -> PathBuf {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn add_rescan_modify_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        write(&root, "a.mp3", b"aaa");
        let b = write(&root, "sub/deep/b.flac", b"bbb");
        write(&root, "notes.txt", b"ignored");
        let db = Database::open_in_memory().unwrap();

        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (2, 0, 0));
        assert_eq!(db.all_tracks().unwrap().len(), 2);

        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
        assert_eq!(db.all_tracks().unwrap().len(), 2);

        fs::write(&b, b"changed size").unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (0, 1, 0));

        fs::remove_file(&b).unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (0, 0, 1));
        assert_eq!(db.all_tracks().unwrap().len(), 1);
    }

    #[test]
    fn overlapping_dirs_do_not_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        write(&root, "x/a.ogg", b"a");
        let db = Database::open_in_memory().unwrap();
        let dirs = [root.clone(), root.join("x")];
        scan(&db, &dirs, |_| {}).unwrap();
        assert_eq!(db.all_tracks().unwrap().len(), 1);
    }

    #[test]
    fn overlapping_dirs_count_once_and_removal_is_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        write(&root, "a/x.ogg", b"a");
        write(&root, "ab/y.ogg", b"b");
        let db = Database::open_in_memory().unwrap();
        let s = scan(&db, &[root.clone(), root.join("a")], |_| {}).unwrap();
        assert_eq!(s.added, 2);
        fs::remove_file(root.join("a/x.ogg")).unwrap();
        fs::remove_file(root.join("ab/y.ogg")).unwrap();
        // Only "a" is scanned: "ab" (prefix sibling) must survive.
        let s = scan(&db, &[root.join("a")], |_| {}).unwrap();
        assert_eq!(s.removed, 1);
        assert_eq!(db.all_tracks().unwrap().len(), 1);
    }

    #[test]
    fn removed_file_in_playlist_survives_and_reappears() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let f = write(&root, "a.mp3", b"a");
        let db = Database::open_in_memory().unwrap();
        scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        let t = db.all_tracks().unwrap().remove(0);
        let pid = db.create_playlist("P").unwrap();
        db.add_to_playlist(pid, &[t]).unwrap();
        fs::remove_file(&f).unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!(s.removed, 1);
        assert_eq!(db.playlist_items(pid).unwrap().len(), 1);
        assert!(db.all_tracks().unwrap().is_empty());
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!(s.removed, 0);
        fs::write(&f, b"a").unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!(s.added, 1);
        assert_eq!(db.all_tracks().unwrap().len(), 1);
        assert_eq!(db.playlist_items(pid).unwrap().len(), 1);
    }

    #[test]
    fn missing_dir_reported_others_scanned() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a.wav", b"a");
        let db = Database::open_in_memory().unwrap();
        let dirs = [tmp.path().join("nope"), tmp.path().to_path_buf()];
        let s = scan(&db, &dirs, |_| {}).unwrap();
        assert_eq!(s.added, 1);
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("not found"));
    }

    #[test]
    fn missing_dir_does_not_remove_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("lib");
        write(&root, "a.wav", b"a");
        let db = Database::open_in_memory().unwrap();
        scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!(s.removed, 0);
        assert_eq!(db.all_tracks().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loop_terminates() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        write(&root, "d/a.mp3", b"a");
        std::os::unix::fs::symlink(&root, root.join("d/loop")).unwrap();
        let db = Database::open_in_memory().unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        assert_eq!(s.added, 1);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_dir_counted_and_rows_kept() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        write(&root, "ok.mp3", b"a");
        write(&root, "locked/l.mp3", b"l");
        let db = Database::open_in_memory().unwrap();
        scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        let locked = root.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let s = scan(&db, std::slice::from_ref(&root), |_| {}).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        // root may bypass permissions; only assert the invariant that nothing is lost.
        assert_eq!(s.removed, 0);
        assert_eq!(db.all_tracks().unwrap().len(), 2);
    }

    #[test]
    fn progress_reported_and_thread_finishes() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..205 {
            write(tmp.path(), &format!("t{i}.mp3"), b"x");
        }
        let db_path = tmp.path().join("db/lib.db");
        let (tx, rx) = mpsc::channel();
        spawn_scan(db_path, vec![tmp.path().to_path_buf()], tx)
            .join()
            .unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        let progress = events
            .iter()
            .filter(|e| matches!(e, AppEvent::Scan(ScanEvent::Progress { .. })))
            .count();
        assert_eq!(progress, 2);
        assert!(matches!(
            events.last(),
            Some(AppEvent::Scan(ScanEvent::Finished {
                added: 205,
                updated: 0,
                removed: 0
            }))
        ));
    }
}
