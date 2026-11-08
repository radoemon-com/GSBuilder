use crate::core::{
    config::Config,
    error::{AppResult, input_error},
    http::http_client,
    language::Language,
};
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicUsize, Ordering};

const STEAMCMD_DOWNLOAD_URL: &str =
    "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz";

pub fn ensure_java(config: &Config, language: &Language) -> AppResult<()> {
    if !java_is_available(config) {
        println!("{}", language.text("setup.java.missing")?);
        if confirm(language.text("setup.install.confirm")?)? {
            install_java(config, language)?;
        }
    }
    Ok(())
}

pub fn ensure_steamcmd(config: &mut Config, language: &Language) -> AppResult<()> {
    if !steamcmd_is_available(config) {
        println!("{}", language.text("setup.steamcmd.missing")?);
        if confirm(language.text("setup.install.confirm")?)? {
            install_steamcmd(config, language)?;
        }
    }
    Ok(())
}

fn java_is_available(config: &Config) -> bool {
    Command::new(config.executable_path(&config.minecraft.java_executable))
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn steamcmd_is_available(config: &Config) -> bool {
    let executable = config.executable_path(&config.steam.steamcmd_executable);
    if executable.components().count() > 1 || executable.is_absolute() {
        is_executable(&executable)
    } else {
        find_in_path(&executable).is_some_and(|path| is_executable(&path))
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn find_in_path(program: &Path) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        let candidate = directory.join(program);
        candidate.is_file().then_some(candidate)
    })
}

fn confirm(prompt: &str) -> AppResult<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Err(input_error(
            "No response was provided to the installation prompt",
        ));
    }
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "はい"
    ))
}

#[derive(Clone, Copy)]
enum PackageManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
}

impl PackageManager {
    fn detect() -> Option<Self> {
        [
            ("apt-get", Self::Apt),
            ("dnf", Self::Dnf),
            ("pacman", Self::Pacman),
            ("zypper", Self::Zypper),
        ]
        .into_iter()
        .find_map(|(name, manager)| find_in_path(Path::new(name)).map(|_| manager))
    }

    fn install_java(self, language: &Language) -> AppResult<()> {
        let commands: Vec<(&str, Vec<&str>)> = match self {
            Self::Apt => vec![
                ("apt-get", vec!["update"]),
                ("apt-get", vec!["install", "-y", "default-jre-headless"]),
            ],
            Self::Dnf => vec![("dnf", vec!["install", "-y", "java-21-openjdk-headless"])],
            Self::Pacman => vec![(
                "pacman",
                vec!["-S", "--needed", "--noconfirm", "jre-openjdk-headless"],
            )],
            Self::Zypper => vec![(
                "zypper",
                vec!["--non-interactive", "install", "java-21-openjdk-headless"],
            )],
        };
        run_package_commands(commands, language)
    }

    fn install_steamcmd_runtime(self, language: &Language) -> AppResult<()> {
        let commands: Vec<(&str, Vec<&str>)> = match self {
            Self::Apt => vec![
                ("apt-get", vec!["update"]),
                ("apt-get", vec!["install", "-y", "lib32gcc-s1"]),
            ],
            Self::Dnf => vec![("dnf", vec!["install", "-y", "glibc.i686", "libstdc++.i686"])],
            Self::Pacman => vec![(
                "pacman",
                vec![
                    "-S",
                    "--needed",
                    "--noconfirm",
                    "lib32-glibc",
                    "lib32-gcc-libs",
                ],
            )],
            Self::Zypper => vec![(
                "zypper",
                vec![
                    "--non-interactive",
                    "install",
                    "glibc-32bit",
                    "libstdc++6-32bit",
                ],
            )],
        };
        run_package_commands(commands, language)
    }
}

fn run_package_commands(commands: Vec<(&str, Vec<&str>)>, language: &Language) -> AppResult<()> {
    for (program, args) in commands {
        let status = run_privileged(program, &args)?;
        if !status.success() {
            return Err(input_error(language.format(
                "setup.package_install.failed",
                &[("status", &status.to_string())],
            )?));
        }
    }
    Ok(())
}

