// Filesystem safety primitives using fd-relative operations.
//
// Path traversal attacks (symlink-based TOCTOU) are a known risk when
// walking and writing to user-controlled directories. This module provides
// Unix-specific fd-relative syscalls that prevent escapes:
//
// - open_root_fd: Open the vault root with O_DIRECTORY | O_NOFOLLOW.
// - walk_to_parent: Walk to a parent directory via safe openat calls.
// - stat_at_nofollow: Stat a file via fstatat(AT_SYMLINK_NOFOLLOW).
// - openat_create_excl: Open a file for exclusive creation under a parent fd.
// - renameat_parent_fd: Atomically rename a file under parent fd.
// - unlinkat_parent_fd: Remove a file under parent fd.
//
// On Windows, these functions return UnsupportedPlatformError.

#![allow(dead_code)]

use std::io;
use std::path::Path;

#[cfg(unix)]
use rustix::fd::{AsFd, OwnedFd};
#[cfg(unix)]
use rustix::fs::{AtFlags, Mode, OFlags};

// ── Root FD ───────────────────────────────────────────────────

/// Open a directory fd with O_DIRECTORY | O_NOFOLLOW.
///
/// # Unix behavior
/// - Opens the directory at `path` with O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC.
/// - Fails if `path` is a symlink (O_NOFOLLOW).
/// - Fails if `path` is not a directory (O_DIRECTORY).
///
/// # Windows behavior
/// - Returns `UnsupportedPlatformError`.
#[cfg(unix)]
pub fn open_root_fd(path: &Path) -> io::Result<OwnedFd> {
    use rustix::fs::openat;
    use rustix::fs::CWD;

    let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY;
    openat(CWD, path, flags, Mode::empty())
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))
}

#[cfg(not(unix))]
pub fn open_root_fd(_path: &Path) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "open_root_fd: fd-relative operations not supported on Windows",
    ))
}

// ── Walk to Parent ────────────────────────────────────────────

/// Walk from `parent_fd` to the parent directory of `relative_path`.
///
/// Returns an `OwnedFd` for the directory containing the final component.
///
/// # Path safety
/// - Rejects paths with `..` components (returns `InvalidInput`).
/// - Rejects absolute paths (returns `InvalidInput`).
/// - Rejects empty path segments (returns `InvalidInput`).
/// - Rejects NUL bytes (returns `InvalidInput`).
/// - Stops if any intermediate component is a symlink (returns `NotADirectory`).
///
/// # Unix behavior
/// Opens each intermediate directory via `openat(O_DIRECTORY | O_NOFOLLOW)`.
///
/// # Windows behavior
/// Returns `UnsupportedPlatformError`.
#[cfg(unix)]
pub fn walk_to_parent<Fd: AsFd>(parent_fd: Fd, relative_path: &Path) -> io::Result<OwnedFd> {
    use rustix::fs::openat;

    // Validate path
    if relative_path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "walk_to_parent: absolute paths rejected",
        ));
    }

    if relative_path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "walk_to_parent: NUL bytes rejected",
        ));
    }

    let components: Vec<_> = relative_path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s),
            std::path::Component::ParentDir => None, // Signal error below
            _ => None,
        })
        .collect();

    // Check for .. components or empty segments
    if relative_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "walk_to_parent: .. components rejected",
        ));
    }

    if components.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "walk_to_parent: empty path",
        ));
    }

    // Walk to parent directory
    let mut current_fd = parent_fd
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;

    let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY;

    for component in &components[..components.len().saturating_sub(1)] {
        let next_fd = openat(&current_fd, component, flags, Mode::empty())
            .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;
        current_fd = next_fd;
    }

    Ok(current_fd)
}

#[cfg(not(unix))]
pub fn walk_to_parent<Fd>(_parent_fd: Fd, _relative_path: &Path) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "walk_to_parent: fd-relative operations not supported on Windows",
    ))
}

// ── Stat at NoFollow ──────────────────────────────────────────

/// Stat tuple: (mtime_ns, ctime_ns, size_bytes, inode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatNoFollow {
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub size_bytes: i64,
    pub inode: i64,
    pub mode_bits: u32,
}

impl FileStatNoFollow {
    const FILE_TYPE_MASK: u32 = 0o170000;
    const DIRECTORY_BITS: u32 = 0o040000;
    const REGULAR_FILE_BITS: u32 = 0o100000;
    const SYMLINK_BITS: u32 = 0o120000;

    pub fn is_directory(&self) -> bool {
        self.mode_bits & Self::FILE_TYPE_MASK == Self::DIRECTORY_BITS
    }

