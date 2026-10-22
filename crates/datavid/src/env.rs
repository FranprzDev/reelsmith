//! Environment expansion for config values: `${VAR}` and `${VAR:-default}`.

/// Expand `${VAR}` and `${VAR:-default}`; an unset variable without a default expands to an empty string.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = &rest[i + 2..i + j];
        let (var, def) = inner.split_once(":-").unwrap_or((inner, ""));
        out.push_str(&std::env::var(var).unwrap_or_else(|_| def.to_string()));
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}