fn run_privileged(program: &str, args: &[&str]) -> AppResult<ExitStatus> {
    if running_as_root() {
        Ok(Command::new(program).args(args).status()?)
    } else {
        Ok(Command::new("sudo")
            .arg("--")
            .arg(program)
            .args(args)
            .status()?)
    }
}

fn running_as_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|output| output.status.success() && output.stdout == b"0\n")
}

fn install_java(config: &Config, language: &Language) -> AppResult<()> {
    let Some(package_manager) = PackageManager::detect() else {
        return Err(input_error(
            language.text("setup.package_manager.unsupported")?,
        ));
    };
    package_manager.install_java(language)?;
    if !java_is_available(config) {
        return Err(input_error(language.text("setup.java.still_missing")?));
    }
    println!("{}", language.text("setup.java.installed")?);
    Ok(())
}

fn install_steamcmd(config: &mut Config, language: &Language) -> AppResult<()> {
    if !matches!(std::env::consts::ARCH, "x86_64" | "x86" | "i686") {
        return Err(input_error(
            language.text("setup.steamcmd.arch_unsupported")?,
        ));
    }
    let current_user = current_user()?;
    println!("{}", language.text("setup.steamcmd.user_choice")?);
    println!(
        "{}",
        language.format("setup.steamcmd.user_current", &[("user", &current_user)])?
    );
    println!("{}", language.text("setup.steamcmd.user_other")?);
    print!("{}", language.text("setup.steamcmd.user_prompt")?);
    io::stdout().flush()?;
    let mut choice = String::new();
    if io::stdin().read_line(&mut choice)? == 0 {
        return Err(input_error(language.text("setup.steamcmd.user_required")?));
    }
    let choice = choice.trim();
    let install_user = match choice {
        "1" => current_user.clone(),
        "2" => {
            print!("{}", language.text("setup.steamcmd.username_prompt")?);
            io::stdout().flush()?;
            let mut username = String::new();
            if io::stdin().read_line(&mut username)? == 0 {
                return Err(input_error(language.text("setup.steamcmd.user_required")?));
            }
            let username = username.trim();
            if username.is_empty() || username.chars().any(char::is_control) {
                return Err(input_error(language.text("setup.steamcmd.user_required")?));
            }
            username.to_owned()
        }
        _ => return Err(input_error(language.text("setup.steamcmd.user_required")?)),
    };
    let home = get_user_home(&install_user)?;
    let run_as_user = if install_user == current_user {
        String::new()
    } else {
        install_user.clone()
    };
    let install_root = home.join(".local/share/gsbuilder");
    let target = install_root.join("steamcmd");
    if target.exists() {
        return Err(input_error(language.format(
            "setup.steamcmd.target_exists",
            &[("path", &target.display().to_string())],
        )?));
    }
    let Some(package_manager) = PackageManager::detect() else {
        return Err(input_error(
            language.text("setup.package_manager.unsupported")?,
        ));
    };
    package_manager.install_steamcmd_runtime(language)?;

    let client = http_client(config)?;
    let archive_bytes = client
        .get(STEAMCMD_DOWNLOAD_URL)
        .send()?
        .error_for_status()?
        .bytes()?;
    let staging = TemporaryDirectory::create("gsbuilder-steamcmd")?;
    let unpacked = staging.path().join("unpacked");
    fs::create_dir(&unpacked)?;
    let decoder = flate2::read::GzDecoder::new(Cursor::new(archive_bytes));
    tar::Archive::new(decoder).unpack(&unpacked)?;
    if !unpacked.join("steamcmd.sh").is_file() {
        return Err(input_error(
            language.text("setup.steamcmd.archive_invalid")?,
        ));
    }

    create_directory_as_user(&install_root, &run_as_user)?;
    let staged_install = install_root.join(format!(
        ".steamcmd.installing-{}-{}",
        std::process::id(),
        NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let install_result = (|| {
        copy_directory_as_user(&unpacked, &staged_install, &run_as_user)?;
        if !staged_install.join("steamcmd.sh").is_file() {
            return Err(input_error(
                language.text("setup.steamcmd.archive_invalid")?,
            ));
        }
        rename_directory_as_user(&staged_install, &target, &run_as_user)
    })();
    if let Err(error) = install_result {
        remove_directory_as_user(&staged_install, &run_as_user)?;
        return Err(error);
    }
    let executable = target.join("steamcmd.sh");
    config.save_steam_install(&executable, &run_as_user)?;
    println!(
        "{}",
        language.format(
            "setup.steamcmd.installed",
            &[
                ("path", &executable.display().to_string()),
                ("user", &install_user)
            ],
        )?
    );
    Ok(())
}

static NEXT_STAGING_ID: AtomicUsize = AtomicUsize::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn create(prefix: &str) -> AppResult<Self> {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if self.path.exists()
            && let Err(error) = fs::remove_dir_all(&self.path)
        {
            eprintln!(
                "Could not remove temporary setup directory {}: {error}",
                self.path.display()
            );
        }
    }
}

fn rename_directory_as_user(source: &Path, target: &Path, username: &str) -> AppResult<()> {
    if username.is_empty() {
        fs::rename(source, target)?;
        return Ok(());
    }
    let status = Command::new("sudo")
        .args(["-u", username, "--", "mv", "--"])
        .arg(source)
        .arg(target)
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not finalize SteamCMD installation as {username}: {status}"
        )));
    }
    Ok(())
}

