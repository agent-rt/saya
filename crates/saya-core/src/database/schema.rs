pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS clipboard_entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    content     TEXT    NOT NULL,
    byte_size   INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_entries_created
    ON clipboard_entries(created_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS clipboard_vectors USING vec0(
    embedding float[384]
);

CREATE TABLE IF NOT EXISTS app_launches (
    path        TEXT    NOT NULL PRIMARY KEY,
    count       INTEGER NOT NULL DEFAULT 0,
    last_used   INTEGER NOT NULL
);
"#;
