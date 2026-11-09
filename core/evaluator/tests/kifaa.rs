//! The device targets compute what native code on the build machine computes, bit for bit:
//! strict functions are built for the device (`nguvu::device`), linked with a generated C
//! harness and the device runtime, run under an emulator, and every result — or failure — is
//! compared with the same call made through native code here.
//!
//! * Cortex-M: linked with `driver/kifaa`, run under `qemu-arm -cpu cortex-m7`. Needs `clang`,
//!   `ld.lld`, `qemu-arm-static` and the `thumbv7em-none-eabihf` Rust target; CI installs them
//!   (`.github/workflows/ci.yml`).
//! * ESP32: an ESP-IDF firmware holding nothing but the harness and the generated object (the
//!   ESP32 needs no runtime library of Asili's own), run under Espressif's `qemu-system-xtensa`. Needs `ASILI_ESP_IDF` set to an ESP-IDF checkout whose
//!   tools (including `qemu-xtensa`) are installed; set `ASILI_ESP32_REQUIRE` to fail instead
//!   of skipping when it is missing.
//!
//! A target whose tools are missing is skipped with a note.

use asili_evaluator::nguvu::{self, device};
use asili_evaluator::{compile_module_explained, run_bytecode_function_on, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/strict_gen.rs"]
mod strict_gen;

/// An argument of a test call.
#[derive(Clone)]
enum Arg {
    Num(f64),
    List(Vec<f64>),
}

fn tool(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The device runtime, built for Cortex-M in its own target directory (the test's own build
/// holds the workspace's lock).
fn runtime() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::temp_dir().join("asili-kifaa-target");
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "-q",
            "-p",
            "asili-kifaa",
            "--target",
            "thumbv7em-none-eabihf",
        ])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .ok()?;
    status
        .success()
        .then(|| target.join("thumbv7em-none-eabihf/release/libasili_kifaa.a"))
}

/// What a call gives: the result's bits, or a failure.
fn host_result(
    library: &asili_evaluator::aot::NativeLibrary,
    program: &asili_evaluator::BytecodeProgram,
    name: &str,
    args: &[Arg],
) -> String {
    let values = args
        .iter()
        .map(|a| match a {
            Arg::Num(n) => Value::Namba(*n),
            Arg::List(items) => Value::Orodha(std::rc::Rc::new(
                items.iter().map(|&n| Value::Namba(n)).collect(),
            )),
        })
        .collect();
    match run_bytecode_function_on(library, program, name, values) {
        Ok(Value::Namba(n)) => format!("{:016x}", canon(n)),
        Ok(Value::Ukweli(b)) => format!("{:016x}", (b as u8 as f64).to_bits()),
        Ok(other) => format!("{other:?}"),
        Err(_) => "kosa".to_string(),
    }
}

/// Every NaN prints alike (their sign and payload bits are not observable in Asili).
fn canon(n: f64) -> u64 {
    if n.is_nan() {
        f64::NAN.to_bits()
    } else {
        n.to_bits()
    }
}

const SYS: &str = "
    .syntax unified
    .thumb
    .global sys3
    .type sys3, %function
    .thumb_func
sys3:
    push {r7, lr}
    mov r7, r0
    mov r0, r1
    mov r1, r2
    mov r2, r3
    svc #0
    pop {r7, pc}
";

/// The C statements making `calls` (each with fresh copies of its lists and printing its
/// result through the harness's `result`), with `lit` writing a double as a C constant.
fn call_statements(calls: &[(&str, Vec<Arg>)], lit: impl Fn(f64) -> String) -> String {
    let mut c = String::new();
    for (k, (f, args)) in calls.iter().enumerate() {
        let mut fields = Vec::new();
        for (j, a) in args.iter().enumerate() {
            match a {
                Arg::Num(n) => fields.push(lit(*n)),
                Arg::List(items) => {
                    let elems: Vec<String> = items.iter().map(|&n| lit(n)).collect();
                    c.push_str(&format!(
                        "    static double l{k}_{j}[] = {{ {} }};\n",
                        if elems.is_empty() {
                            "0".to_string()
                        } else {
                            elems.join(", ")
                        }
                    ));
                    fields.push(format!("{{ l{k}_{j}, {} }}", items.len()));
                }
            }
        }
        // Each call's arguments in a block of their own, so the harness's frame stays one call's
        // worth however many calls there are (a device's main stack is small).
        let (decl, hoja) = if args.is_empty() {
            (String::new(), "0".to_string())
        } else {
            (
                format!("asili_{f}_hoja h{k} = {{ {} }}; ", fields.join(", ")),
                format!("&h{k}"),
            )
        };
        c.push_str(&format!(
            "    {{ {decl}double out = 0; int s = asili_{f}({hoja}, &out); result(s, out); }}\n"
        ));
    }
    c
}

