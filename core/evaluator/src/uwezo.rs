//! What a runtime build can run. Builtin groups that need large libraries are optional
//! features — `mtandao` (sockets, TLS, HTTP) and `ruwaza` (regular expressions) — so a runner
//! built without them is smaller and runs the programs that call none of their builtins. A
//! runner carries the list of features it has ([`UWEZO_ALAMA`]); `pata jenga --namna release`
//! reads it from each runner's file ([`ya_tenda`]) and builds a standalone executable on the
//! smallest runner that has every feature the program needs ([`needed`]).

include!(concat!(env!("OUT_DIR"), "/uwezo.rs"));

/// Each optional feature and the builtins that need it. Every builtin is registered in every
/// build (as a stub without its feature), so builtin indices do not depend on the build.
pub const VIPENGELE: [(&str, &[&str]); 2] = [
    ("mtandao", &crate::builtins::NETWORK_BUILTINS),
    ("ruwaza", &crate::builtins::ruwaza::NAMES),
];

/// The optional features `program` needs: those any of whose builtins it calls. Builtins are
/// only reached through `CallBuiltin`.
pub fn needed(program: &crate::bytecode::BytecodeProgram) -> Vec<&'static str> {
    let names = crate::builtins::builtin_names();
    let called: std::collections::BTreeSet<&str> = program
        .functions
        .iter()
        .flat_map(|f| &f.code)
        .filter_map(|op| match op {
            crate::bytecode::Opcode::CallBuiltin(call) => {
                names.get(call.builtin as usize).map(String::as_str)
            }
            _ => None,
        })
        .collect();
    VIPENGELE
        .iter()
        .filter(|(_, builtins)| builtins.iter().any(|b| called.contains(b)))
        .map(|(feature, _)| *feature)
        .collect()
}

/// The features a runner has, read from its file (`bytes`): the list after its marker. `None`
/// when there is no marker — a runner from before runners described themselves, which has
/// every feature.
pub fn ya_tenda(bytes: &[u8]) -> Option<Vec<String>> {
    // Built here, so the reader's own code never holds the whole marker.
    let needle: Vec<u8> = [&b"\0ASILI-"[..], b"UWEZO:"].concat();
    let mut from = 0;
    while let Some(at) = bytes[from..]
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
    {
        let start = from + at + needle.len();
        let rest = &bytes[start..];
        if let Some(end) = rest.iter().position(|&b| b == b';') {
            let list = &rest[..end];
            if list.iter().all(|&b| b.is_ascii_lowercase() || b == b',') {
                let list = String::from_utf8_lossy(list);
                return Some(
                    list.split(',')
                        .filter(|f| !f.is_empty())
                        .map(String::from)
                        .collect(),
                );
            }
        }
        from = start;
    }
    None
}
