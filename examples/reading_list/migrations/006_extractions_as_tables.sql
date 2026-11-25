-- Move shelf extractions onto Corvid tables (src/extraction_store.cor).
-- The CREATE TABLE statements are what `corvid migrate plan` writes for
-- the declarations; the copies keep every job, reservation and result.

CREATE TABLE "ExtractionJob" (
    "job" INTEGER PRIMARY KEY NOT NULL,
    "owner" TEXT NOT NULL,
    "instruction" TEXT NOT NULL,
    "budget" INTEGER NOT NULL,
    "estimated_microusd" INTEGER NOT NULL,
    "paused" BOOLEAN NOT NULL,
    "revision" INTEGER NOT NULL
);

CREATE TABLE "ExtractionSpending" (
    "job" INTEGER PRIMARY KEY NOT NULL,
    "spent" INTEGER NOT NULL,
    "revision" INTEGER NOT NULL
);

CREATE TABLE "ExtractionItem" (
    "job" INTEGER NOT NULL,
    "shelf" INTEGER NOT NULL,
    "source_title" TEXT NOT NULL,
    "source_revision" INTEGER NOT NULL,
    "attempt" INTEGER NOT NULL,
    "lease_until_ms" INTEGER NOT NULL,
    "used_instruction" TEXT,
    "extracted" TEXT,
    "correction" TEXT,
    "revision" INTEGER NOT NULL,
    PRIMARY KEY ("job", "shelf")
);

-- A job's version counted its settings changes from 0, and a stored row's
-- revision counts from 1. A progress page opened before the move carries
-- the old number, so its form asks the person to refresh and retry.
INSERT INTO "ExtractionJob" ("job", "owner", "instruction", "budget", "estimated_microusd", "paused", "revision")
    SELECT id, owner, instruction, budget, estimated_microusd, paused <> 0, version + 1 FROM extraction_jobs;

INSERT INTO "ExtractionSpending" ("job", "spent", "revision")
    SELECT id, spent, 1 FROM extraction_jobs;

-- Leases were held in seconds since the epoch and are now milliseconds.
INSERT INTO "ExtractionItem" ("job", "shelf", "source_title", "source_revision", "attempt", "lease_until_ms", "used_instruction", "extracted", "correction", "revision")
    SELECT job, shelf, source_title, source_revision, attempt, lease_until * 1000, used_instruction, extracted, correction, 1 FROM extraction_items;

DROP VIEW extraction_progress;
DROP TABLE extraction_items;
DROP TABLE extraction_jobs;
