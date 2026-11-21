CREATE TABLE shelves (
    shelf INTEGER PRIMARY KEY CHECK (shelf BETWEEN 1 AND 3),
    title TEXT NOT NULL
);
INSERT INTO shelves (shelf, title) VALUES
    (1, 'The Odyssey'), (2, 'Persuasion'), (3, 'Frankenstein');
