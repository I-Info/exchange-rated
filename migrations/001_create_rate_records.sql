-- Create rate_records table
CREATE TABLE IF NOT EXISTS rate_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rate TEXT NOT NULL,
    timestamp DATETIME NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Create unique index on timestamp to prevent duplicate records
CREATE UNIQUE INDEX IF NOT EXISTS idx_rate_records_timestamp ON rate_records(timestamp);

-- Create index on created_at for performance
CREATE INDEX IF NOT EXISTS idx_rate_records_created_at ON rate_records(created_at);
