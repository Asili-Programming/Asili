//! The build's optional features, for the code and for anyone holding the runner it ends up in:
//!
//! - the `net` cfg: networking (sockets, TLS, HTTP — the `mtandao` feature) is compiled in only
//!   when the feature is on and the target is not wasm32, which has no sockets. Code for the
//!   network builtins and the values they hold is gated on `net`; without it they are stubs that
//!   fail with a message, as in the browser;
//! - `$OUT_DIR/uwezo.rs`, included by `uwezo`: the features this build has (`mtandao`, `ruwaza`),
//!   as a list and as a marker the runner carries, from which `pata` reads what a runner can run.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(net)");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let net = std::env::var_os("CARGO_FEATURE_MTANDAO").is_some() && !wasm;
    if net {
        println!("cargo::rustc-cfg=net");
    }
    let mut uwezo = Vec::new();
    if net {
        uwezo.push("mtandao");
    }
    if std::env::var_os("CARGO_FEATURE_RUWAZA").is_some() {
        uwezo.push("ruwaza");
    }
    let list = uwezo
        .iter()
        .map(|u| format!("{u:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("uwezo.rs");
    std::fs::write(
        out,
        format!(
            "/// The optional features this build has.\n\
             pub const UWEZO: &[&str] = &[{list}];\n\
             /// [`UWEZO`] as the runner carries it, for [`ya_tenda`] to read from its file.\n\
             pub const UWEZO_ALAMA: &[u8] = b\"\\0ASILI-UWEZO:{};\";\n",
            uwezo.join(",")
        ),
    )
    .expect("write uwezo.rs");
}