/// A double as an exact C99 hexadecimal constant (GCC's C has no `__builtin_bit_cast`).
fn hex_double(x: f64) -> String {
    if x.is_nan() {
        return "__builtin_nan(\"\")".into();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "__builtin_inf()".into()
        } else {
            "(-__builtin_inf())".into()
        };
    }
    let b = x.to_bits();
    let sign = if b >> 63 == 1 { "-" } else { "" };
    let e = ((b >> 52) & 0x7ff) as i64;
    let m = b & ((1u64 << 52) - 1);
    if e == 0 {
        format!("{sign}0x0.{m:013x}p-1022")
    } else {
        format!("{sign}0x1.{m:013x}p{}", e - 1023)
    }
}

/// Every line the device printed must equal the host's.
fn compare(
    target: &str,
    name: &str,
    expected: &str,
    got: &str,
    source: &str,
    calls: &[(&str, Vec<Arg>)],
) {
    if std::env::var_os("ASILI_KIFAA_SHOW").is_some() {
        eprintln!("{name} ({target}):\n{got}");
    }
    for (i, (want, have)) in expected.lines().zip(got.lines()).enumerate() {
        assert_eq!(
            want, have,
            "{name}: call {i} ({}) differs on {target}\n{source}",
            calls[i].0
        );
    }
    assert_eq!(
        expected.lines().count(),
        got.lines().count(),
        "{target}: {got}"
    );
}

/// Build `source`'s strict functions for each device target, run `calls` there and here, and
/// compare.
fn agree(name: &str, source: &str, calls: &[(&str, Vec<Arg>)]) {
    let tokens = asili_lexer::tokenize(source).expect("tokenize");
    let module = asili_parser::parse_tokens(&tokens).expect("parse");
    let program = compile_module_explained(&module).expect("bytecode");
    let strict: Vec<usize> = module
        .functions
        .iter()
        .filter(|f| asili_evaluator::salama::is_strict(f))
        .map(|f| {
            program
                .functions
                .iter()
                .position(|g| g.name == f.name)
                .expect("compiled")
        })
        .collect();
    let library = nguvu::compile(&program).expect("native code");
    let mut expected = String::new();
    for (f, args) in calls {
        expected.push_str(&host_result(&library, &program, f, args));
        expected.push('\n');
    }
    agree_cortex_m(name, &program, &strict, &expected, source, calls);
    agree_esp32(name, &program, &strict, &expected, source, calls);
}