fn remove_directory_as_user(path: &Path, username: &str) -> AppResult<()> {
    if username.is_empty() {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        return Ok(());
    }
    let status = Command::new("sudo")
        .args(["-u", username, "--", "rm", "-rf", "--"])
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not clean failed SteamCMD staging directory {}: {status}",
            path.display()
        )));
    }
    Ok(())
}

fn current_user() -> AppResult<String> {
    let output = Command::new("id").arg("-un").output()?;
    if !output.status.success() {
        return Err(input_error("Could not determine the current Linux user"));
    }
    let username = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if username.is_empty() {
        return Err(input_error("Could not determine the current Linux user"));
    }
    Ok(username)
}

fn get_user_home(username: &str) -> AppResult<PathBuf> {
    let output = Command::new("getent").args(["passwd", username]).output()?;
    if !output.status.success() {
        return Err(input_error(format!(
            "Linux user does not exist: {username}"
        )));
    }
    let entry = String::from_utf8_lossy(&output.stdout);
    let fields = entry.trim().split(':').collect::<Vec<_>>();
    if fields.len() < 6 || fields[0] != username {
        return Err(input_error(format!(
            "Could not resolve Linux user: {username}"
        )));
    }
    Ok(PathBuf::from(fields[5]))
}

fn create_directory_as_user(path: &Path, username: &str) -> AppResult<()> {
    if username.is_empty() {
        fs::create_dir_all(path)?;
        return Ok(());
    }
    let status = Command::new("sudo")
        .args(["-u", username, "--", "mkdir", "-p"])
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not create SteamCMD install directory as {username}: {status}"
        )));
    }
    Ok(())
}

fn copy_directory_as_user(source: &Path, target: &Path, username: &str) -> AppResult<()> {
    if username.is_empty() {
        fs::create_dir(target)?;
        copy_tree(source, target)?;
        return Ok(());
    }
    let status = Command::new("sudo")
        .args(["-u", username, "--", "mkdir"])
        .arg(target)
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not prepare SteamCMD directory as {username}: {status}"
        )));
    }
    let status = Command::new("sudo")
        .args(["-u", username, "--", "cp", "-a"])
        .arg(format!("{}/.", source.display()))
        .arg(target)
        .status()?;
    if !status.success() {
        return Err(input_error(format!(
            "Could not copy SteamCMD files as {username}: {status}"
        )));
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> AppResult<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir(&target_path)?;
            copy_tree(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(source_path, target_path)?;
        } else if file_type.is_symlink() {
            return Err(input_error(
                "SteamCMD archive contains an unexpected symbolic link",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::find_in_path;
    use std::path::Path;

    #[test]
    fn path_lookup_returns_none_for_missing_executable() {
        assert!(find_in_path(Path::new("gsbuilder-missing-executable")).is_none());
    }
}
