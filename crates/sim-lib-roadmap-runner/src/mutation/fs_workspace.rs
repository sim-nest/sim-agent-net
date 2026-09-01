/// Native adapter with no-follow observation, same-directory temporaries, and
/// explicit file and parent-directory synchronization.
pub struct FsWorkspace {
    root: PathBuf,
    nonce: u64,
}
impl FsWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, MutationError> {
        let root = root.into().canonicalize()?;
        if !root.is_dir() {
            return Err(MutationError::InvalidPath);
        }
        Ok(Self { root, nonce: 0 })
    }
    fn target(&self, path: &str) -> Result<PathBuf, MutationError> {
        validate_relative_path(path)?;
        Ok(self.root.join(path))
    }
}

impl MutationWorkspace for FsWorkspace {
    fn observe(&self, path: &str) -> Result<PortableImage, MutationError> {
        let target = self.target(path)?;
        match fs::symlink_metadata(&target) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PortableImage::absent()),
            Err(e) => Err(e.into()),
            Ok(meta) if !meta.file_type().is_file() => {
                Err(MutationError::UnsupportedFileKind(path.into()))
            }
            Ok(meta) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    Ok(PortableImage::file(
                        fs::read(target)?,
                        meta.permissions().mode() & 0o777,
                    ))
                }
                #[cfg(not(unix))]
                {
                    Ok(PortableImage::file(
                        fs::read(target)?,
                        if meta.permissions().readonly() {
                            0o444
                        } else {
                            0o666
                        },
                    ))
                }
            }
        }
    }
    fn apply(
        &mut self,
        entry: &SealedEntry,
        fail: &mut dyn FnMut(Failpoint) -> Result<(), MutationError>,
    ) -> Result<(), MutationError> {
        let target = self.target(&entry.path)?;
        let parent = target.parent().ok_or(MutationError::InvalidPath)?;
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
        match (&entry.postimage.bytes, entry.postimage.mode) {
            (Some(bytes), Some(mode)) => {
                self.nonce += 1;
                let name = format!(".sim-mutation-{}-{}.tmp", std::process::id(), self.nonce);
                let temp = parent.join(name);
                fail(Failpoint::BeforeTempWrite)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                file.write_all(bytes)?;
                fail(Failpoint::AfterTempWrite)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(fs::Permissions::from_mode(mode))?;
                }
                fail(Failpoint::BeforeFlush)?;
                file.sync_all()?;
                fail(Failpoint::AfterFlush)?;
                fail(Failpoint::BeforeReplace)?;
                fs::rename(&temp, &target)?;
                fail(Failpoint::AfterReplace)?;
            }
            (None, None) => {
                fail(Failpoint::BeforeReplace)?;
                match fs::remove_file(&target) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    r => r?,
                }
                fail(Failpoint::AfterReplace)?;
            }
            _ => return Err(MutationError::InvalidImage),
        }
        fail(Failpoint::BeforeDirectorySync)?;
        File::open(parent)?.sync_all()?;
        fail(Failpoint::AfterDirectorySync)
    }
    fn durability(&self) -> Durability {
        Durability::FileAndDirectorySync
    }
}
