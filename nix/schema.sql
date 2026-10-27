CREATE TABLE IF NOT EXISTS events
(
    id UInt64,
    name String,
    amount Float64,
    active Bool,
    payload JSON
)
ENGINE = MergeTree
ORDER BY id;
