CREATE TABLE IF NOT EXISTS moz_places (
    id INTEGER PRIMARY KEY,
    url LONGVARCHAR NOT NULL,
    title LONGVARCHAR,

    visit_count_local INTEGER NOT NULL DEFAULT 0,
    visit_count_remote INTEGER NOT NULL DEFAULT 0,
    hidden INTEGER DEFAULT 0 NOT NULL,
    typed INTEGER DEFAULT 0 NOT NULL,
    frecency INTEGER DEFAULT -1 NOT NULL,

    last_visit_date_local INTEGER NOT NULL DEFAULT 0,
    last_visit_date_remote INTEGER NOT NULL DEFAULT 0,
    guid TEXT NOT NULL UNIQUE,
    foreign_count INTEGER DEFAULT 0 NOT NULL,
    url_hash INTEGER DEFAULT 0 NOT NULL,
    description TEXT,
    preview_image_url TEXT,

    origin_id INTEGER,

    sync_status TINYINT NOT NULL DEFAULT 1,
    sync_change_counter INTEGER NOT NULL DEFAULT 0,
    unknown_fields TEXT,

    FOREIGN KEY(origin_id) REFERENCES moz_origins(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS url_hashindex ON moz_places(url_hash);
CREATE INDEX IF NOT EXISTS visitcountlocal ON moz_places(visit_count_local);
CREATE INDEX IF NOT EXISTS visitcountremote ON moz_places(visit_count_remote);
CREATE INDEX IF NOT EXISTS frecencyindex ON moz_places(frecency);
CREATE INDEX IF NOT EXISTS lastvisitdatelocalindex ON moz_places(last_visit_date_local);
CREATE INDEX IF NOT EXISTS lastvisitdateremoteindex ON moz_places(last_visit_date_remote);
CREATE UNIQUE INDEX IF NOT EXISTS guid_uniqueindex ON moz_places(guid);
CREATE INDEX IF NOT EXISTS originidindex ON moz_places(origin_id);

CREATE TABLE IF NOT EXISTS moz_places_tombstones (
    guid TEXT PRIMARY KEY
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS moz_places_stale_frecencies (
    place_id INTEGER PRIMARY KEY NOT NULL REFERENCES moz_places(id)
                                          ON DELETE CASCADE,
    stale_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS moz_historyvisits (
    id INTEGER PRIMARY KEY,
    is_local INTEGER NOT NULL,
    from_visit INTEGER,
    place_id INTEGER NOT NULL,
    visit_date INTEGER NOT NULL,
    visit_type INTEGER NOT NULL,

    unknown_fields TEXT,

    FOREIGN KEY(place_id) REFERENCES moz_places(id) ON DELETE CASCADE,
    FOREIGN KEY(from_visit) REFERENCES moz_historyvisits(id)
);

CREATE INDEX IF NOT EXISTS placedateindex ON moz_historyvisits(place_id, visit_date);
CREATE INDEX IF NOT EXISTS fromindex ON moz_historyvisits(from_visit);
CREATE INDEX IF NOT EXISTS dateindex ON moz_historyvisits(visit_date);
CREATE INDEX IF NOT EXISTS islocalindex ON moz_historyvisits(is_local);

CREATE INDEX IF NOT EXISTS visits_from_type_idx ON moz_historyvisits(from_visit, visit_type);

CREATE TABLE IF NOT EXISTS moz_historyvisit_tombstones (
    place_id INTEGER NOT NULL,
    visit_date INTEGER NOT NULL,
    FOREIGN KEY(place_id) REFERENCES moz_places(id) ON DELETE CASCADE,
    PRIMARY KEY(place_id, visit_date)
);

CREATE TABLE IF NOT EXISTS moz_inputhistory (
    place_id INTEGER NOT NULL,
    input LONGVARCHAR NOT NULL,
    use_count INTEGER,

    PRIMARY KEY (place_id, input),
    FOREIGN KEY(place_id) REFERENCES moz_places(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS moz_bookmarks (
    id INTEGER PRIMARY KEY,
    fk INTEGER DEFAULT NULL,
    type INTEGER NOT NULL,
    parent INTEGER,
    position INTEGER NOT NULL,
    title TEXT,
    dateAdded INTEGER NOT NULL DEFAULT 0,
    lastModified INTEGER NOT NULL DEFAULT 0,
    guid TEXT NOT NULL UNIQUE CHECK(length(guid) == 12),

    syncStatus INTEGER NOT NULL DEFAULT 0,
    syncChangeCounter INTEGER NOT NULL DEFAULT 1,

    CHECK((type == 1 AND fk IS NOT NULL) OR (type > 1 AND fk IS NULL))

    CHECK(guid == "root________" OR parent IS NOT NULL)

    FOREIGN KEY(fk) REFERENCES moz_places(id) ON DELETE RESTRICT
    FOREIGN KEY(parent) REFERENCES moz_bookmarks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS itemlastmodifiedindex ON moz_bookmarks(fk, lastModified);

CREATE UNIQUE INDEX IF NOT EXISTS guid_uniqueindex ON moz_bookmarks(guid);

CREATE TABLE IF NOT EXISTS moz_bookmarks_deleted (
    guid TEXT PRIMARY KEY,
    dateRemoved INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS moz_origins (
    id INTEGER PRIMARY KEY,
    prefix TEXT NOT NULL,
    host TEXT NOT NULL,
    rev_host TEXT NOT NULL,
    frecency INTEGER NOT NULL,
    UNIQUE (prefix, host)
);

CREATE INDEX IF NOT EXISTS hostindex ON moz_origins(rev_host);

CREATE TABLE IF NOT EXISTS moz_meta (
    key TEXT PRIMARY KEY,
    value NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS moz_tags(
    id INTEGER PRIMARY KEY,
    tag TEXT UNIQUE NOT NULL,
    lastModified INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS moz_tags_relation(
    tag_id INTEGER NOT NULL REFERENCES moz_tags(id) ON DELETE CASCADE,
    place_id INTEGER NOT NULL REFERENCES moz_places(id) ON DELETE CASCADE,
    PRIMARY KEY(tag_id, place_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS moz_bookmarks_synced(
    id INTEGER PRIMARY KEY,

    guid TEXT UNIQUE NOT NULL,

    parentGuid TEXT,

    serverModified INTEGER NOT NULL DEFAULT 0,
    needsMerge BOOLEAN NOT NULL DEFAULT 0,
    validity INTEGER NOT NULL DEFAULT 1,
    isDeleted BOOLEAN NOT NULL DEFAULT 0,
    kind INTEGER NOT NULL DEFAULT -1,

    dateAdded INTEGER NOT NULL DEFAULT 0,
    title TEXT,
    placeId INTEGER REFERENCES moz_places(id)
                    ON DELETE SET NULL,
    keyword TEXT,
    description TEXT,
    loadInSidebar BOOLEAN,
    smartBookmarkName TEXT,
    feedURL TEXT,
    siteURL TEXT,

    unknownFields TEXT
);

CREATE INDEX IF NOT EXISTS moz_bookmarks_synced_urls ON moz_bookmarks_synced(placeId);
CREATE INDEX IF NOT EXISTS moz_bookmarks_synced_keywords ON moz_bookmarks_synced(keyword)
                                                            WHERE keyword NOT NULL;

CREATE TABLE IF NOT EXISTS moz_bookmarks_synced_structure(
    guid TEXT,
    parentGuid TEXT REFERENCES moz_bookmarks_synced(guid)
                    ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY(parentGuid, guid)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS moz_bookmarks_synced_tag_relation(
    itemId INTEGER NOT NULL REFERENCES moz_bookmarks_synced(id)
                            ON DELETE CASCADE,
    tagId INTEGER NOT NULL REFERENCES moz_tags(id)
                           ON DELETE CASCADE,
    PRIMARY KEY(itemId, tagId)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS moz_keywords(
    place_id INTEGER PRIMARY KEY REFERENCES moz_places(id)
                     ON DELETE RESTRICT,
    keyword TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS moz_places_metadata (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,

    place_id INTEGER NOT NULL,

    total_view_time INTEGER NOT NULL DEFAULT 0,
    search_query_id INTEGER,
    referrer_place_id INTEGER,
    document_type INTEGER NOT NULL DEFAULT 0,
    typing_time INTEGER NOT NULL DEFAULT 0,
    key_presses INTEGER NOT NULL DEFAULT 0,

    FOREIGN KEY(place_id) REFERENCES moz_places(id) ON DELETE CASCADE,
    FOREIGN KEY(search_query_id) REFERENCES moz_places_metadata_search_queries(id) ON DELETE CASCADE,
    FOREIGN KEY(referrer_place_id) REFERENCES moz_places(id) ON DELETE CASCADE

    CHECK(place_id != referrer_place_id)
);

CREATE TABLE IF NOT EXISTS moz_places_metadata_search_queries (
    id INTEGER PRIMARY KEY,
    term TEXT NOT NULL UNIQUE
);

INSERT INTO moz_places(id, guid, url, frecency)
VALUES(1, 'page_guid___', 'https://example.com', -1);

INSERT INTO moz_bookmarks(id, type, parent, position, dateAdded, lastModified, guid)
VALUES(1, 2, NULL, 0, 1, 1, 'root________');

INSERT INTO moz_bookmarks(id, type, parent, position, dateAdded, lastModified, guid)
VALUES(2, 2, 1, 0, 1, 1, 'folder_guid_');

INSERT INTO moz_bookmarks(id, type, parent, position, dateAdded, lastModified, fk, guid)
VALUES(3, 1, 2, 0, 1, 1, 1, 'bmk_guid____');

INSERT INTO moz_bookmarks(id, type, parent, position, dateAdded, lastModified, guid)
VALUES(4, 3, 2, 1, 1, 1, 'sep_guid____');

PRAGMA user_version = 17;
