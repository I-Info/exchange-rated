-- Create cib_rate_records table
CREATE TABLE IF NOT EXISTS cib_rate_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rate TEXT NOT NULL,
    timestamp DATETIME NOT NULL
);

-- Create unique index on timestamp to prevent duplicate records
CREATE UNIQUE INDEX IF NOT EXISTS idx_cib_rate_records_timestamp ON cib_rate_records(timestamp);
