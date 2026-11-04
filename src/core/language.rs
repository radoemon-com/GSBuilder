use crate::core::error::{AppResult, input_error};
use serde_json::from_str;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct Language {
    translations: HashMap<String, String>,
}

impl Language {
    pub fn load(path: &Path) -> AppResult<Self> {
        let contents = fs::read_to_string(path).map_err(|error| {
            input_error(format!(
                "Could not read language file {}: {error}",
                path.display()
            ))
        })?;
        let translations = from_str(&contents).map_err(|error| {
            input_error(format!("Invalid language file {}: {error}", path.display()))
        })?;
        Ok(Self { translations })
    }

    pub fn text(&self, key: &str) -> AppResult<&str> {
        self.translations
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| input_error(format!("Missing translation key: {key}")))
    }

    pub fn format(&self, key: &str, values: &[(&str, &str)]) -> AppResult<String> {
        let mut text = self.text(key)?.to_owned();
        for (name, value) in values {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::Language;
    use crate::core::config::Config;
    use serde_json::Value;
    use std::fs;

    #[test]
    fn japanese_and_english_files_have_matching_keys() {
        let config = Config::load().unwrap();
        let japanese: Value =
            serde_json::from_str(&fs::read_to_string(config.language_file("ja").unwrap()).unwrap())
                .unwrap();
        let english: Value =
            serde_json::from_str(&fs::read_to_string(config.language_file("en").unwrap()).unwrap())
                .unwrap();
        let japanese_keys = japanese.as_object().unwrap();
        let english_keys = english.as_object().unwrap();

        assert_eq!(
            japanese_keys.keys().collect::<Vec<_>>(),
            english_keys.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn loads_translations_and_formats_placeholders() {
        let config = Config::load().unwrap();
        let english = Language::load(&config.language_file("en").unwrap()).unwrap();
        assert_eq!(
            english
                .format("ui.choice.invalid", &[("max", "3")])
                .unwrap(),
            "Enter a number from 1 to 3."
        );
    }
}
