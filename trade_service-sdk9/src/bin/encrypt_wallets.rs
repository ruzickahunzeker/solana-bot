#[path = "../wallet_crypto.rs"]
mod wallet_crypto;

use anyhow::{bail, Context};
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

struct Args {
    input: PathBuf,
    output: PathBuf,
    passphrase_file: Option<PathBuf>,
    force: bool,
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let passphrase = read_passphrase(args.passphrase_file.as_ref())?;
    let plaintext =
        fs::read(&args.input).with_context(|| format!("read input {}", args.input.display()))?;
    serde_json::from_slice::<serde_json::Value>(&plaintext)
        .with_context(|| format!("input is not valid JSON: {}", args.input.display()))?;
    let encrypted = wallet_crypto::encrypt_json(&plaintext, &passphrase)?;

    if args.output.exists() && !args.force {
        bail!(
            "output {} already exists; pass --force to overwrite",
            args.output.display()
        );
    }
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&args.output)
        .with_context(|| format!("open output {}", args.output.display()))?;
    file.write_all(encrypted.as_bytes())
        .with_context(|| format!("write {}", args.output.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("write {}", args.output.display()))?;
    eprintln!("encrypted wallets written: {}", args.output.display());
    Ok(())
}

fn parse_args() -> anyhow::Result<Args> {
    let mut input = None;
    let mut output = None;
    let mut passphrase_file = None;
    let mut force = false;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--in" | "--input" => input = Some(next_path(&mut iter, &arg)?),
            "--out" | "--output" => output = Some(next_path(&mut iter, &arg)?),
            "--passphrase-file" => passphrase_file = Some(next_path(&mut iter, &arg)?),
            "--force" => force = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => bail!("unknown argument {arg}; use --help"),
        }
    }

    Ok(Args {
        input: input.context("--in is required")?,
        output: output.context("--out is required")?,
        passphrase_file,
        force,
    })
}

fn next_path(iter: &mut impl Iterator<Item = String>, arg: &str) -> anyhow::Result<PathBuf> {
    iter.next()
        .map(PathBuf::from)
        .with_context(|| format!("{arg} requires a value"))
}

fn read_passphrase(path: Option<&PathBuf>) -> anyhow::Result<String> {
    if let Some(path) = path {
        let value = fs::read_to_string(path)
            .with_context(|| format!("read passphrase file {}", path.display()))?;
        let value = value.trim_end_matches(['\r', '\n']).to_string();
        if value.is_empty() {
            bail!("passphrase file is empty");
        }
        return Ok(value);
    }
    let value = env::var("WALLETS_CONFIG_PASSPHRASE")
        .context("set WALLETS_CONFIG_PASSPHRASE or pass --passphrase-file")?;
    if value.is_empty() {
        bail!("WALLETS_CONFIG_PASSPHRASE is empty");
    }
    Ok(value)
}

fn print_help() {
    eprintln!(
        "Usage: encrypt_wallets --in wallets.json --out wallets.json.enc --passphrase-file .wallets.passphrase [--force]"
    );
}
