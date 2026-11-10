mod commands;
mod pipeline;
#[cfg(test)]
mod test_support;

use commands::{dispatch, CliError};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_build = args.first().is_some_and(|command| command == "jenga")
        && args.iter().any(|arg| arg == "--json");
    // `ASILI_FUATILIA=mti|json|binari:<faili>` traces whatever the command builds or runs.
    let result = asili_trace::install_from_env()
        .map_err(|e| CliError::new(e, 2))
        .and_then(|_| dispatch(&args));
    asili_trace::finish();
    if let Err(err) = result {
        if json_build {
            let output = serde_json::json!({
                "sawa": false,
                "makosa": [{
                    "ujumbe": err.message,
                    "msimbo": err.exit_code,
                }],
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        } else {
            eprintln!("kosa: {}", err.message);
        }
        std::process::exit(err.exit_code);
    }
}
