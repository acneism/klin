CREATE TEMP TRIGGER moz_places_afterinsert_trigger_tombstone
AFTER INSERT ON moz_places
FOR EACH ROW
BEGIN
    DELETE FROM moz_places_tombstones WHERE guid = NEW.guid;
END;

CREATE TEMP TRIGGER moz_bookmarks_beforeinsert_trigger
BEFORE INSERT ON moz_bookmarks
BEGIN

    SELECT throw(format('insert: len(guid)=%d', length(NEW.guid)))
    WHERE length(NEW.guid) <> 12;

    SELECT throw('insert: type=1; fk NULL')
    WHERE NEW.type = 1
      AND NEW.fk IS NULL;

    SELECT throw(format('insert: type=%d; fk NOT NULL', NEW.type))
    WHERE NEW.type <> 1
      AND NEW.fk IS NOT NULL;

    SELECT throw('insert: root with parent')
    WHERE NEW.guid = 'root________'
      AND NEW.parent IS NOT NULL;

    SELECT throw('insert: item without parent')
    WHERE NEW.guid <> 'root________'

      AND NOT EXISTS(
          SELECT 1 FROM moz_bookmarks WHERE id = NEW.parent);
END;

CREATE TEMP TRIGGER moz_bookmarks_beforeupdate_trigger
BEFORE UPDATE ON moz_bookmarks
BEGIN
    SELECT throw(format('update: len(guid)=%d', length(NEW.guid)))
    WHERE length(NEW.guid) <> 12;

    SELECT throw('update: type=1; fk NULL')
    WHERE NEW.type = 1
      AND NEW.fk IS NULL;

    SELECT throw(format('update: type=%d; fk NOT NULL', NEW.type))
    WHERE NEW.type <> 1
      AND NEW.fk IS NOT NULL;

    SELECT throw('update: root with parent')
    WHERE NEW.guid = 'root________'
      AND NEW.parent IS NOT NULL;

    SELECT throw(format(
        'update: item without parent: operation=%q',
        CASE WHEN NEW.syncChangeCounter > 0 THEN 'sync' ELSE 'user' END
    ))
    WHERE NEW.guid <> 'root________'
      AND NEW.parent IS NULL;

    SELECT throw(format('update: old type=%d; new=%d', OLD.type, NEW.type))
    WHERE OLD.type <> NEW.type;
END;

CREATE TEMP TRIGGER moz_historyvisits_afterinsert_trigger
AFTER INSERT ON moz_historyvisits FOR EACH ROW
BEGIN
    UPDATE moz_places SET
        visit_count_remote = visit_count_remote + (NEW.visit_type NOT IN (0, 4, 7, 8, 9) AND NOT(NEW.is_local)),
        visit_count_local =  visit_count_local + (NEW.visit_type NOT IN (0, 4, 7, 8, 9) AND NEW.is_local),
        last_visit_date_local = MAX(last_visit_date_local,
                                    CASE WHEN NEW.is_local THEN NEW.visit_date ELSE 0 END),
        last_visit_date_remote = MAX(last_visit_date_remote,
                                     CASE WHEN NEW.is_local THEN 0 ELSE NEW.visit_date END)
    WHERE id = NEW.place_id;
END;

CREATE TEMP TRIGGER moz_historyvisits_afterdelete_trigger
AFTER DELETE ON moz_historyvisits FOR EACH ROW
BEGIN
    UPDATE moz_places SET
        visit_count_local = visit_count_local - (OLD.visit_type NOT IN (0, 4, 7, 8, 9) AND OLD.is_local),
        visit_count_remote = visit_count_remote - (OLD.visit_type NOT IN (0, 4, 7, 8, 9) AND NOT(OLD.is_local)),
        last_visit_date_local = IFNULL((SELECT visit_date FROM moz_historyvisits
                                        WHERE place_id = OLD.place_id AND is_local
                                        ORDER BY visit_date DESC LIMIT 1), 0),
        last_visit_date_remote = IFNULL((SELECT visit_date FROM moz_historyvisits
                                         WHERE place_id = OLD.place_id AND NOT(is_local)
                                         ORDER BY visit_date DESC LIMIT 1), 0)
    WHERE id = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_bookmarks_afterinsert_trigger
