#![cfg(not(windows))]

//! HTTP/3 from Asili programs: `ChaguoSeva { h3: kweli }` on the server, `ChaguoHttp { h3: kweli }`
//! on the client.

use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::Duration;

use asili_evaluator::{run_function, Value};
use asili_lexer::tokenize;
use asili_parser::{extern_env_from_imports, parse_tokens, semantic_check_with_env, Module};

fn compile(src: &str) -> Module {
    let toks = tokenize(src).expect("tokenize");
    let module = parse_tokens(&toks).unwrap_or_else(|e| panic!("parse: {e:?}"));
    let (fns, consts) = extern_env_from_imports(&module);
    if let Err(e) = semantic_check_with_env(&module, false, fns, consts) {
        panic!("semantic check: {e:?}");
    }
    module
}

const SERVER: &str = r#"
    leta mfumo
    leta sambamba
    leta majira
    leta matumizi

    kazi mtumishi(ombi: OmbiHttp) -> JibuHttp {
        ikiwa ombi.sehemu == "/elekeza" {
            weka v = kamusi()
            v.ingiza("location", "/karibu")
            rejesha JibuHttp { hali: 302, vichwa: v, mwili: "" }
        }
        ikiwa ombi.sehemu == "/kuki" {
            weka v = kamusi()
            v.ingiza("set-cookie", "a=1; Path=/")
            rejesha JibuHttp { hali: 200, vichwa: v, mwili: "kuki" }
        }
        rejesha JibuHttp { hali: 200, vichwa: kamusi(), mwili: "karibu " + ombi.sehemu + " " + ombi.toleo + " " + ombi.vichwa.pata("cookie").angu("-") }
    }

    kazi simamisha(s: MkondoSikilizaji, udhibiti: MkondoSikilizaji) -> Tupu {
        weka _m = jaribu (udhibiti.kubali())
        s.simama()
    }

    kazi anza(anwani: Neno, anwani_ya_udhibiti: Neno, cheti: Neno, ufunguo: Neno) -> Neno {
        weka s = jaribu (mkondo_sikiliza(anwani))
        weka udhibiti = jaribu (mkondo_sikiliza(anwani_ya_udhibiti))
        weka tls = jaribu (tls_sanidi(cheti, ufunguo))
        weka uzi = jaribu (tenda("simamisha", s, udhibiti))
        jaribu (mkondo_tumikia_http(s, "mtumishi", 1, Chaguo::Kuna(tls), ChaguoSeva { h3: kweli }))
        jaribu (subiri_tenda(uzi))
        rejesha "imesimama"
    }

    kazi mteja(url: Neno, cheti: Neno) -> Neno {
        weka j = jaribu (http_ombi("GET", url, ChaguoHttp { h3: kweli, cheti_ca: cheti }))
        rejesha (j.hali kama Neno) + " " + j.toleo + " " + j.mwili
    }
"#;

fn free_ports() -> (String, String) {
    let (server, udp, server_addr) = loop {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let server_addr = server.local_addr().unwrap();
        match UdpSocket::bind(server_addr) {
            Ok(udp) => break (server, udp, server_addr),
            Err(_) => continue,
        }
    };
    let control = TcpListener::bind("127.0.0.1:0").unwrap();
    let control_addr = control.local_addr().unwrap();
    assert_ne!(server_addr, control_addr);
    drop((server, udp, control));
    (server_addr.to_string(), control_addr.to_string())
}

#[test]
fn http3_server_and_client_in_asili() {
    let ck =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let dir = std::env::temp_dir();
    let cert = dir.join(format!("asili-h3-cert-{}.pem", std::process::id()));
    let key = dir.join(format!("asili-h3-key-{}.pem", std::process::id()));
    std::fs::write(&cert, ck.cert.pem()).unwrap();
    std::fs::write(&key, ck.key_pair.serialize_pem()).unwrap();
    let (cert, key) = (cert.display().to_string(), key.display().to_string());

    let (addr, control) = free_ports();
    let server = {
        let (addr, control, cert, key) = (addr.clone(), control.clone(), cert.clone(), key.clone());
        std::thread::spawn(move || {
            let args = vec![
                Value::neno(addr),
                Value::neno(control),
                Value::neno(cert),
                Value::neno(key),
            ];
            run_function(&compile(SERVER), "anza", args)
                .map(|v| format!("{v:?}"))
                .map_err(|e| format!("{e:?}"))
        })
    };
    let port = addr.rsplit(':').next().unwrap().to_string();
    let listening = (0..200).any(|_| {
        if TcpStream::connect(&addr).is_ok() {
            true
        } else {
            std::thread::sleep(Duration::from_millis(10));
            false
        }
    });
    assert!(listening, "server did not listen on {addr}");

    let client = compile(&format!(
        r#"
        leta mfumo
        kazi mteja(url: Neno, cheti: Neno) -> Neno {{
            weka j = jaribu (http_ombi("GET", url, ChaguoHttp {{ h3: kweli, cheti_ca: cheti }}))
            rejesha (j.hali kama Neno) + " " + j.toleo + " " + j.mwili
        }}
        "#
    ));
    let out = run_function(
        &client,
        "mteja",
        vec![
            Value::neno(format!("https://127.0.0.1:{port}/njia")),
            Value::neno(cert.clone()),
        ],
    )
    .expect("runs");
    assert_eq!(
        out,
        Value::neno("200 HTTP/3 karibu /njia HTTP/3 -".to_string())
    );

    // Redirects are followed, and cookies set over HTTP/3 are sent back over it.
    let policy = compile(&format!(
        r#"
        leta mfumo
        kazi jaribu(base: Neno, cheti: Neno) -> Neno {{
            weka a = jaribu (http_ombi("GET", base + "/elekeza", ChaguoHttp {{ h3: kweli, cheti_ca: cheti.clona() }}))
            jaribu (http_ombi("GET", base + "/kuki", ChaguoHttp {{ h3: kweli, cheti_ca: cheti.clona(), vidakuzi: kweli }}))
            weka b = jaribu (http_ombi("GET", base + "/x", ChaguoHttp {{ h3: kweli, cheti_ca: cheti.clona(), vidakuzi: kweli }}))
            rejesha a.mwili + " | " + b.mwili
        }}
        "#
    ));
    let out = run_function(
        &policy,
        "jaribu",
        vec![
            Value::neno(format!("https://127.0.0.1:{port}")),
            Value::neno(cert.clone()),
        ],
    )
    .expect("runs");
    assert_eq!(
        out,
        Value::neno("karibu /karibu HTTP/3 - | karibu /x HTTP/3 a=1".to_string())
    );

    drop(TcpStream::connect(&control).expect("control"));
    let out = server.join().unwrap().expect("server runs");
    assert!(out.contains("imesimama"), "{out}");
    let _ = (std::fs::remove_file(&cert), std::fs::remove_file(&key));
}
