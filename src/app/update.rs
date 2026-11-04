use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::cmp::Ordering;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const RELEASE_ASSET: &str = "GSBuilder-linux-x86_64.tar.gz";
const MAX_ARCHIVE_SIZE: u64 = 128 * 1024 * 1024;
const MAX_BINARY_SIZE: u64 = 128 * 1024 * 1024;
const MAX_UNPACKED_ARCHIVE_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub fn check_and_update(config: &Config, language: &Language) -> AppResult<bool> {
    if config.updates.github_repository.is_empty() {
        return Ok(false);
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        println!("{}", language.text("update.unsupported_platform")?);
        return Ok(false);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let source_root = match source_project_root() {
            Ok(root) => root,
            Err(error) => {
                eprintln!(
                    "{}",
                    language.format("update.check_failed", &[("error", &error.to_string())])?
                );
                return Ok(false);
            }
        };
        let release = match latest_release(config, source_root.is_none()) {
            Ok(release) => release,
            Err(error) => {
                eprintln!(
                    "{}",
                    language.format("update.check_failed", &[("error", &error.to_string())])?
                );
                return Ok(false);
            }
        };
        let Some(release) = release else {
            return Ok(false);
        };
        println!(
            "{}",
            language.format(
                "update.available",
                &[("version", release.tag_name.trim_start_matches('v'))]
            )?
        );
        print!("{}", language.text("update.confirm")?);
        io::stdout().flush()?;
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer)? == 0
            || !matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "はい"
            )
        {
            return Ok(false);
        }

        let result = if let Some(project_root) = source_root {
            apply_source_update(&release, &project_root)
        } else {
            apply_update(&release)
        };
        match result {
            Ok(()) => {
                println!("{}", language.text("update.restart")?);
                Ok(true)
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    language.format("update.install_failed", &[("error", &error.to_string())])?
                );
                Ok(false)
            }
        }
    }
}

fn latest_release(config: &Config, require_binary_asset: bool) -> AppResult<Option<Release>> {
    let client = Client::builder()
        .user_agent(&config.network.user_agent)
        .timeout(Duration::from_secs(15))
        .build()?;
    let release: Release = client
        .get(format!(
            "https://api.github.com/repos/{}/releases/latest",
            config.updates.github_repository
        ))
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()?;
    let current_version = env!("CARGO_PKG_VERSION");
    if compare_versions(&release.tag_name, current_version)? != Ordering::Greater {
        return Ok(None);
    }
    if require_binary_asset
        && !release.assets.iter().any(|asset| {
            asset.name == RELEASE_ASSET && is_github_release_url(&asset.browser_download_url)
        })
    {
        return Err(input_error(format!(
            "GitHub release {} is missing a valid {RELEASE_ASSET} asset",
            release.tag_name
        )));
    }
    Ok(Some(release))
}

fn source_project_root() -> AppResult<Option<PathBuf>> {
    if let Some(project_root) = std::env::var_os("GSBUILDER_PROJECT_DIR") {
        let project_root = PathBuf::from(project_root);
        if !is_source_project(&project_root) {
            return Err(input_error(format!(
                "GSBUILDER_PROJECT_DIR is not a Git source checkout: {}",
                project_root.display()
            )));
        }
        return Ok(Some(project_root));
    }

    let executable = std::env::current_exe()?;
    Ok(executable
        .ancestors()
        .find(|path| is_source_project(path))
        .map(Path::to_path_buf))
}

fn is_source_project(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join(".git").exists()
}