fn agree_cortex_m(
    name: &str,
    program: &asili_evaluator::BytecodeProgram,
    strict: &[usize],
    expected: &str,
    source: &str,
    calls: &[(&str, Vec<Arg>)],
) {
    // CI sets ASILI_KIFAA_REQUIRE so a missing tool fails the job instead of skipping it.
    let skip = |why: String| {
        assert!(std::env::var_os("ASILI_KIFAA_REQUIRE").is_none(), "{why}");
        eprintln!("kifaa: {why}; jaribio limerukwa");
    };
    let tools = ["clang", "ld.lld", "qemu-arm-static"];
    if let Some(missing) = tools.iter().find(|t| !tool(t)) {
        return skip(format!("{missing} haipo"));
    }
    let Some(runtime) = runtime() else {
        return skip("lengo thumbv7em-none-eabihf halipo".into());
    };
    let built = device::build(program, strict, name).unwrap_or_else(|e| panic!("{e}"));

    let dir = std::env::temp_dir().join(format!("asili-kifaa-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}-cortex-m.o")), &built.object).unwrap();
    std::fs::write(dir.join(format!("{name}.h")), &built.header).unwrap();
    std::fs::write(dir.join("sys.s"), SYS).unwrap();

    // The harness: each call with fresh copies of its lists, printing the result's bits.
    let mut c = format!(
        "#include \"{name}.h\"\n\
         long sys3(long n, long a, long b, long c);\n\
         static void put(const char *s, int n) {{ sys3(4, 1, (long)s, n); }}\n\
         static void hex(const void *p) {{\n\
             unsigned long long v = *(const unsigned long long *)p; char b[17];\n\
             for (int i = 15; i >= 0; i--) {{ b[i] = \"0123456789abcdef\"[v & 15]; v >>= 4; }}\n\
             b[16] = '\\n'; put(b, 17);\n\
         }}\n\
         static void result(int s, double out) {{\n\
             if (s != 0) {{ put(\"kosa\\n\", 5); return; }}\n\
             if (out != out) {{ out = __builtin_nan(\"\"); }}\n\
             hex(&out);\n\
         }}\n\
         void _start(void) {{\n"
    );
    c.push_str(&call_statements(calls, |n| {
        format!("__builtin_bit_cast(double, {}ULL)", n.to_bits())
    }));
    c.push_str("    sys3(1, 0, 0, 0);\n}\n");
    std::fs::write(dir.join("main.c"), c).unwrap();

    let run = |cmd: &mut Command| {
        let out = cmd.current_dir(&dir).output().expect("tool runs");
        assert!(
            out.status.success(),
            "{cmd:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let flags = [
        "--target=thumbv7em-none-eabihf",
        "-mfpu=fpv5-d16",
        "-mfloat-abi=hard",
        "-O1",
        "-ffreestanding",
        "-c",
    ];
    run(Command::new("clang")
        .args(flags)
        .args(["main.c", "-o", "main.o"]));
    run(Command::new("clang")
        .args(flags)
        .args(["sys.s", "-o", "sys.o"]));
    run(Command::new("ld.lld")
        .args(["-static", "-e", "_start", "main.o", "sys.o"])
        .arg(format!("{name}-cortex-m.o"))
        .arg(&runtime)
        .args(["-o", "t.elf"]));
    let got = run(Command::new("qemu-arm-static").args(["-cpu", "cortex-m7", "t.elf"]));
    compare("Cortex-M", name, expected, &got, source, calls);
    let _ = std::fs::remove_dir_all(&dir);
}

/// ESP32 builds share one lock: each is a full ESP-IDF build, too heavy to run side by side.
static ESP32: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The default ESP32 efuse image `idf.py qemu` writes (chip revision 3).
fn esp32_efuse() -> Vec<u8> {
    let mut e = vec![0u8; 124];
    e[13] = 0x80;
    e[22] = 0x10;
    e
}

