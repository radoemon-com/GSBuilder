use crate::core::error::{AppResult, input_error};
use crate::core::paths::is_valid_component;
use std::fs;
use std::path::{Path, PathBuf};

pub fn remove_server(root: &Path, name: &str) -> AppResult<PathBuf> {
    if !is_valid_component(name) {
        return Err(input_error("Invalid server name"));
    }

    let root = root.canonicalize()?;
    let target = root.join(name);
    let metadata = fs::symlink_metadata(&target)?;
    if !metadata.file_type().is_dir() {
        return Err(input_error(format!(
            "Server path is not a directory: {}",
            target.display()
        )));
    }

    let canonical_target = target.canonicalize()?;
    if canonical_target.parent() != Some(root.as_path()) {
        return Err(input_error(format!(
            "Server path is outside the configured server directory: {}",
            target.display()
        )));
    }

    fs::remove_dir_all(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::remove_server;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_directory() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-server-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn removes_only_the_requested_server_directory() {
        let root = test_directory();
        let server = root.join("server");
        let other_server = root.join("keep");
        fs::create_dir_all(&server).unwrap();
        fs::write(server.join("world.dat"), b"world").unwrap();
        fs::create_dir(&other_server).unwrap();

        assert_eq!(remove_server(&root, "server").unwrap(), server);
        assert!(!server.exists());
        assert!(other_server.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_names_and_server_symlinks() {
        let root = test_directory();
        let outside = test_directory().with_extension("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.dat"), b"keep").unwrap();

        assert!(remove_server(&root, "../outside").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
            assert!(remove_server(&root, "link").is_err());
            assert!(outside.join("keep.dat").exists());
        }

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
