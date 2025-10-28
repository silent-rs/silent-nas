use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DavLock {
    pub token: String,
    pub exclusive: bool,
    pub owner: Option<String>,
    pub depth_infinity: bool,
    pub expires_at: chrono::NaiveDateTime,
}

impl DavLock {
    pub fn new(
        token: String,
        exclusive: bool,
        timeout_secs: i64,
        owner: Option<String>,
        depth_infinity: bool,
    ) -> Self {
        Self {
            token,
            exclusive,
            owner,
            depth_infinity,
            expires_at: chrono::Local::now().naive_local()
                + chrono::Duration::seconds(timeout_secs),
        }
    }
    pub fn is_expired(&self) -> bool {
        chrono::Local::now().naive_local() > self.expires_at
    }
}