fn agree_esp32(
    name: &str,
    program: &asili_evaluator::BytecodeProgram,
    strict: &[usize],
    expected: &str,
    source: &str,
    calls: &[(&str, Vec<Arg>)],
) {
    let Some(idf) = std::env::var_os("ASILI_ESP_IDF").map(PathBuf::from) else {
        assert!(
            std::env::var_os("ASILI_ESP32_REQUIRE").is_none(),
            "ASILI_ESP_IDF haijawekwa"
        );
        eprintln!("kifaa: ASILI_ESP_IDF haijawekwa; jaribio la ESP32 limerukwa");
        return;
    };
    let built = device::build_for(device::Lengo::Esp32, program, strict, name)
        .unwrap_or_else(|e| panic!("{e}"));
    let _one_at_a_time = ESP32.lock().unwrap_or_else(|e| e.into_inner());

    // A project kept between runs, so later builds are incremental.
    let dir = std::env::temp_dir().join(format!("asili-kifaa-esp32-{name}"));
    let main = dir.join("main");
    std::fs::create_dir_all(&main).unwrap();
    let write = |path: PathBuf, bytes: &[u8]| {
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            std::fs::write(path, bytes).unwrap();
        }
    };
    write(
        dir.join("CMakeLists.txt"),
        b"cmake_minimum_required(VERSION 3.16)\n\
          include($ENV{IDF_PATH}/tools/cmake/project.cmake)\n\
          project(asili_kifaa)\n",
    );
    // The task watchdog would print into the results of a long run. The main task keeps
    // ESP-IDF's default stack: the frames must fit it.
    write(
        dir.join("sdkconfig.defaults"),
        b"CONFIG_ESP_TASK_WDT_INIT=n\nCONFIG_ESP_INT_WDT=n\n",
    );
    write(
        main.join("CMakeLists.txt"),
        format!(
            "idf_component_register(SRCS \"main.c\" INCLUDE_DIRS \".\")\n\
             target_sources(${{COMPONENT_LIB}} PRIVATE \"${{CMAKE_CURRENT_SOURCE_DIR}}/{name}-esp32.o\")\n"
        )
        .as_bytes(),
    );
    let _ = std::fs::remove_file(main.join("asili_kifaa_esp32.c"));
    write(main.join(format!("{name}-esp32.o")), &built.object);
    write(main.join(format!("{name}.h")), built.header.as_bytes());
    let mut c = format!(
        "#include <stdio.h>\n#include <string.h>\n#include \"{name}.h\"\n\
         static void result(int s, double out) {{\n\
             if (s != 0) {{ printf(\"kosa\\n\"); return; }}\n\
             if (out != out) {{ out = __builtin_nan(\"\"); }}\n\
             unsigned long long v; memcpy(&v, &out, 8);\n\
             printf(\"%016llx\\n\", v);\n\
         }}\n\
         void app_main(void) {{\n\
             printf(\"ASILI-MWANZO\\n\");\n"
    );
    c.push_str(&call_statements(calls, hex_double));
    c.push_str("    printf(\"ASILI-MWISHO\\n\");\n    fflush(stdout);\n}\n");
    write(main.join("main.c"), c.as_bytes());

    let sh = |script: &str| -> std::process::Output {
        Command::new("bash")
            .arg("-c")
            .arg(format!(
                ". '{}/export.sh' >/dev/null 2>&1 && {script}",
                idf.display()
            ))
            .current_dir(&dir)
            .output()
            .expect("bash runs")
    };
    let check = |what: &str, out: std::process::Output| {
        assert!(
            out.status.success(),
            "{what}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let defaults = std::fs::read(dir.join("sdkconfig.defaults")).unwrap();
    let stamp = dir.join("sdkconfig.defaults.applied");
    if !dir.join("sdkconfig").exists() || std::fs::read(&stamp).ok() != Some(defaults.clone()) {
        let _ = std::fs::remove_file(dir.join("sdkconfig"));
        check("idf.py set-target", sh("idf.py set-target esp32"));
        std::fs::write(&stamp, &defaults).unwrap();
    }
    check("idf.py build", sh("idf.py build"));
    check(
        "esptool merge-bin",
        sh("python -m esptool --chip esp32 merge-bin -o build/flash.bin --pad-to-size 2MB \
            --flash-mode dio --flash-freq 40m --flash-size 2MB \
            0x1000 build/bootloader/bootloader.bin 0x8000 build/partition_table/partition-table.bin \
            0x10000 build/asili_kifaa.bin"),
    );
    std::fs::write(dir.join("build/efuse.bin"), esp32_efuse()).unwrap();

    // QEMU runs until killed: read its console until the harness's end marker.
    let mut child = Command::new("bash")
        .arg("-c")
        .arg(format!(
            ". '{}/export.sh' >/dev/null 2>&1 && exec qemu-system-xtensa -M esp32 -m 4M \
             -drive file=build/flash.bin,if=mtd,format=raw \
             -drive file=build/efuse.bin,if=none,format=raw,id=efuse \
             -global driver=nvram.esp32.efuse,property=drive,value=efuse \
             -global driver=timer.esp32.timg,property=wdt_disable,value=true -nographic",
            idf.display()
        ))
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("qemu-system-xtensa starts");
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut got = String::new();
    let mut started = false;
    let mut finished = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        let Ok(line) = rx.recv_timeout(left) else {
            break;
        };
        let line = line.trim_end_matches('\r');
        if line == "ASILI-MWANZO" {
            started = true;
        } else if line == "ASILI-MWISHO" {
            finished = true;
            break;
        } else if started {
            got.push_str(line);
            got.push('\n');
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(finished, "ESP32: harness haikufika mwisho\n{got}");
    compare("ESP32", name, expected, &got, source, calls);
}

fn n(x: f64) -> Arg {
    Arg::Num(x)
}

// Verifies: REQ-TGT-2, REQ-RUN-1
#[test]
fn the_controller_example_agrees() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/kidhibiti/src/kuu.as"
    ))
    .unwrap();
    let samples = |base: f64| Arg::List((0..8).map(|i| base + i as f64 * 1.5).collect());
    agree(
        "kidhibiti",
        &source,
        &[
            ("wastani", vec![samples(0.0)]),
            ("dhibiti", vec![samples(0.0), n(5.0)]),
            ("dhibiti", vec![samples(-30.0), n(5.0)]),
            ("dhibiti", vec![samples(40.0), n(5.0)]),
            ("dhibiti", vec![samples(1e300), n(-0.0)]),
            ("dhibiti", vec![samples(0.0), n(f64::NAN)]),
            // Too short a list: an index out of range fails on both — at the first index or
            // only at the last (the checks of the unrolled loop are one check on the device).
            ("wastani", vec![Arg::List(vec![1.0, 2.0])]),
            ("wastani", vec![Arg::List((0..7).map(f64::from).collect())]),
            ("wastani", vec![Arg::List((0..9).map(f64::from).collect())]),
        ],
    );
}

