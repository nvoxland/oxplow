-- Each section a page shows by a switch param (`switch:` in its view),
-- beside the one it opens on: what the launcher and search list as
-- "<page>: <section>", opening the page on it. Restated with each
-- extension generation, as `page_param` is.
CREATE TABLE page_section (
    page_uri TEXT NOT NULL,
    position INTEGER NOT NULL,
    section TEXT NOT NULL,
    title TEXT NOT NULL,
    uri TEXT NOT NULL,
    PRIMARY KEY (page_uri, position)
) STRICT;
