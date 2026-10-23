-- store: meta
CREATE TABLE IF NOT EXISTS "nodes" ("volume_id" TEXT NOT NULL, "path" TEXT NOT NULL, "parent" TEXT, "name" TEXT NOT NULL, "kind" TEXT NOT NULL, "size" BIGINT NOT NULL DEFAULT 0, "mode" BIGINT NOT NULL, "mtime" DOUBLE PRECISION NOT NULL, "ctime" DOUBLE PRECISION NOT NULL, "atime" DOUBLE PRECISION NOT NULL, "sha256" TEXT, PRIMARY KEY ("volume_id", "path"))

CREATE TABLE IF NOT EXISTS "blob_refs" ("volume_id" TEXT NOT NULL, "sha256" TEXT NOT NULL, "refcount" BIGINT NOT NULL, "size" BIGINT NOT NULL, PRIMARY KEY ("volume_id", "sha256"))

CREATE TABLE IF NOT EXISTS "trash_entries" ("volume_id" TEXT NOT NULL, "trash_path" TEXT NOT NULL, "original_path" TEXT NOT NULL, "size" BIGINT NOT NULL DEFAULT 0, "kind" TEXT NOT NULL, "deleted_at" TEXT NOT NULL, "deleted_by" TEXT, PRIMARY KEY ("volume_id", "trash_path"))

CREATE TABLE IF NOT EXISTS "volume_usage" ("volume_id" TEXT NOT NULL, "used_bytes" BIGINT NOT NULL DEFAULT 0, PRIMARY KEY ("volume_id"))

CREATE TABLE IF NOT EXISTS "export_links" ("token" TEXT NOT NULL, "volume_id" TEXT NOT NULL, "created_at" TEXT NOT NULL, "expires_at" TEXT NOT NULL, PRIMARY KEY ("token"))

CREATE TABLE IF NOT EXISTS "doc_artifact_current" ("volume_id" TEXT NOT NULL, "path" TEXT NOT NULL, "set_id" TEXT NOT NULL, "node_rev" TEXT NOT NULL, "updated_at" BIGINT NOT NULL, PRIMARY KEY ("volume_id", "path"))

CREATE TABLE IF NOT EXISTS "doc_artifact_sets" ("volume_id" TEXT NOT NULL, "set_id" TEXT NOT NULL, "path" TEXT NOT NULL, "node_rev" TEXT NOT NULL, "text_sha" TEXT, "text_len" BIGINT NOT NULL DEFAULT 0, "report" TEXT NOT NULL, "superseded_at" BIGINT, "created_at" BIGINT NOT NULL, PRIMARY KEY ("volume_id", "set_id"))

CREATE TABLE IF NOT EXISTS "doc_images" ("volume_id" TEXT NOT NULL, "set_id" TEXT NOT NULL, "seq" BIGINT NOT NULL, "caption" TEXT NOT NULL, "page" BIGINT, "format" TEXT NOT NULL, "sha256" TEXT NOT NULL, "size" BIGINT NOT NULL, "line_start" BIGINT NOT NULL, "line_count" BIGINT NOT NULL, "caption_line" BIGINT, PRIMARY KEY ("volume_id", "set_id", "seq"))

CREATE TABLE IF NOT EXISTS "doc_tables" ("volume_id" TEXT NOT NULL, "set_id" TEXT NOT NULL, "seq" BIGINT NOT NULL, "caption" TEXT NOT NULL, "row_count" BIGINT NOT NULL, "col_count" BIGINT NOT NULL, "quality" TEXT NOT NULL, "cells" TEXT NOT NULL, "line_start" BIGINT NOT NULL, "line_count" BIGINT NOT NULL, PRIMARY KEY ("volume_id", "set_id", "seq"))

CREATE INDEX IF NOT EXISTS "idx_nodes_parent" ON "nodes" ("volume_id", "parent")

CREATE INDEX IF NOT EXISTS "idx_trash_entries_deleted_at" ON "trash_entries" ("volume_id", "deleted_at")

CREATE INDEX IF NOT EXISTS "idx_doc_artifact_sets_path" ON "doc_artifact_sets" ("volume_id", "path")

-- store: admin
CREATE TABLE IF NOT EXISTS "project" ("id" TEXT NOT NULL, "owner" TEXT NOT NULL, "created_at" TEXT NOT NULL, PRIMARY KEY ("id"))

CREATE TABLE IF NOT EXISTS "project_member" ("project_id" TEXT NOT NULL, "person" TEXT NOT NULL, "role" TEXT NOT NULL, "added_by" TEXT NOT NULL, "added_at" TEXT NOT NULL, PRIMARY KEY ("project_id", "person"), FOREIGN KEY ("project_id") REFERENCES "project" ("id") ON DELETE CASCADE)

CREATE TABLE IF NOT EXISTS "project_purge_config" ("project_id" TEXT NOT NULL, "autopurge_enabled" BIGINT NOT NULL DEFAULT 0, "use_internal_purge" BIGINT NOT NULL DEFAULT 0, "file_retention_days" BIGINT, "project_retention_days" BIGINT, PRIMARY KEY ("project_id"), FOREIGN KEY ("project_id") REFERENCES "project" ("id") ON DELETE CASCADE)

-- store: git
CREATE TABLE IF NOT EXISTS "git_objects" ("volume_id" TEXT NOT NULL, "hash" TEXT NOT NULL, "type" TEXT NOT NULL, "size" BIGINT NOT NULL, PRIMARY KEY ("volume_id", "hash"))

CREATE TABLE IF NOT EXISTS "git_refs" ("volume_id" TEXT NOT NULL, "name" TEXT NOT NULL, "target" TEXT NOT NULL, "symbolic" BIGINT NOT NULL DEFAULT 0, PRIMARY KEY ("volume_id", "name"))

CREATE TABLE IF NOT EXISTS "git_operations" ("volume_id" TEXT NOT NULL, "op_type" TEXT NOT NULL, "state" TEXT NOT NULL, "source_ref" TEXT, "onto_sha" TEXT, "original_tip_sha" TEXT, "todo" TEXT, "current_step" BIGINT NOT NULL DEFAULT 0, "total_steps" BIGINT NOT NULL DEFAULT 0, "conflicts" TEXT, "resolutions" TEXT, "created_at" TEXT NOT NULL, "updated_at" TEXT NOT NULL, PRIMARY KEY ("volume_id"))

CREATE TABLE IF NOT EXISTS "git_remotes" ("volume_id" TEXT NOT NULL, "name" TEXT NOT NULL, "url" TEXT NOT NULL, PRIMARY KEY ("volume_id", "name"))

-- store: oauth
CREATE TABLE IF NOT EXISTS "oauth_tokens" ("person" TEXT NOT NULL, "host" TEXT NOT NULL, "provider" TEXT NOT NULL, "token_enc" BYTEA NOT NULL, "scopes" TEXT NOT NULL, "expires_at" TEXT, "instance_url" TEXT, PRIMARY KEY ("person", "host"))

