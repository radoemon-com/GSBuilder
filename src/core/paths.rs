use crate::core::error::{AppResult, input_error};
use std::fs;
use std::path::Path;

pub fn is_valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !matches!(value.chars().last(), Some(' ' | '.'))
        && !value
            .chars()
            .any(|character| character.is_control() || r#"/\<>:"|?*"#.contains(character))
}

pub fn prepare_target(target: &Path) -> AppResult<()> {
    if target.exists() {
        let is_nonempty = if target.is_dir() {
            fs::read_dir(target)?.next().transpose()?.is_some()
        } else {
            true
        };
        if is_nonempty {
            return Err(input_error(format!(
                "Install directory must be empty: {}",
                target.display()
            )));
        }
    } else {
        fs::create_dir_all(target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_valid_component;

    #[test]
    fn rejects_path_traversal_and_invalid_component_names() {
        for value in ["", ".", "..", "../outside", "name/other", "name."] {
            assert!(
                !is_valid_component(value),
                "unexpectedly accepted {value:?}"
            );
        }
        assert!(is_valid_component("My Server_日本語"));
    }
}
