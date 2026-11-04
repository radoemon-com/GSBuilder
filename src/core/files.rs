use crate::core::error::{AppResult, input_error};
use std::fs;
use std::path::Path;

pub fn copy_path(source: &Path, destination: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(input_error(format!(
            "Symbolic links are not supported as mod sources: {}",
            source.display()
        )));
    }
    if metadata.is_dir() {
        return copy_directory_contents(source, destination);
    }
    if !metadata.is_file() {
        return Err(input_error(format!(
            "Unsupported mod source type: {}",
            source.display()
        )));
    }
    fs::create_dir_all(destination)?;
    let file_name = source
        .file_name()
        .ok_or_else(|| input_error("Mod source file has no file name"))?;
    let destination_file = destination.join(file_name);
    if destination_file.exists() {
        return Err(input_error(format!(
            "Mod file already exists: {}",
            destination_file.display()
        )));
    }
    fs::copy(source, destination_file)?;
    Ok(())
}

pub fn copy_directory_contents(source: &Path, destination: &Path) -> AppResult<()> {
    let source_path = source.canonicalize()?;
    let destination_path = if destination.exists() {
        destination.canonicalize()?
    } else {
        destination
            .parent()
            .ok_or_else(|| input_error("Mod destination has no parent directory"))?
            .canonicalize()?
            .join(
                destination
                    .file_name()
                    .ok_or_else(|| input_error("Mod destination has no file name"))?,
            )
    };
    if destination_path.starts_with(&source_path) {
        return Err(input_error(
            "The mod source folder cannot contain the destination folder",
        ));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(input_error(format!(
                "Symbolic links are not supported in mod folders: {}",
                source_path.display()
            )));
        }
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            if destination_path.exists() {
                return Err(input_error(format!(
                    "Mod file already exists: {}",
                    destination_path.display()
                )));
            }
            fs::copy(source_path, destination_path)?;
        } else {
            return Err(input_error(format!(
                "Unsupported mod file type: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{copy_directory_contents, copy_path};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_directory() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn copies_nested_mod_files_and_rejects_recursive_destinations() {
        let root = test_directory();
        let source = root.join("source");
        let destination = root.join("server").join("mods");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(source.join("nested").join("example.jar"), b"mod").unwrap();

        copy_directory_contents(&source, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("nested").join("example.jar")).unwrap(),
            b"mod"
        );
        assert!(copy_directory_contents(&source, &source.join("mods")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copies_a_single_mod_file_to_the_destination_directory() {
        let root = test_directory();
        let source = root.join("example.pak");
        let destination = root.join("server").join("mods");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"mod").unwrap();

        copy_path(&source, &destination).unwrap();
        assert_eq!(fs::read(destination.join("example.pak")).unwrap(), b"mod");
        assert!(copy_path(&source, &destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
