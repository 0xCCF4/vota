-- Theme and Theme Tweaks (ticket 10). The Theme is a name resolved against
-- the Theme registry at render time (a Theme removed from the theme folder
-- falls back to the default); existing Ballots get the default Theme. The
-- accent colour is stored lowercase as `#rrggbb`; the logo is a name resolved
-- against the logo folder at render time.
ALTER TABLE ballots ADD COLUMN theme TEXT NOT NULL DEFAULT 'classic'
    CHECK (length(theme) BETWEEN 1 AND 64);
ALTER TABLE ballots ADD COLUMN accent_color TEXT
    CHECK (accent_color GLOB '#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]');
ALTER TABLE ballots ADD COLUMN logo TEXT
    CHECK (length(logo) BETWEEN 1 AND 64);
