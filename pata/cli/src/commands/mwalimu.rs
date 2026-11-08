//! Run the Mwalimu LSP server (stdio). Used by editors for diagnostics and hover.

use super::CliResult;

const MWALIMU_USAGE: &str = r#"matumizi: pata mwalimu

Anza Mwalimu, seva ya LSP ya Asili, kwenye stdin/stdout. Mhariri (kama VS Code) huianzisha;
haiendeshwi kwa mkono.
"#;

/// Run the LSP server. Blocks until the client disconnects.
#[allow(dead_code)] // used via dispatch when user runs `pata mwalimu`
pub fn run(args: &[String]) -> CliResult {
    if args.iter().any(|a| a == "--msaada") {
        print!("{MWALIMU_USAGE}");
        return Ok(());
    }
    pata_lsp::run_stdio_blocking();
    Ok(())
}
