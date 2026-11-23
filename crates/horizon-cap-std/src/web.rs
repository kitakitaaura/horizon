//! cap-std over `std::fs` for the browser: each [`fs::Dir`] holds its path.

/// What cap-std asks of a caller that opens a path outside any directory it holds.
#[derive(Clone, Copy, Debug)]
pub struct AmbientAuthority(());

/// The authority to open any path ([`fs::Dir::open_ambient_dir`]).
pub fn ambient_authority() -> AmbientAuthority {
    AmbientAuthority(())
}

pub mod time {
    use std::time::Duration;

    /// A file time, as cap-std's `SystemTime` wraps std's.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SystemTime(std::time::SystemTime);

    impl SystemTime {
        pub fn from_std(time: std::time::SystemTime) -> Self {
            Self(time)
        }
        pub fn into_std(self) -> std::time::SystemTime {
            self.0
        }
        pub fn duration_since(
            &self,
            earlier: Self,
        ) -> Result<Duration, std::time::SystemTimeError> {
            self.0.duration_since(earlier.0)
        }
    }
}

pub mod fs {
    use super::AmbientAuthority;
    use std::ffi::OsString;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};

    pub use std::fs::{FileType, Permissions};

    /// A directory: the path it was opened at.
    #[derive(Debug)]
    pub struct Dir {
        path: PathBuf,
    }

    /// The path of `name` in `dir`. cap-std refuses an absolute path or one that leaves the
    /// directory; so does this.
    fn inside(dir: &Path, name: &Path) -> io::Result<PathBuf> {
        use std::path::Component;
        if name
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "a path outside the directory",
            ));
        }
        Ok(dir.join(name))
    }

    impl Dir {
        pub fn open_ambient_dir(path: impl AsRef<Path>, _: AmbientAuthority) -> io::Result<Self> {
            let path = path.as_ref().to_path_buf();
            if !std::fs::metadata(&path)?.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "not a directory",
                ));
            }
            Ok(Self { path })
        }
        pub fn create_ambient_dir_all(
            path: impl AsRef<Path>,
            _: AmbientAuthority,
        ) -> io::Result<()> {
            std::fs::create_dir_all(path)
        }
        fn at(&self, name: impl AsRef<Path>) -> io::Result<PathBuf> {
            inside(&self.path, name.as_ref())
        }
        pub fn open_dir(&self, name: impl AsRef<Path>) -> io::Result<Self> {
            let path = self.at(name)?;
            if !std::fs::metadata(&path)?.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "not a directory",
                ));
            }
            Ok(Self { path })
        }
        pub fn create_dir(&self, name: impl AsRef<Path>) -> io::Result<()> {
            std::fs::create_dir(self.at(name)?)
        }
        pub fn create_dir_all(&self, name: impl AsRef<Path>) -> io::Result<()> {
            std::fs::create_dir_all(self.at(name)?)
        }
        pub fn create_dir_with(
            &self,
            name: impl AsRef<Path>,
            builder: &DirBuilder,
        ) -> io::Result<()> {
            let path = self.at(name)?;
            if builder.recursive {
                std::fs::create_dir_all(path)
            } else {
                std::fs::create_dir(path)
            }
        }
        pub fn open(&self, name: impl AsRef<Path>) -> io::Result<File> {
            self.open_with(name, OpenOptions::new().read(true))
        }
        pub fn create(&self, name: impl AsRef<Path>) -> io::Result<File> {
            self.open_with(
                name,
                OpenOptions::new().write(true).create(true).truncate(true),
            )
        }
        pub fn open_with(&self, name: impl AsRef<Path>, options: &OpenOptions) -> io::Result<File> {
            Ok(File::from_std(options.std.open(self.at(name)?)?))
        }
        pub fn metadata(&self, name: impl AsRef<Path>) -> io::Result<Metadata> {
            std::fs::metadata(self.at(name)?).map(Metadata)
        }
        pub fn symlink_metadata(&self, name: impl AsRef<Path>) -> io::Result<Metadata> {
            std::fs::symlink_metadata(self.at(name)?).map(Metadata)
        }
        pub fn dir_metadata(&self) -> io::Result<Metadata> {
            std::fs::metadata(&self.path).map(Metadata)
        }
        pub fn exists(&self, name: impl AsRef<Path>) -> bool {
            self.at(name).is_ok_and(|path| path.exists())
        }
        pub fn is_dir(&self, name: impl AsRef<Path>) -> bool {
            self.at(name).is_ok_and(|path| path.is_dir())
        }
        pub fn is_file(&self, name: impl AsRef<Path>) -> bool {
            self.at(name).is_ok_and(|path| path.is_file())
        }
        pub fn entries(&self) -> io::Result<ReadDir> {
            Ok(ReadDir(std::fs::read_dir(&self.path)?))
        }
        pub fn read_dir(&self, name: impl AsRef<Path>) -> io::Result<ReadDir> {
            Ok(ReadDir(std::fs::read_dir(self.at(name)?)?))
        }
        pub fn read(&self, name: impl AsRef<Path>) -> io::Result<Vec<u8>> {
            std::fs::read(self.at(name)?)
        }
        pub fn read_to_string(&self, name: impl AsRef<Path>) -> io::Result<String> {
            std::fs::read_to_string(self.at(name)?)
        }
        pub fn write(&self, name: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
            std::fs::write(self.at(name)?, contents)
        }
        pub fn remove_file(&self, name: impl AsRef<Path>) -> io::Result<()> {
            std::fs::remove_file(self.at(name)?)
        }
        pub fn remove_dir(&self, name: impl AsRef<Path>) -> io::Result<()> {
            std::fs::remove_dir(self.at(name)?)
        }
        pub fn remove_dir_all(&self, name: impl AsRef<Path>) -> io::Result<()> {
            std::fs::remove_dir_all(self.at(name)?)
        }
        pub fn remove_open_dir(self) -> io::Result<()> {
            std::fs::remove_dir(&self.path)
        }
        pub fn remove_open_dir_all(self) -> io::Result<()> {
            std::fs::remove_dir_all(&self.path)
        }
        pub fn rename(
            &self,
            from: impl AsRef<Path>,
            to_dir: &Self,
            to: impl AsRef<Path>,
        ) -> io::Result<()> {
            std::fs::rename(self.at(from)?, to_dir.at(to)?)
        }
        pub fn copy(
            &self,
            from: impl AsRef<Path>,
            to_dir: &Self,
            to: impl AsRef<Path>,
        ) -> io::Result<u64> {
            std::fs::copy(self.at(from)?, to_dir.at(to)?)
        }
        pub fn try_clone(&self) -> io::Result<Self> {
            Ok(Self {
                path: self.path.clone(),
            })
        }
    }

    /// `std::fs::DirBuilder`'s options, applied by [`Dir::create_dir_with`].
    #[derive(Clone, Debug, Default)]
    pub struct DirBuilder {
        recursive: bool,
    }
    impl DirBuilder {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn recursive(&mut self, recursive: bool) -> &mut Self {
            self.recursive = recursive;
            self
        }
    }

    /// `std::fs::OpenOptions`, with the follow setting cap-fs-ext adds.
    #[derive(Clone, Debug)]
    pub struct OpenOptions {
        std: std::fs::OpenOptions,
    }
    impl Default for OpenOptions {
        fn default() -> Self {
            Self::new()
        }
    }
    impl OpenOptions {
        pub fn new() -> Self {
            Self {
                std: std::fs::OpenOptions::new(),
            }
        }
        pub fn read(&mut self, read: bool) -> &mut Self {
            self.std.read(read);
            self
        }
        pub fn write(&mut self, write: bool) -> &mut Self {
            self.std.write(write);
            self
        }
        pub fn append(&mut self, append: bool) -> &mut Self {
            self.std.append(append);
            self
        }
        pub fn truncate(&mut self, truncate: bool) -> &mut Self {
            self.std.truncate(truncate);
            self
        }
        pub fn create(&mut self, create: bool) -> &mut Self {
            self.std.create(create);
            self
        }
        pub fn create_new(&mut self, create_new: bool) -> &mut Self {
            self.std.create_new(create_new);
            self
        }
    }

    /// A file, as std's.
    #[derive(Debug)]
    pub struct File(std::fs::File);
    impl File {
        pub fn from_std(file: std::fs::File) -> Self {
            Self(file)
        }
        pub fn into_std(self) -> std::fs::File {
            self.0
        }
        pub fn metadata(&self) -> io::Result<Metadata> {
            self.0.metadata().map(Metadata)
        }
        pub fn sync_all(&self) -> io::Result<()> {
            self.0.sync_all()
        }
        pub fn sync_data(&self) -> io::Result<()> {
            self.0.sync_data()
        }
        pub fn set_len(&self, size: u64) -> io::Result<()> {
            self.0.set_len(size)
        }
        pub fn try_clone(&self) -> io::Result<Self> {
            self.0.try_clone().map(Self)
        }
    }
    impl Read for File {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Read for &File {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&self.0).read(buf)
        }
    }
    impl Write for File {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Write for &File {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&self.0).write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            (&self.0).flush()
        }
    }
    impl Seek for File {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.seek(pos)
        }
    }
    impl Seek for &File {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            (&self.0).seek(pos)
        }
    }

    /// std's metadata, with cap-std's time type.
    #[derive(Clone, Debug)]
    pub struct Metadata(std::fs::Metadata);
    impl Metadata {
        pub fn from_std(metadata: std::fs::Metadata) -> Self {
            Self(metadata)
        }
        pub fn is_dir(&self) -> bool {
            self.0.is_dir()
        }
        pub fn is_file(&self) -> bool {
            self.0.is_file()
        }
        pub fn is_symlink(&self) -> bool {
            self.0.is_symlink()
        }
        pub fn len(&self) -> u64 {
            self.0.len()
        }
        pub fn file_type(&self) -> FileType {
            self.0.file_type()
        }
        pub fn permissions(&self) -> Permissions {
            self.0.permissions()
        }
        pub fn modified(&self) -> io::Result<super::time::SystemTime> {
            self.0.modified().map(super::time::SystemTime::from_std)
        }
        pub fn accessed(&self) -> io::Result<super::time::SystemTime> {
            self.0.accessed().map(super::time::SystemTime::from_std)
        }
        pub fn created(&self) -> io::Result<super::time::SystemTime> {
            self.0.created().map(super::time::SystemTime::from_std)
        }
    }

    /// A directory's entries ([`Dir::entries`]).
    #[derive(Debug)]
    pub struct ReadDir(std::fs::ReadDir);
    impl Iterator for ReadDir {
        type Item = io::Result<DirEntry>;
        fn next(&mut self) -> Option<Self::Item> {
            self.0.next().map(|entry| entry.map(DirEntry))
        }
    }

    /// One of a directory's entries.
    #[derive(Debug)]
    pub struct DirEntry(std::fs::DirEntry);
    impl DirEntry {
        pub fn file_name(&self) -> OsString {
            self.0.file_name()
        }
        pub fn file_type(&self) -> io::Result<FileType> {
            self.0.file_type()
        }
        pub fn metadata(&self) -> io::Result<Metadata> {
            self.0.metadata().map(Metadata)
        }
        pub fn open(&self) -> io::Result<File> {
            std::fs::File::open(self.0.path()).map(File)
        }
        pub fn open_dir(&self) -> io::Result<Dir> {
            Ok(Dir {
                path: self.0.path(),
            })
        }
        pub fn remove_file(&self) -> io::Result<()> {
            std::fs::remove_file(self.0.path())
        }
        pub fn remove_dir(&self) -> io::Result<()> {
            std::fs::remove_dir(self.0.path())
        }
    }
}