AFTER INSERT ON moz_bookmarks FOR EACH ROW
BEGIN
    UPDATE moz_places
        SET foreign_count = foreign_count + 1
        WHERE id = NEW.fk;
    DELETE from moz_bookmarks_deleted WHERE guid = NEW.guid;
END;

CREATE TEMP TRIGGER moz_bookmarks_foreign_count_afterupdate_trigger
AFTER UPDATE OF fk, syncChangeCounter ON moz_bookmarks FOR EACH ROW
BEGIN
    UPDATE moz_places
        SET foreign_count = foreign_count + 1
        WHERE OLD.fk <> NEW.fk AND id = NEW.fk;
    UPDATE moz_places
        SET foreign_count = foreign_count - 1
        WHERE OLD.fk <> NEW.fk AND id = OLD.fk;
END;

CREATE TEMP TRIGGER moz_places_afterinsert_trigger_origins
AFTER INSERT ON moz_places FOR EACH ROW
BEGIN
    INSERT OR IGNORE INTO moz_updateoriginsinsert_temp (place_id, prefix, host, rev_host, frecency)
    VALUES (
        NEW.id,
        get_prefix(NEW.url),
        get_host_and_port(NEW.url),
        reverse_host(get_host_and_port(NEW.url)),
        NEW.frecency
    );
END;

CREATE TEMP TRIGGER moz_updateoriginsinsert_afterdelete_trigger
AFTER DELETE ON moz_updateoriginsinsert_temp FOR EACH ROW
BEGIN

    {decrease_frecency_stats};

    INSERT INTO moz_origins (prefix, host, rev_host, frecency)
    VALUES (
        OLD.prefix,
        OLD.host,
        OLD.rev_host,
        MAX(OLD.frecency, 0)
    )
    ON CONFLICT(host, prefix) DO UPDATE
        SET frecency = frecency + OLD.frecency
        WHERE OLD.frecency > 0;

    {increase_frecency_stats};

    UPDATE moz_places SET origin_id = (
        SELECT id FROM moz_origins
        WHERE prefix = OLD.prefix
          AND host = OLD.host
    )
    WHERE id = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_places_afterdelete_trigger_origins
AFTER DELETE ON moz_places
FOR EACH ROW
BEGIN
    INSERT INTO moz_updateoriginsdelete_temp (prefix, host, frecency_delta)
    VALUES (
        get_prefix(OLD.url),
        get_host_and_port(OLD.url),
        -MAX(OLD.frecency, 0)
    )
    ON CONFLICT(host, prefix) DO UPDATE
    SET frecency_delta = frecency_delta - OLD.frecency
    WHERE OLD.frecency > 0;
END;

CREATE TEMP TRIGGER moz_updateoriginsdelete_afterdelete_trigger
AFTER DELETE ON moz_updateoriginsdelete_temp FOR EACH ROW
BEGIN

    {decrease_frecency_stats};
    UPDATE moz_origins SET frecency = frecency + OLD.frecency_delta
    WHERE prefix = OLD.prefix AND host = OLD.host;

    DELETE FROM moz_origins
    WHERE prefix = OLD.prefix
        AND host = OLD.host
        AND NOT EXISTS (
            SELECT id FROM moz_places
            WHERE origin_id = moz_origins.id
            LIMIT 1
        );

    {increase_frecency_stats};
END;

CREATE TEMP TRIGGER moz_places_afterupdate_frecency_trigger
AFTER UPDATE OF frecency ON moz_places FOR EACH ROW
BEGIN
    INSERT INTO moz_updateoriginsupdate_temp (prefix, host, frecency_delta)
    VALUES (
        get_prefix(NEW.url),
        get_host_and_port(NEW.url),
        MAX(NEW.frecency, 0) - MAX(OLD.frecency, 0)
    )
    ON CONFLICT(host, prefix) DO UPDATE
    SET frecency_delta = frecency_delta + EXCLUDED.frecency_delta;
END;

CREATE TEMP TRIGGER moz_updateoriginsupdate_afterdelete_trigger
AFTER DELETE ON moz_updateoriginsupdate_temp FOR EACH ROW
BEGIN

    {decrease_frecency_stats};
    UPDATE moz_origins
    SET frecency = frecency + OLD.frecency_delta
    WHERE prefix = OLD.prefix
      AND host = OLD.host;

    {increase_frecency_stats};
