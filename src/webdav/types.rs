use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DavLock {
    pub token: String,
    pub exclusive: bool,
    pub expires_at: chrono::NaiveDateTime,
}

impl DavLock {
    pub fn new_exclusive(token: String, timeout_secs: i64) -> Self {
        Self {
            token,
            exclusive: true,
            expires_at: chrono::Local::now().naive_local()
                + chrono::Duration::seconds(timeout_secs),
        }
    }
    pub fn is_expired(&self) -> bool {
        chrono::Local::now().naive_local() > self.expires_at
    }
}
