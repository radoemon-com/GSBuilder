use crate::core::config::Config;
use crate::core::error::{AppResult, input_error};
use crate::core::language::Language;
use std::io::{self, Write};

mod commands;

fn confirm_server_deletion(language: &Language, server_name: &str) -> AppResult<bool> {
    print!(
        "{}",
        language.format("server.delete.confirm", &[("name", server_name)])?
    );
    io::stdout().flush()?;
    let mut confirmation = String::new();
    if io::stdin().read_line(&mut confirmation)? == 0
        || confirmation.trim_end_matches(['\r', '\n']) != server_name
    {
        println!("{}", language.text("server.delete.cancelled")?);
        return Ok(false);
    }
    Ok(true)
}

pub fn run(config: &Config, language: &Language) -> AppResult<()> {
    loop {
        print!("{}", language.text("cli.prompt")?);
        io::stdout().flush()?;
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                println!();
                continue;
            }
            Err(error) => return Err(Box::new(error)),
        }
        let words = match tokenize(&line) {
            Ok(words) => words,
            Err(_) => {
                eprintln!("{}", language.text("cli.parse.unclosed_quote")?);
                continue;
            }
        };
        if words.is_empty() {
            continue;
        }
        match dispatch(config, language, &words) {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => eprintln!(
                "{}",
                language.format("cli.error", &[("error", &error.to_string())])?
            ),
        }
    }
    println!("{}", language.text("cli.goodbye")?);
    Ok(())
}

fn dispatch(config: &Config, language: &Language, words: &[String]) -> AppResult<bool> {
    match words[0].as_str() {
        "help" | "?" => {
            let (topic_args, verbose) = parse_help_args(&words[1..]);
            let Some(key) = help_key(&topic_args, verbose) else {
                let topic = topic_args.join(" ");
                return Err(input_error(
                    language.format("cli.help.topic.unknown", &[("topic", &topic)])?,
                ));
            };
            println!("{}", language.text(key)?);
            Ok(true)
        }
        "exit" | "quit" => Ok(false),
        "mc" | "minecraft" => {
            commands::minecraft(config, language, &words[1..])?;
            Ok(true)
        }
        "steam" => {
            commands::steam(config, language, &words[1..])?;
            Ok(true)
        }
        "terraria" => {
            commands::terraria(config, language, &words[1..])?;
            Ok(true)
        }
        command => Err(input_error(
            language.format("cli.command.unknown", &[("command", command)])?,
        )),
    }
}

fn parse_help_args(args: &[String]) -> (Vec<String>, bool) {
    let verbose = args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--verbose" | "-v"));
    let topic = args
        .iter()
        .filter(|arg| !matches!(arg.as_str(), "--verbose" | "-v"))
        .cloned()
        .collect();
    (topic, verbose)
}

fn help_key(args: &[String], verbose: bool) -> Option<&'static str> {
    let topic = args.iter().map(String::as_str).collect::<Vec<_>>();
    let (short, detailed) = match topic.as_slice() {
        [] | ["all"] => ("cli.help.short", "cli.help"),
        ["mc" | "minecraft"] => ("cli.help.short.mc", "cli.help.minecraft"),
        ["mc" | "minecraft", "server"] => ("cli.help.short.mc", "cli.help.minecraft"),
        ["mc" | "minecraft", "server", "create"] => {
            ("cli.help.short.mc.create", "cli.help.mc.create")
        }
        ["mc" | "minecraft", "server", "delete"]
        | ["steam", "server", "delete"]
        | ["terraria", "server", "delete"] => {
            ("cli.help.short.server.delete", "cli.help.server.delete")
        }
        ["mc" | "minecraft", "api"] | ["mc" | "minecraft", "api", "list" | "remove"] => {
            ("cli.help.short.mc.api", "cli.help.mc.api")
        }
        ["mc" | "minecraft", "api", "add"] | ["mc" | "minecraft", "api", "add", _] => {
            ("cli.help.short.mc.api.add", "cli.help.mc.api.add")
        }
        ["mc" | "minecraft", "versions"] => ("cli.help.short.mc.versions", "cli.help.mc.versions"),
        ["mc" | "minecraft", "loaders"] => ("cli.help.short.mc.loaders", "cli.help.mc.loaders"),
        ["mc" | "minecraft", "create"] => ("cli.help.short.mc.create", "cli.help.mc.create"),
        ["mc" | "minecraft", "mod"] | ["mc" | "minecraft", "mod", "add"] => {
            ("cli.help.short.mc.mod", "cli.help.mc.mod")
        }
        ["steam"] => ("cli.help.short.steam", "cli.help.steam"),
        ["steam", "server"] => ("cli.help.short.steam", "cli.help.steam"),
        ["steam", "server", "create"] | ["steam", "install"] => {
            ("cli.help.short.steam.install", "cli.help.steam.install")
        }
        ["steam", "mod"] => ("cli.help.short.steam.mod", "cli.help.steam.mod"),
        ["steam", "mod", "add"] => ("cli.help.short.steam.mod.add", "cli.help.steam.mod.add"),
        ["steam", "mod", "list"] => ("cli.help.short.steam.mod.list", "cli.help.steam.mod.list"),
        ["steam", "workshop"]
        | ["steam", "workshop", "item" | "collection" | "remove"]
        | ["steam", "workshop", "remove", "item" | "collection"] => {
            ("cli.help.short.steam.workshop", "cli.help.steam.workshop")
        }
        ["terraria"] => ("cli.help.short.terraria", "cli.help.terraria"),
        ["terraria", "server"] => ("cli.help.short.terraria", "cli.help.terraria"),
        ["terraria", "server", "create"] | ["terraria", "create"] => {
            ("cli.help.short.terraria.create", "cli.help.terraria")
        }
        ["terraria", "mod"] | ["terraria", "mod", "add" | "add-file"] => {
            ("cli.help.short.terraria.mod", "cli.help.terraria.mod")
        }
        _ => return None,
    };
    Some(if verbose { detailed } else { short })
}

