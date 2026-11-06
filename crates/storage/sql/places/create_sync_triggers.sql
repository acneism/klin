CREATE TEMP TRIGGER pushUploadedChanges
AFTER UPDATE OF uploadedAt ON itemsToUpload WHEN NEW.uploadedAt > -1
BEGIN

    UPDATE moz_bookmarks SET
        syncChangeCounter = max(syncChangeCounter - NEW.syncChangeCounter, 0),
        syncStatus = 2
    WHERE guid = NEW.guid;

    DELETE FROM moz_bookmarks_deleted
    WHERE guid = NEW.guid;

    REPLACE INTO moz_bookmarks_synced(guid, parentGuid, serverModified, needsMerge,
                                      validity, isDeleted, kind, dateAdded, title,
                                      placeId, keyword)
    VALUES(NEW.guid, NEW.parentGuid, NEW.uploadedAt, 0,
           1,
           NEW.isDeleted, NEW.kind, NEW.dateAdded, NEW.title,
           NEW.placeId, NEW.keyword);

    INSERT INTO moz_bookmarks_synced_structure(guid, parentGuid, position)
    SELECT guid, NEW.guid, position
    FROM structureToUpload
    WHERE parentId = NEW.id;

    INSERT INTO moz_bookmarks_synced_tag_relation(itemId, tagId)
    SELECT v.id, (SELECT t.id FROM moz_tags t
                  WHERE t.tag = o.tag)
    FROM tagsToUpload o
    JOIN itemsToUpload u ON u.id = o.id
    JOIN moz_bookmarks_synced v ON v.guid = u.guid
    WHERE o.id = NEW.id;
END;

CREATE TEMP TRIGGER changeGuids
AFTER DELETE ON changeGuidOps
BEGIN
  UPDATE moz_bookmarks SET
    guid = OLD.mergedGuid,
    lastModified = OLD.lastModified,
    syncStatus = IFNULL(OLD.syncStatus, syncStatus)
  WHERE guid = OLD.localGuid;
END;

CREATE TEMP TRIGGER applyNewLocalStructure
AFTER DELETE ON applyNewLocalStructureOps
BEGIN
  UPDATE moz_bookmarks SET
    parent = (SELECT id FROM moz_bookmarks
              WHERE guid = OLD.mergedParentGuid),
    position = OLD.position
  WHERE guid = OLD.mergedGuid;
END;
