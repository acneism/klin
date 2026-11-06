CREATE TEMP TABLE moz_updateoriginsinsert_temp (
    place_id INTEGER PRIMARY KEY,
    prefix TEXT NOT NULL,
    host TEXT NOT NULL,
    rev_host TEXT NOT NULL,
    frecency INTEGER NOT NULL
);

CREATE TEMP TABLE moz_updateoriginsdelete_temp (
    prefix TEXT NOT NULL,
    host TEXT NOT NULL,
    frecency_delta INTEGER NOT NULL,
    PRIMARY KEY (prefix, host)
) WITHOUT ROWID;

CREATE TEMP TABLE moz_updateoriginsupdate_temp (
    prefix TEXT NOT NULL,
    host TEXT NOT NULL,
    frecency_delta INTEGER NOT NULL,
    PRIMARY KEY (prefix, host)
) WITHOUT ROWID;
