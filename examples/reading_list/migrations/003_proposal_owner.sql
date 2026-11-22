-- Existing anonymous proposals have no authenticated owner. Start a new review to apply.
ALTER TABLE proposal_items ADD COLUMN owner TEXT;