fn apply_update(release: &Release) -> AppResult<()> {
    let asset = release
        .assets
        .iter()
        .find(|asset| {
            asset.name == RELEASE_ASSET && is_github_release_url(&asset.browser_download_url)
        })
        .ok_or_else(|| input_error(format!("Release asset not found: {RELEASE_ASSET}")))?;
    let response = Client::builder()
        .user_agent(concat!("GSBuilder/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()?
        .get(&asset.browser_download_url)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_SIZE)
    {
        return Err(input_error(
            "Release archive exceeds the 128 MiB size limit",
        ));
    }
    let mut archive = Vec::new();
    response
        .take(MAX_ARCHIVE_SIZE + 1)
        .read_to_end(&mut archive)?;
    if archive.len() as u64 > MAX_ARCHIVE_SIZE {
        return Err(input_error(
            "Release archive exceeds the 128 MiB size limit",
        ));
    }

    let executable = std::env::current_exe()?;
    let replacement = replacement_path(&executable);
    if let Err(error) = write_binary_from_archive(&archive, &replacement) {
        let _ = fs::remove_file(&replacement);
        return Err(error);
    }
    install_replacement(&replacement, &executable)
}

fn apply_source_update(release: &Release, project_root: &Path) -> AppResult<()> {
    ensure_clean_worktree(project_root)?;
    let tag_ref = format!("refs/tags/{}", release.tag_name);
    run_git(project_root, &["fetch", "--no-tags", "origin", &tag_ref])?;
    run_git(project_root, &["merge", "--ff-only", "FETCH_HEAD"])?;

    println!("Building updated source checkout...");
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(project_root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(project_root.join("target"))
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not build the updated source checkout: {status}"
        )));
    }

    let built_executable = project_root.join("target/release/GSBuilder");
    let metadata = fs::metadata(&built_executable)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BINARY_SIZE {
        return Err(input_error(format!(
            "Built executable has an invalid file size: {}",
            built_executable.display()
        )));
    }

    let executable = std::env::current_exe()?;
    let replacement = replacement_path(&executable);
    let write_result = (|| -> AppResult<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&replacement)?;
        let mut input = fs::File::open(&built_executable)?;
        let copied = io::copy(&mut input, &mut output)?;
        if copied != metadata.len() {
            return Err(input_error(
                "Built executable size changed while preparing the update",
            ));
        }
        output.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&replacement);
        return Err(error);
    }
    install_replacement(&replacement, &executable)
}

fn ensure_clean_worktree(project_root: &Path) -> AppResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Could not inspect Git worktree at {}: {}",
            project_root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if !output.stdout.is_empty() {
        return Err(input_error(format!(
            "Source update stopped because the Git worktree has local changes. Commit or stash them first: {}",
            project_root.display()
        )));
    }
    Ok(())
}

fn run_git(project_root: &Path, args: &[&str]) -> AppResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Git command failed ({}): {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn install_replacement(replacement: &Path, executable: &Path) -> AppResult<()> {
    if let Err(error) = fs::rename(replacement, executable) {
        let _ = fs::remove_file(replacement);
        if error.kind() == io::ErrorKind::PermissionDenied {
            return Err(input_error(format!(
                "Cannot replace {}. Install GSBuilder in a writable directory or use sudo",
                executable.display()
            )));
        }
        return Err(Box::new(error));
    }
    Ok(())
}

fn write_binary_from_archive(archive_bytes: &[u8], destination: &Path) -> AppResult<()> {
    let decoder = flate2::read::GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries()?;
    let mut total_size = 0_u64;
    let mut binary = None;
    for entry in entries {
        let mut entry = entry?;
        let size = entry.size();
        total_size = total_size
            .checked_add(size)
            .ok_or_else(|| input_error("Release archive size overflow"))?;
        if total_size > MAX_UNPACKED_ARCHIVE_SIZE {
            return Err(input_error(
                "Unpacked release archive exceeds the 512 MiB size limit",
            ));
        }
        if entry.path()? != Path::new("GSBuilder") {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(input_error("GSBuilder archive entry is not a regular file"));
        }
        if size == 0 || size > MAX_BINARY_SIZE {
            return Err(input_error("Updated executable has an invalid file size"));
        }
        if binary.is_some() {
            return Err(input_error(
                "Release archive contains more than one GSBuilder executable",
            ));
        }
        let mut contents = Vec::with_capacity(size as usize);
        let copied = io::copy(&mut entry, &mut contents)?;
        if copied != size {
            return Err(input_error(
                "Updated executable size does not match its archive entry",
            ));
        }
        binary = Some(contents);
    }
    let binary = binary.ok_or_else(|| {
        input_error("Release archive does not contain a root-level GSBuilder executable")
    })?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    output.write_all(&binary)?;
    output.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn replacement_path(executable: &Path) -> PathBuf {
    let file_name = executable.file_name().unwrap_or_default().to_string_lossy();
    executable.with_file_name(format!(".{file_name}.update-{}.tmp", std::process::id()))
}

fn is_github_release_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.path().contains("/releases/download/")
    })
}

fn compare_versions(remote: &str, current: &str) -> AppResult<Ordering> {
    let remote = parse_version(remote)?;
    let current = parse_version(current)?;
    Ok(remote.cmp(&current))
}

