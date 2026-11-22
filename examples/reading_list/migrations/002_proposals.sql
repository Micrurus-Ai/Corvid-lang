ALTER TABLE shelves ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;

CREATE TABLE proposal_items (
    proposal_key TEXT NOT NULL,
    shelf INTEGER NOT NULL REFERENCES shelves(shelf),
    base_revision INTEGER NOT NULL,
    applied_title TEXT,
    outcome TEXT CHECK (outcome IN ('Saved', 'Conflict')),
    PRIMARY KEY (proposal_key, shelf)
);
