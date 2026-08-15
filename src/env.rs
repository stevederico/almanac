use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub host: String,
    pub cal_name: String,
    pub db_path: String,
    pub feed_token: String,
    pub agent_key: String,
    pub tls_key: String,
    pub tls_cert: String,
}

const DEFAULT_PORT: u16 = 18788;

/// Load KEY=value pairs into the process env. Existing env wins. Symlinks are refused.
pub fn load_env_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "{} must be a regular file, not a symlink",
            path.display()
        ));
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        if eq == 0 {
            continue;
        }
        let key = line[..eq].trim();
        if !is_env_key(key) {
            continue;
        }
        if std::env::var_os(key).is_some() {
            continue;
        }
        let mut value = line[eq + 1..].trim().to_string();
        if is_quoted(&value) {
            value = value[1..value.len() - 1].to_string();
        }
        // yagni: set_var is process-wide; fine for a single-process server
        unsafe { std::env::set_var(key, value) };
    }
    Ok(())
}

fn is_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_quoted(value: &str) -> bool {
    (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
        || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
}

/// Read runtime config. Empty secrets are allowed here; callers decide whether to seed home.
pub fn read_config(env: &HashMap<String, String>) -> Config {
    let port_raw = env.get("PORT").map(String::as_str).unwrap_or("");
    let port = port_raw
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .unwrap_or(DEFAULT_PORT);
    let host = env
        .get("HOST")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    let cal_name = env
        .get("CAL_NAME")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Almanac".to_string());
    Config {
        port,
        host,
        cal_name,
        db_path: env
            .get("DB_PATH")
            .cloned()
            .unwrap_or_else(|| "./data/calendar.db".to_string()),
        feed_token: env.get("FEED_TOKEN").cloned().unwrap_or_default(),
        agent_key: env.get("AGENT_KEY").cloned().unwrap_or_default(),
        tls_key: env.get("TLS_KEY").cloned().unwrap_or_default(),
        tls_cert: env.get("TLS_CERT").cloned().unwrap_or_default(),
    }
}

pub fn read_config_from_os() -> Config {
    let keys = [
        "PORT",
        "HOST",
        "CAL_NAME",
        "DB_PATH",
        "FEED_TOKEN",
        "AGENT_KEY",
        "TLS_KEY",
        "TLS_CERT",
    ];
    let mut map = HashMap::new();
    for key in keys {
        if let Ok(v) = std::env::var(key) {
            map.insert(key.to_string(), v);
        }
    }
    read_config(&map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_port_and_name() {
        let cfg = read_config(&HashMap::new());
        assert_eq!(cfg.port, 18788);
        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.cal_name, "Almanac");
        assert_eq!(cfg.tls_key, "");
        assert_eq!(cfg.tls_cert, "");
    }

    #[test]
    fn reads_env_values() {
        let mut env = HashMap::new();
        env.insert("PORT".into(), "9000".into());
        env.insert("CAL_NAME".into(), "Agents".into());
        env.insert("DB_PATH".into(), "/tmp/c.db".into());
        env.insert("FEED_TOKEN".into(), "feed".into());
        env.insert("AGENT_KEY".into(), "key".into());
        let cfg = read_config(&env);
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.cal_name, "Agents");
        assert_eq!(cfg.db_path, "/tmp/c.db");
        assert_eq!(cfg.feed_token, "feed");
        assert_eq!(cfg.agent_key, "key");

        let mut prod = HashMap::new();
        prod.insert("HOST".into(), "::".into());
        prod.insert("PORT".into(), "8000".into());
        let prod = read_config(&prod);
        assert_eq!(prod.host, "::");
        assert_eq!(prod.port, 8000);
    }

    #[test]
    fn load_env_file_cases() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = std::env::temp_dir().join(format!(
            "almanac-env-{}",
            crate::util::hex_encode(&crate::util::random_bytes(8))
        ));
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join(".env");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "FEED_TOKEN=fromfile\nPORT=8123").unwrap();
        drop(f);

        unsafe {
            std::env::remove_var("FEED_TOKEN");
            std::env::set_var("PORT", "8000");
        }
        load_env_file(&path).unwrap();
        assert_eq!(std::env::var("FEED_TOKEN").unwrap(), "fromfile");
        assert_eq!(std::env::var("PORT").unwrap(), "8000");
        unsafe {
            std::env::remove_var("FEED_TOKEN");
            std::env::remove_var("PORT");
        }

        let real = dir.join("real.env");
        fs::write(&real, "FEED_TOKEN=x\n").unwrap();
        let link = dir.join("link.env");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let err = load_env_file(&link).unwrap_err();
            assert!(err.contains("regular file"));
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