#[derive(Debug, Eq, PartialEq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_version(value: &str) -> AppResult<Version> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let value = value.split_once('+').map_or(value, |(version, _)| version);
    let (release, prerelease) = value
        .split_once('-')
        .map_or((value, None), |(release, prerelease)| {
            (release, Some(prerelease.to_owned()))
        });
    let mut parts = release.split('.');
    let parse_part = |part: Option<&str>| {
        part.and_then(|part| part.parse::<u64>().ok())
            .ok_or_else(|| input_error(format!("Invalid version tag: {value}")))
    };
    let version = Version {
        major: parse_part(parts.next())?,
        minor: parse_part(parts.next())?,
        patch: parse_part(parts.next())?,
        prerelease,
    };
    if parts.next().is_some() {
        return Err(input_error(format!("Invalid version tag: {value}")));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::{
        check_and_update, compare_versions, ensure_clean_worktree, is_github_release_url,
        replacement_path, write_binary_from_archive,
    };
    use crate::core::config::Config;
    use crate::core::language::Language;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::cmp::Ordering;
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "gsbuilder-update-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, AtomicOrdering::Relaxed)
        ))
    }

    #[test]
    fn compares_release_versions_and_prereleases() {
        assert_eq!(
            compare_versions("v1.2.0", "1.1.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.2.0", "1.2.0").unwrap(), Ordering::Equal);
        assert_eq!(
            compare_versions("1.2.0", "1.2.0-rc.1").unwrap(),
            Ordering::Greater
        );
        assert!(compare_versions("latest", "1.0.0").is_err());
    }

    #[test]
    fn skips_update_checks_when_no_repository_is_configured() {
        let config = Config::default();
        let language = Language::load(&config.language_file("en").unwrap()).unwrap();
        assert!(!check_and_update(&config, &language).unwrap());
    }

    #[test]
    fn accepts_only_https_github_release_asset_urls() {
        assert!(is_github_release_url(
            "https://github.com/owner/repo/releases/download/v1.0.1/GSBuilder-linux-x86_64.tar.gz"
        ));
        assert!(!is_github_release_url(
            "http://github.com/owner/repo/releases/download/v1.0.1/file"
        ));
        assert!(!is_github_release_url(
            "https://example.com/releases/download/v1.0.1/file"
        ));
    }

    #[test]
    fn creates_a_temporary_path_next_to_the_executable() {
        assert_eq!(
            replacement_path(Path::new("/opt/gsbuilder/GSBuilder")),
            PathBuf::from(format!(
                "/opt/gsbuilder/.GSBuilder.update-{}.tmp",
                std::process::id()
            ))
        );
    }

    #[test]
    fn source_updates_require_a_clean_git_worktree() {
        let directory = temp_dir();
        fs::create_dir_all(&directory).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&directory)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(directory.join("tracked.txt"), "initial").unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&directory)
                .args(["add", "tracked.txt"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&directory)
                .args([
                    "-c",
                    "user.name=GSBuilder Tests",
                    "-c",
                    "user.email=tests@example.invalid",
                    "commit",
                    "-q",
                    "-m",
                    "initial",
                ])
                .status()
                .unwrap()
                .success()
        );

        assert!(ensure_clean_worktree(&directory).is_ok());
        fs::write(directory.join("tracked.txt"), "changed").unwrap();
        assert!(ensure_clean_worktree(&directory).is_err());
        fs::write(directory.join("tracked.txt"), "initial").unwrap();
        fs::write(directory.join("untracked.txt"), "untracked").unwrap();
        assert!(ensure_clean_worktree(&directory).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extracts_only_the_root_executable_from_a_release_archive() {
        let directory =
            std::env::temp_dir().join(format!("gsbuilder-update-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("GSBuilder.new");
        let mut archive_bytes = Vec::new();
        {
            let encoder = GzEncoder::new(&mut archive_bytes, Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(6);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, "GSBuilder", Cursor::new(b"binary"))
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }

        write_binary_from_archive(&archive_bytes, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"binary");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_archives_without_a_root_executable() {
        let mut archive_bytes = Vec::new();
        {
            let encoder = GzEncoder::new(&mut archive_bytes, Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(6);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, "nested/GSBuilder", Cursor::new(b"binary"))
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }
        assert!(
            write_binary_from_archive(&archive_bytes, Path::new("/tmp/gsbuilder-invalid-update"))
                .is_err()
        );
    }
}
