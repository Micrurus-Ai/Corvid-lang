-- Move shelves and reviews onto Corvid tables (src/library_store.cor).
-- The CREATE TABLE statements are what `corvid migrate plan` writes for
-- the declarations; the copies keep every row and revision.

CREATE TABLE "StoredShelf" (
    "shelf" INTEGER PRIMARY KEY NOT NULL,
    "title" TEXT NOT NULL,
    "revision" INTEGER NOT NULL
);

CREATE TABLE "ProposalItem" (
    "proposal_key" TEXT NOT NULL,
    "shelf" INTEGER NOT NULL,
    "base_revision" INTEGER NOT NULL,
    "owner" TEXT,
    "applied_title" TEXT,
    "outcome" TEXT,
    "revision" INTEGER NOT NULL,
    PRIMARY KEY ("proposal_key", "shelf")
);

INSERT INTO "StoredShelf" ("shelf", "title", "revision")
    SELECT shelf, title, revision FROM shelves;

-- A proposal item's own revision starts at 1, as a newly inserted row's does.
INSERT INTO "ProposalItem" ("proposal_key", "shelf", "base_revision", "owner", "applied_title", "outcome", "revision")
    SELECT proposal_key, shelf, base_revision, owner, applied_title, outcome, 1 FROM proposal_items;

DROP TABLE proposal_items;
DROP TABLE shelves;
