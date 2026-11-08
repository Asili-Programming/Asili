pub mod fuatilia;
pub mod jaribu;
pub mod jenga;
pub mod kagua;
pub mod mwalimu;
pub mod nadhifu;
pub mod njozi;
pub mod ondoa;
pub mod ongeza;
pub mod repl;
pub mod tenda;
pub mod thibitisha;

#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub exit_code: i32,
}

impl CliError {
    pub fn new(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }
}

/// `pata_core::Error` has no concept of a process exit code (a shared library consumed by an
/// LSP server can't have one) — every conversion into a CLI-facing `CliError` uses exit code 1,
/// matching how every other I/O/parse failure in this crate's command layer already exits.
impl From<pata_core::Error> for CliError {
    fn from(err: pata_core::Error) -> Self {
        CliError::new(err.message, 1)
    }
}

pub type CliResult = Result<(), CliError>;

/// `--fuatilia` / `--fuatilia=<namna>`: trace this command (`mti` unless a format is named; see
/// `asili_trace::install_spec`). `None` when `arg` is not the flag.
pub(crate) fn trace_flag(arg: &str) -> Option<CliResult> {
    let spec = match arg {
        "--fuatilia" => "mti",
        _ => arg.strip_prefix("--fuatilia=")?,
    };
    Some(asili_trace::install_spec(spec).map_err(|e| CliError::new(e, 2)))
}

#[cfg(test)]
pub static TEST_CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn dispatch(args: &[String]) -> CliResult {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::new(usage(), 2));
    };

    match command.as_str() {
        "njozi" => njozi::run(rest),
        "ongeza" => ongeza::run(rest),
        "ondoa" => ondoa::run(rest),
        "jenga" => jenga::run(rest),
        "jaribu" => jaribu::run(rest),
        "mwalimu" => mwalimu::run(rest),
        "nadhifu" => nadhifu::run(rest),
        "kagua" => kagua::run(rest),
        "repl" => repl::run(rest),
        "tenda" => tenda::run(rest),
        "fuatilia" => fuatilia::run(rest),
        "thibitisha" => thibitisha::run(rest),
        "--msaada" | "msaada" => {
            println!("{}", usage());
            Ok(())
        }
        _ => Err(CliError::new(format!("amri isiyotambuliwa: {command}"), 2)),
    }
}

fn usage() -> &'static str {
    r#"matumizi: pata <amri> [chagua...] [hoja...]

Amri ni kitendo unachotaka kufanya. Chagua na hoja hutofautiana kwa kila amri.

Amri:
  jenga [faili.as] [chagua...]  Jenga mradi (kutoka pata.toml) au faili moja.
  tenda <path.asb|manifest>      Tenda kilele bila kujenga upya.
  fuatilia <faili>             Soma ufuatiliaji wa binari na kuuonyesha kama mti.
  jaribu [chagua...]            Endesha majaribio (#[jaribio]).
  mwalimu                      Anza seva ya LSP (Mwalimu).
  repl                         Fungua REPL.
  njozi [jina]                 Unda mradi mpya.
  ongeza <lib> [chagua...]     Ongeza tegemezi.
  ondoa <lib>                  Ondoa tegemezi.
  nadhifu [chagua...]          Nadhifisha chanzo.
  kagua [njia] [chagua...]     Kagua mtindo na mazoea ya chanzo.
  thibitisha [chagua...]       Thibitisha mradi.

Onyesha msaada kwa amri: pata <amri> --msaada.
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command names listed under "Amri:" in `pata --msaada`, so a new command is checked
    /// as soon as it is listed there.
    fn listed_commands() -> Vec<String> {
        usage()
            .lines()
            .skip_while(|l| !l.starts_with("Amri:"))
            .skip(1)
            .take_while(|l| !l.trim().is_empty())
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect()
    }

    #[test]
    fn every_listed_command_answers_msaada() {
        let commands = listed_commands();
        assert!(
            commands.len() >= 12,
            "expected the full command list, got {commands:?}"
        );
        // No command may touch the project, start a server or read stdin for `--msaada`, so this
        // runs from wherever the test runner is, with no project around it.
        for command in commands {
            let result = dispatch(&[command.clone(), "--msaada".to_string()]);
            assert!(
                result.is_ok(),
                "`pata {command} --msaada` failed: {:?}",
                result.err().map(|e| e.message)
            );
        }
    }
}
