//! Defines the `net` cfg: networking (sockets, TLS, HTTP — the `mtandao` feature) is compiled in
//! only when the feature is on and the target is not wasm32, which has no sockets. Code for the
//! network builtins and the values they hold is gated on `net`; without it they are stubs that
//! fail with a message, as in the browser.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(net)");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    if std::env::var_os("CARGO_FEATURE_MTANDAO").is_some() && !wasm {
        println!("cargo::rustc-cfg=net");
    }
}