    pub fn is_regular_file(&self) -> bool {
        self.mode_bits & Self::FILE_TYPE_MASK == Self::REGULAR_FILE_BITS
    }

    pub fn is_symlink(&self) -> bool {
        self.mode_bits & Self::FILE_TYPE_MASK == Self::SYMLINK_BITS
    }
}

/// Stat a file via fstatat(AT_SYMLINK_NOFOLLOW).
///
/// # Unix behavior
/// - Stats the file at `name` relative to `parent_fd`.
/// - Does NOT follow symlinks (AT_SYMLINK_NOFOLLOW).
/// - Returns all four fields: mtime_ns, ctime_ns, size_bytes, inode.
///
/// # Windows behavior
/// Returns `UnsupportedPlatformError`.
#[cfg(unix)]
pub fn stat_at_nofollow<Fd: AsFd>(parent_fd: Fd, name: &Path) -> io::Result<FileStatNoFollow> {
    use rustix::fs::statat;

    let stat = statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;

    let mtime_ns = stat.st_mtime * 1_000_000_000 + stat.st_mtime_nsec;
    let ctime_ns = stat.st_ctime * 1_000_000_000 + stat.st_ctime_nsec;
    let size_bytes = stat.st_size as i64;
    let inode = stat.st_ino as i64;

    Ok(FileStatNoFollow {
        mtime_ns,
        ctime_ns,
        size_bytes,
        inode,
        mode_bits: stat.st_mode,
    })
}

#[cfg(not(unix))]
pub fn stat_at_nofollow<Fd>(_parent_fd: Fd, _name: &Path) -> io::Result<FileStatNoFollow> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stat_at_nofollow: fd-relative operations not supported on Windows",
    ))
}

// ── Open at Create Exclusive ──────────────────────────────────

/// Open a file for exclusive creation under `parent_fd`.
///
/// # Unix behavior
/// - Opens the file at `name` relative to `parent_fd`.
/// - O_CREAT | O_EXCL: fails if the file already exists.
/// - O_NOFOLLOW: fails if the final component is a symlink.
///
/// # Windows behavior
/// Returns `UnsupportedPlatformError`.
#[cfg(unix)]
pub fn openat_create_excl<Fd: AsFd>(parent_fd: Fd, name: &Path) -> io::Result<OwnedFd> {
    use rustix::fs::openat;

    let flags = OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::WRONLY | OFlags::CLOEXEC;
    let mode = Mode::from_raw_mode(0o644);

    openat(parent_fd, name, flags, mode).map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))
}

#[cfg(not(unix))]
pub fn openat_create_excl<Fd>(_parent_fd: Fd, _name: &Path) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "openat_create_excl: fd-relative operations not supported on Windows",
    ))
}

// ── Rename at Parent FD ───────────────────────────────────────

/// Atomically rename a file under `parent_fd`.
///
/// Renames `old_name` to `new_name` within the directory referenced by `parent_fd`.
///
/// # Unix behavior
/// - Uses `renameat(parent_fd, old_name, parent_fd, new_name)`.
/// - Atomic operation at the directory level.
///
/// # Windows behavior
/// Returns `UnsupportedPlatformError`.
#[cfg(unix)]
pub fn renameat_parent_fd<Fd: AsFd>(
    parent_fd: Fd,
    old_name: &Path,
    new_name: &Path,
) -> io::Result<()> {
    use rustix::fs::renameat;

    renameat(parent_fd.as_fd(), old_name, parent_fd.as_fd(), new_name)
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))
}

#[cfg(not(unix))]
pub fn renameat_parent_fd<Fd>(
    _parent_fd: Fd,
    _old_name: &Path,
    _new_name: &Path,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "renameat_parent_fd: fd-relative operations not supported on Windows",
    ))
}

// ── Unlink at Parent FD ───────────────────────────────────────

/// Remove a file under `parent_fd`.
///
/// # Unix behavior
/// - Uses `unlinkat(parent_fd, name, 0)`.
/// - Removes the file at `name` relative to `parent_fd`.
///
/// # Windows behavior
/// Returns `UnsupportedPlatformError`.
#[cfg(unix)]
pub fn unlinkat_parent_fd<Fd: AsFd>(parent_fd: Fd, name: &Path) -> io::Result<()> {
    use rustix::fs::unlinkat;

    unlinkat(parent_fd.as_fd(), name, AtFlags::empty())
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))
}