// Verifies: REQ-TGT-2
#[test]
fn integers_bits_and_calls_agree() {
    let source = r#"
        #[salama]
        kazi hatua(x: Namba, k: Namba) -> Namba {
            weka jumla = 0
            kwa i kutoka 0 hadi 40 {
                jumla += (x * i) // (k + 1) + (x * i) % (k + 3)
                ikiwa jumla > 100000000000 { jumla -= 99999999999 }
            }
            rejesha jumla
        }

        #[salama]
        kazi biti(a: Namba, b: Namba) -> Namba {
            weka r = 0
            kwa s kutoka 0 hadi 64 {
                r = r ^ ((a << s) | (b >> s)) & 4294967295
                r += (a & b) + (a | 3) - (b ^ 5)
            }
            rejesha r
        }

        #[salama]
        kazi mraba(x: Namba) -> Namba {
            rejesha x * x
        }

        #[salama]
        kazi mchanganyiko(x: Namba, y: Namba) -> Namba {
            weka a = mraba(x) + mraba(y)
            ikiwa a > 50 && x < y {
                rejesha sakafu(a / 3) + dari(y / 7) + x % 4 + y ** 2
            }
            rejesha a - 1
        }

        #[salama]
        kazi ni_kubwa(x: Namba) -> Ukweli {
            rejesha x > 10
        }

        #[salama]
        kazi panga(l: Orodha<Namba>) -> Namba {
            weka jumla = 0
            kwa i kutoka 0 hadi 6 {
                kwa j kutoka 0 hadi 5 {
                    ikiwa l[j] > l[j + 1] {
                        weka t = l[j]
                        l[j] = l[j + 1]
                        l[j + 1] = t
                    }
                }
            }
            kwa i kutoka 0 hadi 6 { jumla = jumla * 10 + l[i] }
            rejesha jumla
        }
    "#;
    let big = 9007199254740990.0;
    let mut calls: Vec<(&str, Vec<Arg>)> = Vec::new();
    for (x, k) in [
        (3.0, 2.0),
        (-7.0, 4.0),
        (1e6, 0.0),
        (big, 7.0),
        (-big, 1.0),
        (0.5, 2.0),
    ] {
        calls.push(("hatua", vec![n(x), n(k)]));
    }
    for (a, b) in [
        (5.0, 9.0),
        (-1.0, 3.0),
        (4294967296.0, 65535.0),
        (-5.9, 123456789.0),
        (big, -big),
        (f64::NAN, 1.0),
        (f64::INFINITY, 2.0),
    ] {
        calls.push(("biti", vec![n(a), n(b)]));
    }
    for (x, y) in [
        (3.0, 9.0),
        (8.0, 2.0),
        (-4.5, 7.25),
        (1e10, 3e10),
        (0.0, -0.0),
    ] {
        calls.push(("mchanganyiko", vec![n(x), n(y)]));
    }
    for x in [3.0, 11.0, f64::NAN] {
        calls.push(("ni_kubwa", vec![n(x)]));
    }
    calls.push(("panga", vec![Arg::List(vec![5.0, 3.0, 9.0, 1.0, 4.0, 2.0])]));
    // Stores between the index checks: too short a list fails after some of them.
    calls.push(("panga", vec![Arg::List(vec![5.0, 3.0, 9.0, 1.0])]));
    agree("hesabu", source, &calls);
}

