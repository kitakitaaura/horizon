//! cap-fs-ext's traits over horizon-cap-std's browser stand-in.
use cap_std::fs::{Dir, Metadata, OpenOptions};
use std::io;
use std::path::Path;

/// Whether an open follows a symlink at its last component. The browser's filesystem has
/// none, so both behave alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowSymlinks {
    Yes,
    No,
}

/// [`OpenOptions`]'s follow setting.
pub trait OpenOptionsFollowExt {
    fn follow(&mut self, follow: FollowSymlinks) -> &mut Self;
}
impl OpenOptionsFollowExt for OpenOptions {
    fn follow(&mut self, _: FollowSymlinks) -> &mut Self {
        self
    }
}

/// cap-fs-ext's extra directory operations.
pub trait DirExt {
    fn open_dir_nofollow(&self, path: impl AsRef<Path>) -> io::Result<Dir>;
    fn remove_file_or_symlink(&self, path: impl AsRef<Path>) -> io::Result<()>;
}
impl DirExt for Dir {
    fn open_dir_nofollow(&self, path: impl AsRef<Path>) -> io::Result<Dir> {
        self.open_dir(path)
    }
    fn remove_file_or_symlink(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.remove_file(path)
    }
}

/// The file identity natively read from a file's metadata: the browser's filesystem gives
/// none, so every file reads as device 0, inode 0, one link.
pub trait MetadataExt {
    fn dev(&self) -> u64;
    fn ino(&self) -> u64;
    fn nlink(&self) -> u64;
}
impl MetadataExt for Metadata {
    fn dev(&self) -> u64 {
        0
    }
    fn ino(&self) -> u64 {
        0
    }
    fn nlink(&self) -> u64 {
        1
    }
}
