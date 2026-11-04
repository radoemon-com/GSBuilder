use crate::core::error::{AppResult, input_error};
use crate::core::files::copy_path;
use crate::core::language::Language;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) fn add(
    language: &Language,
    server: &Path,
    source: &Path,
    destination: Option<&str>,
) -> AppResult<()> {
    if !source.exists() {
        return Err(input_error(format!(
            "Mod source directory does not exist: {}",
            source.display()
        )));
    }
    let destination = server.join(safe_relative_path(destination.unwrap_or("mods"))?);
    copy_path(source, &destination)?;
    println!(
        "{}",
        language.format(
            "cli.mods.copied",
            &[("path", &destination.display().to_string())]
        )?
    );
    Ok(())
}

pub(super) fn list(language: &Language, server: &Path, destination: Option<&str>) -> AppResult<()> {
    let directory = server.join(safe_relative_path(destination.unwrap_or("mods"))?);
    if !directory.is_dir() {
        println!(
            "{}",
            language.format(
                "cli.mods.none",
                &[("path", &directory.display().to_string())]
            )?
        );
        return Ok(());
    }
    let mut files = Vec::new();
    list_files(&directory, &directory, &mut files)?;
    files.sort();
    if files.is_empty() {
        println!(
            "{}",
            language.format(
                "cli.mods.none",
                &[("path", &directory.display().to_string())]
            )?
        );
    } else {
        for file in files {
            println!("{}", file.display());
        }
    }
    Ok(())
}

fn safe_relative_path(path: &str) -> AppResult<PathBuf> {
    let relative = Path::new(path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(input_error(
            "Mod destination must be a non-empty relative path without '..'",
        ));
    }
    Ok(relative.to_path_buf())
}

fn list_files(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) -> AppResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(input_error(format!(
                "Symbolic links are not supported in mod folders: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            list_files(root, &path, output)?;
        } else if file_type.is_file() {
            output.push(path.strip_prefix(root)?.to_path_buf());
        } else {
            return Err(input_error(format!(
                "Unsupported mod file type: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::safe_relative_path;
    use std::path::Path;

    #[test]
    fn mod_destinations_are_relative_and_cannot_escape_server_directory() {
        assert_eq!(safe_relative_path("mods").unwrap(), Path::new("mods"));
        assert!(safe_relative_path("../outside").is_err());
        assert!(safe_relative_path("/outside").is_err());
        assert!(safe_relative_path("").is_err());
    }
}
