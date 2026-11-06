CREATE TEMP TRIGGER moz_create_bookmarks_deleted_trigger
AFTER DELETE ON moz_bookmarks
FOR EACH ROW WHEN OLD.syncStatus = 2
BEGIN
    INSERT into moz_bookmarks_deleted VALUES (OLD.guid, now());
END;

CREATE TEMP TRIGGER moz_remove_bookmarks_deleted_update_trigger
AFTER UPDATE ON moz_bookmarks
FOR EACH ROW WHEN OLD.guid != NEW.guid
BEGIN
    SELECT RAISE(FAIL, 'guids are immutable');
END;

CREATE TEMP TRIGGER moz_tags_relations_afterinsert_sync_trigger
AFTER INSERT ON moz_tags_relation
BEGIN
    UPDATE moz_bookmarks SET
        syncChangeCounter = syncChangeCounter + 1
    WHERE fk = NEW.place_id;
END;

CREATE TEMP TRIGGER moz_tags_relations_afterupdate_sync_trigger
AFTER UPDATE ON moz_tags_relation
BEGIN
    UPDATE moz_bookmarks SET
        syncChangeCounter = syncChangeCounter + 1
    WHERE fk IN (OLD.place_id, NEW.place_id);
END;

CREATE TEMP TRIGGER moz_tags_relations_afterdelete_sync_trigger
AFTER DELETE ON moz_tags_relation
BEGIN
    UPDATE moz_bookmarks SET
        syncChangeCounter = syncChangeCounter + 1
    WHERE fk = OLD.place_id;
END;

CREATE TEMP TRIGGER moz_bookmarks_gscc_after_insert
AFTER INSERT ON moz_bookmarks FOR EACH ROW
BEGIN
    SELECT note_bookmarks_sync_change() WHERE NEW.syncChangeCounter > 0;
END;

CREATE TEMP TRIGGER moz_bookmarks_gscc_after_update
AFTER UPDATE OF fk, syncChangeCounter ON moz_bookmarks FOR EACH ROW
BEGIN
    SELECT note_bookmarks_sync_change()
    WHERE NEW.syncChangeCounter <> OLD.syncChangeCounter;
END;

CREATE TEMP TRIGGER moz_bookmarks_gscc_after_delete
AFTER INSERT ON moz_bookmarks_deleted FOR EACH ROW
BEGIN
    SELECT note_bookmarks_sync_change();
END;