fn tokenize(line: &str) -> AppResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut chars = line.chars().peekable();
    let mut token_started = false;

    while let Some(character) = chars.next() {
        match (quote, character) {
            (Some(current), character) if character == current => quote = None,
            (Some('"'), '\\') if matches!(chars.peek(), Some('"') | Some('\\')) => {
                token.push(chars.next().expect("peeked character exists"));
            }
            (Some(_), character) => token.push(character),
            (None, '"' | '\'') => {
                quote = Some(character);
                token_started = true;
            }
            (None, character) if character.is_whitespace() => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            (None, character) => {
                token.push(character);
                token_started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(input_error("unclosed quote"));
    }
    if token_started {
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::{help_key, parse_help_args, tokenize};

    #[test]
    fn parses_quoted_paths_and_empty_quoted_arguments() {
        assert_eq!(
            tokenize(r#"steam mod add "Core Keeper" "My Server" "mods folder" "#).unwrap(),
            [
                "steam",
                "mod",
                "add",
                "Core Keeper",
                "My Server",
                "mods folder"
            ]
        );
        assert_eq!(
            tokenize(r#"mc api add x "" https://example.com"#).unwrap()[4],
            ""
        );
    }

    #[test]
    fn rejects_unclosed_quotes() {
        assert!(tokenize(r#"mc create "vanilla"#).is_err());
    }

    #[test]
    fn resolves_short_and_verbose_help_topics() {
        assert_eq!(
            help_key(&["steam".to_owned(), "install".to_owned()], false),
            Some("cli.help.short.steam.install")
        );
        assert_eq!(
            help_key(
                &["mc".to_owned(), "server".to_owned(), "delete".to_owned()],
                true
            ),
            Some("cli.help.server.delete")
        );
        assert_eq!(
            help_key(&["mc".to_owned(), "create".to_owned()], true),
            Some("cli.help.mc.create")
        );
        assert_eq!(
            help_key(
                &[
                    "mc".to_owned(),
                    "api".to_owned(),
                    "add".to_owned(),
                    "fabric".to_owned()
                ],
                true
            ),
            Some("cli.help.mc.api.add")
        );
        assert_eq!(
            help_key(
                &["steam".to_owned(), "workshop".to_owned(), "item".to_owned()],
                true
            ),
            Some("cli.help.steam.workshop")
        );
        assert_eq!(
            help_key(
                &[
                    "steam".to_owned(),
                    "workshop".to_owned(),
                    "remove".to_owned(),
                    "item".to_owned()
                ],
                true
            ),
            Some("cli.help.steam.workshop")
        );
        assert_eq!(
            help_key(&["terraria".to_owned(), "create".to_owned()], true),
            Some("cli.help.terraria")
        );
        assert_eq!(
            help_key(
                &["terraria".to_owned(), "mod".to_owned(), "add".to_owned()],
                true
            ),
            Some("cli.help.terraria.mod")
        );
        assert_eq!(help_key(&["unknown".to_owned()], false), None);
    }

    #[test]
    fn parses_verbose_flag_before_or_after_help_topic() {
        assert_eq!(
            parse_help_args(&[
                "steam".to_owned(),
                "install".to_owned(),
                "--verbose".to_owned()
            ]),
            (vec!["steam".to_owned(), "install".to_owned()], true)
        );
        assert_eq!(
            parse_help_args(&["-v".to_owned(), "mc".to_owned()]),
            (vec!["mc".to_owned()], true)
        );
    }
}
