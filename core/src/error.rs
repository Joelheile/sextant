use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("wrong key or not a Sextant database")]
    BadKey,
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); run `sextant privacy delete-everything` to start fresh"
    )]
    SchemaTooNew { found: i32, supported: i32 },
    #[error(
        "database schema version {found} is older than {expected}; open it with the writer first"
    )]
    SchemaTooOld { found: i32, expected: i32 },
    #[error("the database has no valid email secret; open it with the app first")]
    NoSecret,
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Platform(String),
    #[error("{0}")]
    Invalid(String),
}