// Verifies: REQ-TGT-2
#[test]
fn random_strict_programs_agree() {
    let count: u64 = std::env::var("ASILI_KIFAA_PROGRAMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    // In batches: one image holding hundreds of programs outgrows an ESP32 app partition
    // (code with every value in a stack slot is large).
    let seeds: Vec<u64> = (1..=count).collect();
    for batch in seeds.chunks(50) {
        let mut source = String::new();
        let mut calls: Vec<(String, Vec<Arg>)> = Vec::new();
        for &seed in batch {
            let (text, name) = strict_gen::function(seed);
            source.push_str(&text);
            let mut rng = strict_gen::Rng::new(seed ^ 0xABCD);
            for _ in 0..3 {
                let list: Vec<f64> = (0..8).map(|_| strict_gen::value(&mut rng)).collect();
                let a = strict_gen::value(&mut rng);
                let b = strict_gen::value(&mut rng);
                calls.push((name.clone(), vec![Arg::List(list), n(a), n(b)]));
            }
        }
        let calls: Vec<(&str, Vec<Arg>)> =
            calls.iter().map(|(f, a)| (f.as_str(), a.clone())).collect();
        agree("nasibu", &source, &calls);
    }
}

/// Builds for the device only (no tools needed): what a device cannot run is a build error
/// naming the function and the reason.
// Verifies: REQ-TGT-3
#[test]
fn what_a_device_cannot_run_is_a_build_error() {
    let build = |source: &str| -> Result<(), String> {
        let tokens = asili_lexer::tokenize(source).expect("tokenize");
        let module = asili_parser::parse_tokens(&tokens).expect("parse");
        let program = compile_module_explained(&module).expect("bytecode");
        let strict: Vec<usize> = module
            .functions
            .iter()
            .filter(|f| asili_evaluator::salama::is_strict(f))
            .filter_map(|f| program.functions.iter().position(|g| g.name == f.name))
            .collect();
        device::build(&program, &strict, "t").map(|_| ())
    };
    // A callee that writes the list it receives (inlined on a device, it would write the
    // caller's list).
    let e = build(
        "#[salama]\nkazi weka_sifuri(l: Orodha<Namba>) -> Namba {\n    l[0] = 0\n    rejesha 1\n}\n\n\
         #[salama]\nkazi f(l: Orodha<Namba>) -> Namba {\n    rejesha weka_sifuri(l)\n}\n",
    )
    .unwrap_err();
    assert!(
        e.contains("kazi 'f' haiwezi kujengwa kwa Cortex-M")
            && e.contains("inayoandika kwenye orodha"),
        "{e}"
    );
    // Within the subset a device runs, the same shapes build.
    build(
        "#[salama]\nkazi soma(l: Orodha<Namba>) -> Namba {\n    rejesha l[0] * 2\n}\n\n\
         #[salama]\nkazi f(l: Orodha<Namba>, x: Namba) -> Namba {\n    rejesha soma(l) + x\n}\n",
    )
    .unwrap();
}

/// Builds for the ESP32 only (no tools needed): the object needs no runtime library of Asili's
/// own, and values that are never live together share stack slots, so a function with unrolled
/// loops keeps a small frame (it once needed 4.6 KB, more than ESP-IDF's default main stack).
#[test]
fn esp32_objects_need_no_runtime_and_keep_small_frames() {
    let source = r#"
        #[salama]
        kazi panga(l: Orodha<Namba>) -> Namba {
            weka jumla = 0
            kwa i kutoka 0 hadi 6 {
                kwa j kutoka 0 hadi 5 {
                    ikiwa l[j] > l[j + 1] {
                        weka t = l[j]
                        l[j] = l[j + 1]
                        l[j + 1] = t
                    }
                }
            }
            kwa i kutoka 0 hadi 6 { jumla = jumla * 10 + l[i] }
            rejesha jumla + sakafu(l[0] / 3) + (l[1] << 2)
        }
    "#;
    let tokens = asili_lexer::tokenize(source).expect("tokenize");
    let module = asili_parser::parse_tokens(&tokens).expect("parse");
    let program = compile_module_explained(&module).expect("bytecode");
    let strict: Vec<usize> = (0..program.functions.len())
        .filter(|&i| program.functions[i].name == "panga")
        .collect();
    let built = device::build_for(device::Lengo::Esp32, &program, &strict, "t")
        .unwrap_or_else(|e| panic!("{e}"));
    let object = &built.object;
    assert!(
        !object.windows(11).any(|w| w == b"asili_kifaa"),
        "the ESP32 object must not call the device runtime"
    );
    // The stack a call takes, as the build reports it: well within ESP-IDF's default main
    // task stack (3.5 KB).
    assert_eq!(built.stack.len(), 1);
    assert!(built.stack[0].1 <= 1024, "stack: {:?}", built.stack);
}

/// The program's functions named in `names`, by index.
fn indices(program: &asili_evaluator::BytecodeProgram, names: &[&str]) -> Vec<usize> {
    names
        .iter()
        .map(|n| {
            program
                .functions
                .iter()
                .position(|g| g.name == *n)
                .expect("compiled")
        })
        .collect()
}

fn bytecode(source: &str) -> asili_evaluator::BytecodeProgram {
    let tokens = asili_lexer::tokenize(source).expect("tokenize");
    let module = asili_parser::parse_tokens(&tokens).expect("parse");
    compile_module_explained(&module).expect("bytecode")
}

// Verifies: REQ-TGT-2
#[test]
fn esp32_drives_pins_and_waits() {
    // `#[wazi]` code on the ESP32: a pin written through the GPIO registers and a wait on the
    // cycle counter, giving what native code (on the virtual board) gives.
    let source = r#"
        leta ubao
        #[wazi]
        kazi washa(hali: Ukweli, ms: Namba) -> Namba {
            ubao_andika(2, hali)
            ubao_subiri(ms)
            rejesha ms * 2
        }
    "#;
    let program = bytecode(source);
    let exports = indices(&program, &["washa"]);
    let calls: Vec<(&str, Vec<Arg>)> = vec![
        ("washa", vec![n(1.0), n(3.0)]),
        ("washa", vec![n(0.0), n(0.0)]),
        ("washa", vec![n(1.0), n(-4.0)]),
        ("washa", vec![n(0.0), n(1.5)]),
    ];
    let library = nguvu::compile(&program).expect("native code");
    let mut expected = String::new();
    for (f, args) in &calls {
        expected.push_str(&host_result(&library, &program, f, args));
        expected.push('\n');
    }
    agree_esp32("ubao", &program, &exports, &expected, source, &calls);
}

#[test]
fn board_calls_a_device_cannot_make_are_build_errors() {
    let esp32 = |source: &str| {
        let program = bytecode(source);
        let f = indices(&program, &["f"]);
        device::build_for(device::Lengo::Esp32, &program, &f, "t").map(|_| ())
    };
    let pin = |p: &str| {
        format!("leta ubao\n#[wazi]\nkazi f() -> Namba {{\n    ubao_andika({p}, kweli)\n    rejesha 0\n}}\n")
    };
    // A pin the build cannot see.
    let e = esp32("leta ubao\n#[wazi]\nkazi f(p: Namba) -> Namba {\n    ubao_andika(p, kweli)\n    rejesha 0\n}\n").unwrap_err();
    assert!(e.contains("namba thabiti"), "{e}");
    // The flash's pins, a pin that does not exist, an input-only pin written.
    assert!(esp32(&pin("6")).unwrap_err().contains("flash"));
    assert!(esp32(&pin("24")).unwrap_err().contains("haina pini 24"));
    assert!(esp32(&pin("2.5")).unwrap_err().contains("haina pini"));
    assert!(esp32(&pin("35")).unwrap_err().contains("kusoma tu"));
    // ...which may still be read.
    esp32("leta ubao\n#[wazi]\nkazi f() -> Ukweli {\n    rejesha ubao_soma(35)\n}\n").unwrap();
    // Cortex-M has no board yet.
    let program = bytecode(&pin("2"));
    let e = device::build(&program, &indices(&program, &["f"]), "t")
        .map(|_| ())
        .unwrap_err();
    assert!(e.contains("ESP32 pekee"), "{e}");
}

#[test]
fn a_wazi_kuu_is_the_esp32_entry_point() {
    let program = bytecode(
        "leta ubao\n#[wazi]\nkazi kuu(hoja: Orodha<Neno>) -> Tupu {\n    wakati kweli {\n        \
         ubao_andika(2, kweli)\n        ubao_subiri(500)\n        ubao_andika(2, si_kweli)\n        \
         ubao_subiri(500)\n    }\n}\n",
    );
    let kuu = indices(&program, &["kuu"])[0];
    let built = device::build_firmware(device::Lengo::Esp32, &program, &[], Some(kuu), "t")
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(built.object.windows(8).any(|w| w == b"app_main"));
    assert_eq!(built.exports, ["kuu (app_main)"]);
    assert_eq!(built.stack.len(), 1);
    assert!(built.header.contains("app_main"));
    // Only on the ESP32.
    let e = device::build_firmware(device::Lengo::CortexM, &program, &[], Some(kuu), "t")
        .map(|_| ())
        .unwrap_err();
    assert!(e.contains("ESP32 pekee"), "{e}");
}
