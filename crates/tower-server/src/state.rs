//! Shared server state: pool, event log, config, driver (D§3).

use std::sync::Arc;

use sqlx::SqlitePool;
use tower_driver::Harness;

use crate::config::Config;
use crate::storage::EventLog;

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub pool: SqlitePool,
    pub events: EventLog,
    pub _config: Config,
    pub _token: String,
    pub driver: Arc<dyn Harness>,
    pub started_at: i64,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(
        pool: SqlitePool,
        events: EventLog,
        config: Config,
        token: String,
        driver: Arc<dyn Harness>,
    ) -> Self {
        Self(Arc::new(Inner {
            pool,
            events,
            _config: config,
            _token: token,
            driver,
            started_at: tower_core::now_ms(),
        }))
    }
}
