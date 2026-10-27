//! The text a value shows as: `x kama Neno`, and the one place that defines how a value reads
//! to a program's user (spec: `Onyesheka`). Scalars read as they always have; containers and
//! `Chaguo`/`Tokeo` read in Swahili, not as Rust debug output.

use super::{numeric::format_namba, Value};

/// The text of `v` as `kama Neno` shows it.
pub(crate) fn display_value(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, false);
    out
}

/// Write `v` into `out`. Text inside a container is quoted, so `[1, "a"]` and `[1, 2]` differ.
fn write_value(out: &mut String, v: &Value, nested: bool) {
    match v {
        Value::Namba(n) => out.push_str(&format_namba(*n)),
        Value::Biti(x, _) => out.push_str(&x.to_string()),
        Value::NambaKuu(b) => out.push_str(&b.to_string()),
        Value::NambaSahihi(b) => out.push_str(&b.to_string()),
        Value::Neno(s) => {
            if nested {
                out.push_str(&format!("{:?}", &**s));
            } else {
                out.push_str(s);
            }
        }
        Value::Ukweli(true) => out.push_str("kweli"),
        Value::Ukweli(false) => out.push_str("si_kweli"),
        Value::Tupu => out.push_str("Tupu"),
        Value::Hamna | Value::Chaguo(None) => out.push_str("Hamna"),
        Value::Herufi(c) => {
            if nested {
                out.push_str(&format!("{c:?}"));
            } else {
                out.push(*c);
            }
        }
        Value::Chaguo(Some(inner)) => wrap(out, "Kuna", inner),
        Value::Tokeo(Ok(inner)) => wrap(out, "Sawa", inner),
        Value::Tokeo(Err(inner)) => wrap(out, "Kosa", inner),
        Value::Enum(en, vn, data) if en == "Chaguo" => match (vn.as_str(), data) {
            ("Kuna", Some(inner)) => wrap(out, "Kuna", inner),
            _ => out.push_str("Hamna"),
        },
        Value::Enum(en, vn, data) if en == "Tokeo" => match data {
            Some(inner) => wrap(out, vn.as_str(), inner),
            None => out.push_str(vn.as_str()),
        },
        Value::Orodha(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, true);
            }
            out.push(']');
        }
        Value::Jozi(a, b) => {
            out.push('(');
            write_value(out, a, true);
            out.push_str(", ");
            write_value(out, b, true);
            out.push(')');
        }
        Value::Struct(name, fields) => {
            out.push_str(name.as_str());
            out.push_str(" { ");
            for (i, (field, value)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(field.as_str());
                out.push_str(": ");
                write_value(out, value, true);
            }
            out.push_str(" }");
        }
        // Other values (maps, handles, tasks) keep the Rust debug text until each has its own
        // reading; see the audit's item 9 notes.
        other => out.push_str(&format!("{other:?}")),
    }
}

fn wrap(out: &mut String, label: &str, inner: &Value) {
    out.push_str(label);
    out.push('(');
    write_value(out, inner, true);
    out.push(')');
}
