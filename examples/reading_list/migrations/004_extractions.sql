CREATE TABLE extraction_jobs (
    id INTEGER PRIMARY KEY,
    owner TEXT NOT NULL,
    instruction TEXT NOT NULL,
    budget INTEGER NOT NULL CHECK (budget BETWEEN 1 AND 12),
    estimated_microusd INTEGER NOT NULL CHECK (estimated_microusd BETWEEN 0 AND 1000000000),
    spent INTEGER NOT NULL DEFAULT 0 CHECK (spent BETWEEN 0 AND budget),
    paused INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX extraction_owner ON extraction_jobs(owner, id);

CREATE TABLE extraction_items (
    job INTEGER NOT NULL REFERENCES extraction_jobs(id),
    shelf INTEGER NOT NULL,
    source_title TEXT NOT NULL,
    source_revision INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0,
    lease_until INTEGER NOT NULL DEFAULT 0,
    used_instruction TEXT,
    extracted TEXT,
    correction TEXT,
    PRIMARY KEY (job, shelf)
);

CREATE VIEW extraction_progress AS
SELECT i.*, j.instruction, j.paused,
    CASE WHEN extracted IS NOT NULL THEN 'Completed'
         WHEN lease_until > unixepoch() THEN 'In flight'
         WHEN spent >= budget THEN 'Skipped'
         ELSE 'Pending' END AS state
FROM extraction_items i JOIN extraction_jobs j ON j.id = i.job;
