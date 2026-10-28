use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("wrong key or not a Sextant database")]
    BadKey,
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: i32, supported: i32 },
    #[error(
        "database schema version {found} is older than {expected}; open it with the writer first"
    )]
    SchemaTooOld { found: i32, expected: i32 },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no workflow with id {0}")]
    NoWorkflow(i64),
    #[error("no correction rule matches this phrase")]
    UnknownCorrection,
    #[error("correction needs {0}")]
    BadTarget(&'static str),
}