#[cfg(not(unix))]
pub fn unlinkat_parent_fd<Fd>(_parent_fd: Fd, _name: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unlinkat_parent_fd: fd-relative operations not supported on Windows",
    ))
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn test_open_root_fd_success() {
        let dir = TempDir::new().unwrap();
        let fd = open_root_fd(dir.path());
        assert!(fd.is_ok());
    }

    #[test]
    fn test_open_root_fd_rejects_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();

        let result = open_root_fd(&link);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_open_root_fd_rejects_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"test").unwrap();

        let result = open_root_fd(&file);
        assert!(result.is_err());
    }

    #[test]
    fn test_walk_to_parent_simple() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.txt"), b"test").unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();
        let parent_fd = walk_to_parent(root_fd, Path::new("sub/file.txt")).unwrap();

        // Should be able to stat the file
        let stat = stat_at_nofollow(parent_fd, Path::new("file.txt"));
        assert!(stat.is_ok());
    }

    #[test]
    fn test_walk_to_parent_rejects_parent_dir() {
        let dir = TempDir::new().unwrap();
        let root_fd = open_root_fd(dir.path()).unwrap();

        let result = walk_to_parent(root_fd, Path::new("../file.txt"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_walk_to_parent_rejects_absolute() {
        let dir = TempDir::new().unwrap();
        let root_fd = open_root_fd(dir.path()).unwrap();

        let result = walk_to_parent(root_fd, Path::new("/etc/passwd"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_walk_to_parent_rejects_nul_bytes() {
        let dir = TempDir::new().unwrap();
        let root_fd = open_root_fd(dir.path()).unwrap();

        // Path with embedded NUL (using unsafe to construct)
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let bad_path = Path::new(OsStr::from_bytes(b"file\0.txt"));

        let result = walk_to_parent(root_fd, bad_path);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_walk_to_parent_rejects_symlinked_ancestor() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();

        let sub = link.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.txt"), b"test").unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();

        // Should fail because "link" is a symlink
        let result = walk_to_parent(root_fd, Path::new("link/sub/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_stat_at_nofollow_success() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"hello").unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();
        let stat = stat_at_nofollow(root_fd, Path::new("file.txt")).unwrap();

        assert_eq!(stat.size_bytes, 5);
        assert!(stat.mtime_ns > 0);
        assert!(stat.ctime_ns > 0);
        assert!(stat.inode > 0);
    }

    #[test]
    fn test_stat_at_nofollow_rejects_symlink() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"hello").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&file, &link).unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();

        // Should stat the symlink itself, not the target
        let stat = stat_at_nofollow(root_fd, Path::new("link.txt")).unwrap();

        // Symlink size is different from target size
        assert_ne!(stat.size_bytes, 5);
    }

    #[test]
    fn test_openat_create_excl_success() {
        let dir = TempDir::new().unwrap();
        let root_fd = open_root_fd(dir.path()).unwrap();

        let fd = openat_create_excl(&root_fd, Path::new("new.txt"));
        assert!(fd.is_ok());

        // File should exist
        assert!(dir.path().join("new.txt").exists());
    }

    #[test]
    fn test_openat_create_excl_rejects_existing() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"test").unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();
        let result = openat_create_excl(&root_fd, Path::new("file.txt"));

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn test_openat_create_excl_rejects_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target.txt");
        fs::write(&target, b"test").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&target, &link).unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();

        // O_EXCL should fail on existing symlink
        let result = openat_create_excl(&root_fd, Path::new("link.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_renameat_parent_fd_success() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("old.txt");
        fs::write(&file, b"test").unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();
        let result = renameat_parent_fd(&root_fd, Path::new("old.txt"), Path::new("new.txt"));

        assert!(result.is_ok());
        assert!(!dir.path().join("old.txt").exists());
        assert!(dir.path().join("new.txt").exists());
    }

    #[test]
    fn test_unlinkat_parent_fd_success() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"test").unwrap();

        let root_fd = open_root_fd(dir.path()).unwrap();
        let result = unlinkat_parent_fd(&root_fd, Path::new("file.txt"));

        assert!(result.is_ok());
        assert!(!file.exists());
    }

    #[test]
    fn test_round_trip_safe_write() {
        let dir = TempDir::new().unwrap();
        let root_fd = open_root_fd(dir.path()).unwrap();

        // Create a temp file
        let temp_fd = openat_create_excl(&root_fd, Path::new("temp.txt")).unwrap();
        drop(temp_fd); // Close the file

        // Write some content (using std::fs for simplicity)
        fs::write(dir.path().join("temp.txt"), b"hello").unwrap();

        // Rename it
        renameat_parent_fd(&root_fd, Path::new("temp.txt"), Path::new("final.txt")).unwrap();

        // Verify
        let content = fs::read(dir.path().join("final.txt")).unwrap();
        assert_eq!(content, b"hello");
        assert!(!dir.path().join("temp.txt").exists());
    }
}
