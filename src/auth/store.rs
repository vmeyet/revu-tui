use anyhow::{Context, Result, bail, ensure};
use std::io::Write;
use std::process::{Command, Stdio};

pub trait SecretStore {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

/// macOS keychain through `/usr/bin/security`.
/// Reads and deletes name the service and account in argv, which hold nothing secret; the secret
/// comes back on a pipe. Writes go through `security -i`, so the secret never shows in argv.
/// Items are owned by Apple's signed binary, so a rebuilt CLI never triggers an ACL prompt.
pub struct SecurityCli {
    service: String,
}

const NOT_FOUND: i32 = 44;

impl SecurityCli {
    pub fn new(service: &str) -> Self {
        Self { service: service.to_owned() }
    }

    fn command(&self, verb: &str, account: &str) -> Command {
        let mut cmd = Command::new("/usr/bin/security");
        cmd.arg(verb).arg("-a").arg(account).arg("-s").arg(&self.service);
        cmd
    }
}

impl SecretStore for SecurityCli {
    fn get(&self, account: &str) -> Result<Option<String>> {
        let output = self.command("find-generic-password", account).arg("-w").output().context("running security")?;
        if output.status.code() == Some(NOT_FOUND) {
            return Ok(None);
        }
        if !output.status.success() {
            bail!("keychain read failed: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(Some(String::from_utf8(output.stdout)?.trim_end().to_owned()))
    }

    /// Written through `security -i` with hex data: the secret never appears in argv,
    /// and unlike a bare `-w` it is never read from the terminal.
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let line = format!("add-generic-password -a {} -s {} -U -X {}\n", plain(account)?, self.service, hex(secret));
        let mut child = Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("running security")?;
        child.stdin.take().context("security took no input")?.write_all(line.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("keychain write failed: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        let output = self.command("delete-generic-password", account).output().context("running security")?;
        if output.status.success() || output.status.code() == Some(NOT_FOUND) {
            return Ok(());
        }
        bail!("keychain delete failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
}

/// `security -i` splits its line on spaces and quotes, so an account holds only what a host or a
/// provider name does: nothing in it can end the account and start another option.
fn plain(account: &str) -> Result<&str> {
    let is_plain = !account.is_empty() && account.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    ensure!(is_plain, "not a keychain account name: {account:?}");
    Ok(account)
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    items: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.items.lock().unwrap().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.items.lock().unwrap().insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.items.lock().unwrap().remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let s = MemoryStore::default();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        s.set("a", "2").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("2"));
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    #[test]
    fn a_keychain_account_with_a_space_or_a_quote_is_refused_before_security_runs() {
        let store = SecurityCli::new("revu-test");
        for account in ["gitlab.com -s other", "gitlab.com\"", "gitlab.com'", "gitlab.com\nlist-keychains", ""] {
            let err = store.set(account, "glpat-xxxx").unwrap_err().to_string();
            assert!(err.starts_with("not a keychain account name"), "{account:?}: {err}");
        }
        for account in ["gitlab.com", "gitlab.acme.dev:8443", "anthropic", "slack-cli_test"] {
            assert_eq!(plain(account).unwrap(), account);
        }
    }

    #[test]
    fn hex_encodes_bytes() {
        assert_eq!(hex("A{\"}"), "417b227d");
    }

    #[test]
    #[ignore = "touches the real login keychain"]
    fn security_cli_round_trip() {
        let s = SecurityCli::new("slack-cli-test");
        s.delete("probe").unwrap();
        assert_eq!(s.get("probe").unwrap(), None);
        s.set("probe", r#"{"token":"one"}"#).unwrap();
        assert_eq!(s.get("probe").unwrap().as_deref(), Some(r#"{"token":"one"}"#));
        s.set("probe", "two").unwrap();
        assert_eq!(s.get("probe").unwrap().as_deref(), Some("two"));
        s.delete("probe").unwrap();
        s.delete("probe").unwrap();
        assert_eq!(s.get("probe").unwrap(), None);
    }
}
