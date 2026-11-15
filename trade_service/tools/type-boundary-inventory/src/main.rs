use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use syn::{Attribute, Item};

fn test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

fn collect(
    source: &str,
    directory: &Path,
    prefix: &str,
    guarded: &BTreeSet<String>,
    aliases: &mut BTreeSet<String>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    items(&file.items, directory, prefix, guarded, aliases, visited)
}

fn items(
    items: &[Item],
    directory: &Path,
    prefix: &str,
    guarded: &BTreeSet<String>,
    aliases: &mut BTreeSet<String>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    for item in items {
        match item {
            Item::Type(alias) if !test_only(&alias.attrs) => {
                aliases.insert(format!("{prefix}::{}", alias.ident));
            }
            Item::Mod(module) if !test_only(&module.attrs) => {
                let name = module.ident.to_string();
                if prefix == "pump_trade_service" && guarded.contains(&name) {
                    continue;
                }
                if module
                    .attrs
                    .iter()
                    .any(|attribute| attribute.path().is_ident("path"))
                {
                    return Err(format!(
                        "Explicit production module paths require review: {prefix}::{name}"
                    ));
                }
                let prefix = format!("{prefix}::{name}");
                let child_directory = directory.join(&name);
                if let Some((_, children)) = &module.content {
                    self::items(
                        children,
                        &child_directory,
                        &prefix,
                        guarded,
                        aliases,
                        visited,
                    )?;
                } else {
                    let flat = directory.join(format!("{name}.rs"));
                    let nested = child_directory.join("mod.rs");
                    let path = match (flat.is_file(), nested.is_file()) {
                        (true, false) => flat,
                        (false, true) => nested,
                        _ => return Err(format!("Missing or ambiguous source module: {prefix}")),
                    };
                    if fs::symlink_metadata(&path)
                        .map_err(|error| error.to_string())?
                        .file_type()
                        .is_symlink()
                        || !visited.insert(path.clone())
                    {
                        return Err(format!("Non-regular or duplicate source module: {prefix}"));
                    }
                    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
                    collect(
                        &source,
                        &child_directory,
                        &prefix,
                        guarded,
                        aliases,
                        visited,
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn inventory(root: &Path, guarded: BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    let mut aliases = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let main = root.join("src/main.rs");
    let source = fs::read_to_string(&main).map_err(|error| error.to_string())?;
    visited.insert(main);
    collect(
        &source,
        &root.join("src"),
        "pump_trade_service",
        &guarded,
        &mut aliases,
        &mut visited,
    )?;
    Ok(aliases)
}

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let root = PathBuf::from(
        arguments
            .next()
            .ok_or("Supply the source-only project directory")?,
    );
    let guarded = arguments.collect();
    let aliases = inventory(&root, guarded)?;
    serde_json::to_writer(std::io::stdout(), &aliases).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_use_rust_syntax_and_do_not_parse_comments_strings_or_guarded_layers() {
        let source = r#"
            // type NotAnAlias = Secret;
            const TEXT: &str = "type AlsoNotAnAlias = Secret;";
            mod adapter { pub type Hidden = crate::config::Config; }
            mod application { pub type ProjectType = u64; }
            #[cfg(test)] mod tests { type TestAlias = u8; }
        "#;
        let mut aliases = BTreeSet::new();
        collect(
            source,
            Path::new("unused"),
            "pump_trade_service",
            &BTreeSet::from(["application".into()]),
            &mut aliases,
            &mut BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(
            aliases,
            BTreeSet::from(["pump_trade_service::adapter::Hidden".into()])
        );
    }

    #[test]
    fn malformed_rust_and_unreviewed_production_paths_are_rejected() {
        for source in ["mod {", "#[path = \"another.rs\"] mod adapter;"] {
            assert!(collect(
                source,
                Path::new("unused"),
                "pump_trade_service",
                &BTreeSet::new(),
                &mut BTreeSet::new(),
                &mut BTreeSet::new()
            )
            .is_err());
        }
    }

    #[test]
    fn associated_types_are_not_confused_with_module_aliases() {
        let source = "impl Trait for Adapter { type Output = u64; } pub type Alias = u8;";
        let mut aliases = BTreeSet::new();
        collect(
            source,
            Path::new("unused"),
            "pump_trade_service",
            &BTreeSet::new(),
            &mut aliases,
            &mut BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(
            aliases,
            BTreeSet::from(["pump_trade_service::Alias".into()])
        );
    }
}
