/// Reconstruct historical schemas from a fresh current synthetic database.
pub fn rewind(db: &rusqlite::Connection, version: u32) {
    if version < 72 {
        db.execute_batch("DROP TABLE IF EXISTS mobile_link;")
            .unwrap();
    }
    if version < 71 {
        db.execute_batch("DROP TABLE IF EXISTS mobile_wallpapers;")
            .unwrap();
    }
    if version < 70 {
        db.execute_batch("DROP TABLE IF EXISTS call_history;")
            .unwrap();
    }
    if version < 69 {
        db.execute_batch("DROP TABLE IF EXISTS mobile_uploads;")
            .unwrap();
    }
    if version < 67 {
        db.execute_batch("DROP TABLE call_cursor; DROP TABLE call_jobs; DROP TABLE calls;")
            .unwrap();
    }
    if version < 66 {
        db.execute_batch("DROP TABLE service_queries;").unwrap();
    }
    if version < 65 {
        db.execute_batch("DROP TABLE location_jobs;").unwrap();
    }
    if version < 64 {
        db.execute_batch("DROP TABLE structured_alarm_cursor; DROP TABLE structured_alarms;")
            .unwrap();
        db.execute_batch("DROP TABLE structured_closed_pages; DROP TABLE structured_closed_voters; DROP TABLE structured_closed_totals; DROP TABLE structured_close_drafts; DROP TABLE structured_close_parts; DROP TABLE structured_close_tree;").unwrap();
        db.execute_batch("DROP TABLE structured_dependencies; DROP TABLE structured_origins; DROP TABLE structured_drafts;
ALTER TABLE structured_sources RENAME TO structured_sources_new;
DROP INDEX structured_sources_card;
CREATE TABLE structured_sources(record BLOB PRIMARY KEY,card BLOB NOT NULL REFERENCES structured_cards(id),live INTEGER NOT NULL CHECK(live IN(0,1)),content BLOB NOT NULL);
INSERT INTO structured_sources SELECT * FROM structured_sources_new;
DROP TABLE structured_sources_new;
CREATE INDEX structured_sources_card ON structured_sources(card,live);").unwrap();
    }
    if version < 63 {
        db.execute_batch("DROP TABLE conversation_archive_refs;")
            .unwrap();
        db.execute_batch("DROP TABLE archive_lifecycle; DROP TABLE archive_protected; DROP TABLE archive_remote; DROP TABLE archive_garbage; DROP TABLE archive_local; DROP TABLE archive_media_garbage; DROP TABLE archive_media_removed; DROP TABLE conversation_removed; DROP TABLE conversation_cleanup;").unwrap();
    }
    if version < 62 {
        db.execute_batch(
            "DROP TABLE conversation_receipts; DROP TABLE conversation_receipt_cursor;",
        )
        .unwrap();
        db.execute_batch("DROP TABLE conversation_ops; DROP TABLE conversation_clock; DROP TABLE conversation_time; DROP TABLE conversation_operation_ids; DROP TABLE conversation_sync; DROP TABLE conversation_sync_deferred; DROP TABLE conversation_origins; DROP TABLE conversation_fragments; DROP TABLE conversation_cancelled;").unwrap();
    }
    if version < 61 {
        db.execute_batch("DROP TABLE group_shared_history; DROP TABLE group_history_work;")
            .unwrap();
    }
    if version < 60 {
        db.execute_batch("DROP TABLE group_key_recovery;").unwrap();
    }
    if version < 59 {
        db.execute_batch("DROP TABLE group_bootstrap;").unwrap();
    }
    if version < 58 {
        db.execute_batch("DROP TABLE group_invitation_packets; DROP TABLE group_invitations;")
            .unwrap();
    }
    if version < 57 {
        db.execute_batch(
            "DROP TABLE group_channels; DROP TABLE group_credentials; DROP TABLE group_work;",
        )
        .unwrap();
    }
    if version < 56 {
        db.execute_batch("DROP TABLE group_envelopes; DROP TABLE group_routes;")
            .unwrap();
    }
    if version < 55 {
        db.execute_batch("DROP TABLE group_service_outbox; DROP TABLE group_service;")
            .unwrap();
    }
    if version < 53 {
        db.execute_batch(
            "DROP TABLE structured_task_completions; DROP TABLE structured_task_counts;",
        )
        .unwrap();
    }
    if version < 52 {
        db.execute_batch("DROP TABLE structured_work; DROP TABLE structured_sources; DROP TABLE structured_totals; DROP TABLE structured_heads; DROP TABLE structured_actions; DROP TABLE structured_cards;").unwrap();
    }
    if version < 49 {
        db.execute_batch("DROP TABLE push_state; ALTER TABLE sync_schedule RENAME TO new_schedule; CREATE TABLE sync_schedule(id INTEGER PRIMARY KEY CHECK(id IN (1,2)),state BLOB NOT NULL); INSERT INTO sync_schedule SELECT id,state FROM new_schedule WHERE id IN (1,2); DROP TABLE new_schedule;").unwrap();
    }
    if version < 48 {
        db.execute_batch("DROP TABLE archive_media;").unwrap();
    }
    if version < 47 {
        db.execute_batch("DROP TABLE archive_work; ALTER TABLE sync_schedule RENAME TO new_schedule; CREATE TABLE sync_schedule(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); INSERT INTO sync_schedule SELECT id,state FROM new_schedule WHERE id=1; DROP TABLE new_schedule;").unwrap();
    }
    if version < 45 {
        db.execute_batch("DROP TABLE group_delivery_cursor; DROP TABLE group_incoming; DROP TABLE group_delivery; DROP TABLE group_messages;").unwrap();
    }
    if version < 44 {
        db.execute_batch("DROP TABLE group_controls; DROP TABLE group_key_outbox; DROP TABLE group_receivers; DROP TABLE group_senders;").unwrap();
    }
    if version < 43 {
        db.execute_batch("DROP TABLE group_forks; DROP TABLE group_proposals; DROP TABLE group_commits; DROP TABLE group_genesis; DROP TABLE groups;").unwrap();
    }
    if version < 42 {
        db.execute_batch("DROP INDEX prekeys_live; DROP INDEX claims_pending; DROP INDEX sessions_live; DROP INDEX incoming_pending; DROP INDEX retry_incoming_pending; DROP INDEX recovered_pending; DROP TABLE control_dependencies; DROP TABLE control_journals; DROP TABLE control_cleanup_cursor; DROP INDEX peers_current; ALTER TABLE peers DROP COLUMN obsolete;").unwrap();
    }
    if version < 41 {
        db.execute_batch("DROP TABLE retry_send_cursor; DROP INDEX retry_outbox_pending; ALTER TABLE retry_outbox DROP COLUMN complete;").unwrap();
    }
    if version < 40 {
        db.execute_batch("DROP TABLE send_intents; DROP TABLE send_intent_cursor;")
            .unwrap();
    }
    if version < 39 {
        db.execute_batch("DROP TABLE sync_schedule;").unwrap();
    }
    if version < 38 {
        db.execute_batch("DROP TABLE prekey_cursor; DROP INDEX prekey_pending_publications;")
            .unwrap();
    }
    if version < 37 {
        db.execute_batch("DROP TABLE outbound_cursor; DROP INDEX outbox_pending_sessions;")
            .unwrap();
    }
    if version < 36 {
        db.execute_batch("DROP TABLE session_activity; DROP TABLE session_maintenance;")
            .unwrap();
    }
    if version < 35 {
        db.execute_batch("DROP TABLE device_link_records;").unwrap();
    }
    if version < 34 {
        db.execute_batch("ALTER TABLE retry_gc_cursor RENAME TO retry_gc_cursor_new; CREATE TABLE retry_gc_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); INSERT INTO retry_gc_cursor SELECT id,state FROM retry_gc_cursor_new WHERE id=1; DROP TABLE retry_gc_cursor_new;").unwrap();
    }
    if version < 33 {
        db.execute_batch("DROP TABLE recovered_deliveries;")
            .unwrap();
    }
    if version < 32 {
        db.execute_batch("DROP TABLE retired_retry_requests;")
            .unwrap();
    }
    if version < 31 {
        db.execute_batch("DROP TABLE retired_retry_outbox; DROP INDEX inbox_message;")
            .unwrap();
    }
    if version < 30 {
        db.execute_batch("DROP TABLE retry_gc_cursor;").unwrap();
    }
    if version < 29 {
        db.execute_batch(
            "DROP INDEX retry_pending; ALTER TABLE retry_requests DROP COLUMN finished;",
        )
        .unwrap();
    }
    if version < 28 {
        db.execute_batch("DROP TABLE retry_cursor;").unwrap();
    }
    if version < 27 {
        db.execute_batch("DROP TABLE retry_incoming;").unwrap();
    }
    if version < 25 {
        db.execute_batch("DROP TABLE retry_outbox; DROP TABLE retry_requests;")
            .unwrap();
    }
    if version < 23 {
        db.execute_batch("DROP TABLE initial_headers;").unwrap();
    }
    if version < 22 {
        db.execute_batch("DROP TABLE active_sessions;").unwrap();
    }
    if version < 21 {
        db.execute_batch("ALTER TABLE deliveries DROP COLUMN expired;")
            .unwrap();
    }
    if version < 20 {
        db.execute_batch("ALTER TABLE sessions DROP COLUMN retired;")
            .unwrap();
    }
    if version < 19 {
        db.execute_batch("DROP TABLE archive_competition;").unwrap();
    }
    if version < 18 {
        db.execute_batch("DROP TABLE archive_checkpoints;").unwrap();
    }
    if version < 17 {
        db.execute_batch("DROP TABLE text_events;").unwrap();
    }
    if version < 16 {
        db.execute_batch("DROP TABLE incoming_cursor;").unwrap();
    }
    if version < 15 {
        db.execute_batch("DROP TABLE incoming; DROP INDEX sessions_peer;")
            .unwrap();
    }
    if version < 14 {
        db.execute_batch("ALTER TABLE sessions DROP COLUMN peer;")
            .unwrap();
    }
    if version < 13 {
        db.execute_batch("DROP TABLE peers; DROP TABLE own_device_binding;")
            .unwrap();
    }
    if version < 12 {
        db.execute_batch("DROP INDEX prekey_publications_retirement; ALTER TABLE prekey_publications DROP COLUMN retire_at; CREATE INDEX prekey_publications_expiry ON prekey_publications(expires_at);").unwrap();
    }
    if version < 11 {
        db.execute_batch("DROP TABLE prekey_claims;").unwrap();
    }
    if version < 10 {
        db.execute_batch("DROP TABLE prekey_publications;").unwrap();
    }
    if version < 9 {
        db.execute_batch("DROP TABLE connection_roots; DROP TABLE connection; ALTER TABLE archive_pages DROP COLUMN complete;")
            .unwrap();
    }
    if version < 8 {
        db.execute_batch("DROP TABLE archive_pages; DROP TABLE archive_import; DROP TABLE archive_objects; DROP TABLE archive_records; DROP TABLE archive;").unwrap();
    }
    if version < 7 {
        db.execute_batch("ALTER TABLE outbox DROP COLUMN content;")
            .unwrap();
    }
    if version < 6 {
        db.execute_batch("ALTER TABLE sessions DROP COLUMN suite;")
            .unwrap();
    }
    if version < 5 {
        db.execute_batch("ALTER TABLE deliveries DROP COLUMN receipt;")
            .unwrap();
    }
    if version < 4 {
        db.execute_batch("DROP TABLE deliveries;").unwrap();
    }
    if version < 3 {
        db.execute_batch("DROP TABLE initiations;").unwrap();
    }
    if version < 2 {
        db.execute_batch("DROP TABLE prekeys; DROP TABLE identity;")
            .unwrap();
    }
    db.pragma_update(None, "user_version", version).unwrap();
}
