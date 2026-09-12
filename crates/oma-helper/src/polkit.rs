//! Minimal polkit authorization check via `org.freedesktop.PolicyKit1.Authority`.

use std::collections::HashMap;
use zbus::zvariant::Value;

#[zbus::proxy(interface = "org.freedesktop.PolicyKit1.Authority", default_service = "org.freedesktop.PolicyKit1", default_path = "/org/freedesktop/PolicyKit1/Authority")]
trait Authority {
    #[allow(clippy::type_complexity)]
    fn check_authorization(&self, subject: &(&str, HashMap<&str, Value<'_>>), action_id: &str, details: HashMap<&str, &str>, flags: u32, cancellation_id: &str) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

/// Ask polkit whether the D-Bus `sender` may perform `action_id`.
/// `interactive` allows an authentication agent to prompt.
pub async fn check(conn: &zbus::Connection, sender: &str, action_id: &str, interactive: bool) -> anyhow::Result<bool> {
    let auth = AuthorityProxy::new(conn).await?;
    let mut subj = HashMap::new();
    subj.insert("name", Value::from(sender));
    let subject = ("system-bus-name", subj);
    let flags = if interactive { 1 } else { 0 };
    let (authorized, _challenge, _details) = auth.check_authorization(&subject, action_id, HashMap::new(), flags, "").await?;
    Ok(authorized)
}