END;

CREATE TEMP TRIGGER moz_bookmarks_synced_foreign_count_afterinsert_trigger
AFTER INSERT ON moz_bookmarks_synced
BEGIN
    UPDATE moz_places SET
        foreign_count = foreign_count + 1
    WHERE id = NEW.placeId;
END;

CREATE TEMP TRIGGER moz_bookmarks_synced_foreign_count_afterupdate_trigger
AFTER UPDATE OF placeId ON moz_bookmarks_synced
BEGIN
    UPDATE moz_places SET
        foreign_count = foreign_count + 1
    WHERE id = NEW.placeId;

    UPDATE moz_places SET
        foreign_count = foreign_count - 1
    WHERE id = OLD.placeId;
END;

CREATE TEMP TRIGGER moz_bookmarks_synced_foreign_count_afterdelete_trigger
AFTER DELETE ON moz_bookmarks_synced
BEGIN
    UPDATE moz_places SET
        foreign_count = foreign_count - 1
    WHERE id = OLD.placeId;
END;

CREATE TEMP TRIGGER moz_cleanup_origin_bookmark_deleted_trigger
AFTER DELETE ON moz_bookmarks
BEGIN

    UPDATE moz_places
        SET foreign_count = foreign_count - 1
        WHERE id = OLD.fk;

    DELETE FROM moz_places
        WHERE id = OLD.fk
        AND foreign_count = 0
        AND last_visit_date_local = 0
        AND last_visit_date_remote = 0;
END;

CREATE TEMP TRIGGER moz_bookmarks_parent_afterdelete_trigger
AFTER DELETE ON moz_bookmarks
BEGIN
    DELETE FROM moz_bookmarks
    WHERE parent = OLD.id;
END;

CREATE TEMP TRIGGER moz_tags_relations_afterinsert_trigger
AFTER INSERT ON moz_tags_relation
BEGIN
    UPDATE moz_tags SET
        lastModified = now()
    WHERE id = NEW.tag_id;

    UPDATE moz_places SET
        foreign_count = foreign_count + 1
    WHERE id = NEW.place_id;
END;

CREATE TEMP TRIGGER moz_tags_relations_afterupdate_trigger
AFTER UPDATE ON moz_tags_relation
BEGIN
    UPDATE moz_tags SET
        lastModified = now()
    WHERE id IN (OLD.tag_id, NEW.tag_id);

    UPDATE moz_places SET
        foreign_count = foreign_count + 1
    WHERE id = NEW.place_id;

    UPDATE moz_places SET
        foreign_count = foreign_count - 1
    WHERE id = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_tags_relations_afterdelete_trigger
AFTER DELETE ON moz_tags_relation
BEGIN
    UPDATE moz_tags SET
        lastModified = now()
    WHERE id = OLD.tag_id;

    UPDATE moz_places SET
        foreign_count = foreign_count - 1
    WHERE id = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_keywords_afterinsert_trigger
AFTER INSERT ON moz_keywords
BEGIN
    UPDATE moz_places SET
        foreign_count = foreign_count + 1
    WHERE id = NEW.place_id;
END;

CREATE TEMP TRIGGER moz_keywords_afterupdate_trigger
AFTER UPDATE OF place_id ON moz_keywords
WHEN OLD.place_id <> NEW.place_id
BEGIN
    UPDATE moz_places SET
        foreign_count = foreign_count + 1
    WHERE id = NEW.place_id;

    UPDATE moz_places SET
        foreign_count = foreign_count - 1
    WHERE id = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_keywords_afterdelete_trigger
AFTER DELETE ON moz_keywords
BEGIN
    UPDATE moz_places SET
        foreign_count = foreign_count - 1
    WHERE id = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_places_metadata_afterdelete_trigger_search_queries
AFTER DELETE ON moz_places_metadata
FOR EACH ROW
BEGIN
    DELETE FROM moz_places_metadata_search_queries WHERE id = OLD.search_query_id AND NOT EXISTS (
        SELECT id FROM moz_places_metadata pm WHERE pm.search_query_id = OLD.search_query_id
    );
END;
